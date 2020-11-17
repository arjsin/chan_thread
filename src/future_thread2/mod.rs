mod call_future;
mod transform_future;

pub use call_future::{Callable, CallFuture};
use futures::{
    channel::{mpsc, oneshot},
    executor::block_on,
    prelude::*,
};
use log::error;
use std::{sync::Arc, thread};
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

pub type FutureThreadResult<T> = Result<T, FutureThreadError>;

#[inline]
fn work(mut receiver: mpsc::Receiver<Box<dyn FnOnce() + Send>>) {
    while let Some(closure) = block_on(receiver.next()) {
        closure();
    }
}

#[derive(Clone)]
pub struct FutureThread {
    thread: Option<Arc<thread::JoinHandle<()>>>,
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

        // spawn thread with receiver
        let thread = thread::spawn(move || {
            work(receiver);
        });
        FutureThread {
            thread: Some(Arc::new(thread)),
            sender,
        }
    }

    pub async fn spawn<A, R>(&mut self, closure: A) -> FutureThreadResult<R>
    where
        A: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let c = || {
            let res = closure();
            if sender.send(res).is_err() {
                error!("BUG: Receiver dropped, error sending response from spawn of StreamThread");
            }
        };
        let sender = &mut self.sender;
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
    pub fn into_call_future<A, P, R>(
        self,
        mut closure: A,
    ) -> CallFuture<impl FnOnce(P, oneshot::Sender<R>) + Send + Clone, P, R>
    where
        A: FnMut(P) -> R + Send + Clone + 'static,
        P: Send,
        R: Send,
    {
        let c = move |param: P, sender: oneshot::Sender<R>| {
            let res = closure(param);
            if sender.send(res).is_err() {
                error!("BUG: Receiver dropped, error sending response from call of StreamThread");
            }
        };
        CallFuture::new(self, c)
    }

    pub fn into_transform<A, P, R>(
        self,
        closure: A,
    ) -> TransformFuture<impl FnOnce(Option<P>) + Send + Clone, P, R>
    where
        A: FnOnce(P) -> R + Send + Clone + Unpin + 'static,
        P: Send,
        R: Send,
    {
        let (mut return_sender, return_receiver) = mpsc::channel(1);
        let closure = move |param: Option<P>| {
            if let Some(param) = param {
                let res = closure(param);
                if block_on(return_sender.send(res)).is_err() {
                    error!("BUG: Receiver dropped, error sending response from transform of StreamThread");
                }
            } else {
                return_sender.close_channel();
            }
        };
        TransformFuture::new(self, return_receiver, closure)
    }
}

impl Drop for FutureThread {
    fn drop(&mut self) {
        let thread = self
            .thread
            .take()
            .expect("BUG: FutureThread's join handler already dropped");
        if let Ok(thread) = Arc::try_unwrap(thread) {
            self.sender.close_channel();
            thread.join().unwrap_or_else(|_| {
                error!("BUG: FutureThread's thread unable to join. Possible thread panic!")
            });
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
            let increase = fut_thread.clone().into_call_future(increase);
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
            let (tx, rx) = fut_thread.into_transform(|param| param * param).split();
            let fut1 = stream.map(Ok).forward(tx);
            let fut2 = rx.collect::<Vec<_>>();
            let (res1, res2) = future::join(fut1, fut2).await;
            assert_eq!(res1, Ok(()));
            assert_eq!(res2, vec![9, 25, 36, 49, 81]);
        };
        block_on(fut);
    }
}
