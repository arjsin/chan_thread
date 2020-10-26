mod call_future;
mod transform_future;

pub use call_future::CallFuture;
use futures::{
    channel::{mpsc, oneshot},
    executor::block_on,
    prelude::*,
};
use log::error;
use std::thread;
pub use transform_future::TransformFuture;

#[derive(Debug, Clone, PartialEq)]
pub enum FutureThreadError {
    Full,
    Disconnected,
    InternallyCancelled,
}

impl From<mpsc::SendError> for FutureThreadError {
    fn from(e: mpsc::SendError) -> Self {
        if e.is_full() {
            FutureThreadError::Full
        } else {
            FutureThreadError::Disconnected
        }
    }
}

#[inline]
fn work(mut receiver: mpsc::Receiver<Box<dyn FnOnce() + Send>>) {
    while let Some(closure) = block_on(receiver.next()) {
        closure();
    }
}

pub struct FutureThread(Option<Inner>);

struct Inner {
    thread: thread::JoinHandle<()>,
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

impl Default for FutureThread {
    fn default() -> Self {
        Self::new()
    }
}

impl FutureThread {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<Box<dyn FnOnce() + Send>>(1);

        // spawn thread with crossbeam receiver
        let thread = thread::spawn(move || {
            work(receiver);
        });
        FutureThread(Some(Inner { thread, sender }))
    }

    pub async fn spawn<A, R>(&mut self, closure: A) -> Result<R, FutureThreadError>
    where
        A: FnOnce() -> R + Send + 'static,
        R: Send + std::fmt::Debug + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let c = || {
            let res = closure();
            if sender.send(res).is_err() {
                error!("BUG: Receiver dropped, error sending response from spawn of StreamThread");
            }
        };
        let sender = &mut self.0.as_mut().unwrap().sender;
        let resp = match sender.send(Box::new(c)).await {
            Ok(()) => receiver
                .await
                .map_err(|_| FutureThreadError::InternallyCancelled), // TODO: Add strategy to recover thread panics
            Err(e) => Err(e.into()),
        };
        resp
    }
}

impl FutureThread {
    pub fn with_closure<A, P, R>(
        &mut self,
        mut closure: A,
    ) -> CallFuture<impl FnOnce(P, oneshot::Sender<R>) + Send + Clone, P, R>
    where
        A: FnMut(P) -> R + Send + Clone + 'static,
        P: Send,
        R: Send,
    {
        let sender = self.0.as_ref().unwrap().sender.clone();

        let c = move |param: P, sender: oneshot::Sender<R>| {
            let res = closure(param);
            if sender.send(res).is_err() {
                error!("BUG: Receiver dropped, error sending response from call of StreamThread");
            }
        };
        CallFuture::new(sender, c)
    }

    pub fn transform<A, P, R>(
        &self,
        closure: A,
    ) -> TransformFuture<impl FnOnce(P, oneshot::Sender<R>) + Send + Clone, P, R>
    where
        A: FnOnce(P) -> R + Send + Clone + Unpin + 'static,
        P: Send,
        R: Send,
    {
        let sender = self.0.as_ref().unwrap().sender.clone();
        let closure = move |param: P, sender: oneshot::Sender<R>| {
            let res = closure(param);
            if sender.send(res).is_err() {
                error!(
                    "BUG: Receiver dropped, error sending response from transform of StreamThread"
                );
            }
        };
        TransformFuture::new(closure, sender)
    }
}

impl Drop for FutureThread {
    fn drop(&mut self) {
        match self.0.take() {
            Some(Inner { thread, sender }) => {
                drop(sender);
                thread.join().unwrap_or_else(|_| {
                    error!("BUG: StreamThread's thread unable to join. Possible Thread panic!")
                });
            }
            _ => unreachable!("BUG: StreamThread dropping in unhandled state"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn smoke() {
        let fut = async {
            let mut fut_thread = FutureThread::new();
            let increase = |param| 1 + param;
            let mut increase = fut_thread.with_closure(increase);
            let val = increase.call(4u32).await;
            assert_eq!(val, Ok(5u32));
            let val = increase.call(8u32).await;
            assert_eq!(val, Ok(9u32));
            let val = fut_thread.spawn(|| 4 + 5).await;
            assert_eq!(val, Ok(9u32));
        };
        block_on(fut);
    }

    #[test]
    fn transform() {
        let input = [3u32, 5, 6, 7, 9];
        let stream = stream::unfold(0, |id| async move {
            if id < input.len() {
                Some((input[id], id + 1))
            } else {
                None
            }
        });
        let fut = async {
            let fut_thread = FutureThread::new();
            let (tx, rx) = fut_thread.transform(|param| param * param).split();
            stream.map(Ok).forward(tx).await.unwrap();
            let res = rx.collect::<Vec<_>>().await;
            assert_eq!(res, vec![9, 25, 36, 49, 81]);
        };
        block_on(fut);
    }
}
