use crossbeam_channel as channel;
use futures::{channel::oneshot, prelude::*};
use log::error;
use std::{
    pin::Pin,
    task::{Context, Poll},
    thread,
};

#[inline]
fn work<F, S, R>(receiver: channel::Receiver<(S, oneshot::Sender<R>)>, mut closure: F)
where
    F: FnMut(S) -> R + Send + 'static,
{
    while let Ok((parameter, sender)) = receiver.recv() {
        let r = closure(parameter);
        if sender.send(r).is_err() {
            break;
        }
    }
}

/// Creates a thread to repeatedly spawn future on it
pub struct FutureThread<S: Send, R: Send>(Option<Inner<S, R>>);

struct Inner<S: Send, R: Send> {
    thread: thread::JoinHandle<()>,
    sender: channel::Sender<(S, oneshot::Sender<R>)>,
}

impl<S: Send + 'static, R: Send + 'static> FutureThread<S, R> {
    pub fn new<F>(closure: F) -> Self
    where
        F: FnMut(S) -> R + Send + 'static,
    {
        // channel of closure and 'sender of futures oneshot'
        let (sender, receiver) = channel::bounded::<(S, oneshot::Sender<R>)>(1);

        // spawn thread with crossbeam receiver
        let thread = thread::spawn(move || work(receiver, closure));
        FutureThread(Some(Inner { thread, sender }))
    }

    pub fn call(&self, parameter: S) -> FutureThreadFuture<R> {
        let (remote_sender, receiver) = oneshot::channel();
        let sender = &self.0.as_ref().unwrap().sender;
        FutureThreadFuture::new(sender, (parameter, remote_sender), receiver) // TODO: Add strategy to recover thread panics
    }
}

impl<S: Send, R: Send> Drop for FutureThread<S, R> {
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

pub enum FutureThreadFuture<T> {
    SendError,
    Receiving(oneshot::Receiver<T>),
}

impl<T> FutureThreadFuture<T> {
    fn new<S>(
        sender: &channel::Sender<(S, oneshot::Sender<T>)>,
        value: (S, oneshot::Sender<T>),
        receiver: oneshot::Receiver<T>,
    ) -> Self {
        match sender.send(value) {
            Ok(()) => FutureThreadFuture::Receiving(receiver),
            Err(_) => FutureThreadFuture::SendError,
        }
    }
}

impl<T> Future for FutureThreadFuture<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<Self::Output> {
        match *self {
            FutureThreadFuture::SendError => Poll::Ready(None),
            FutureThreadFuture::Receiving(ref mut r) => Pin::new(r).poll(context).map(Result::ok),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn smoke() {
        let fut_thread = FutureThread::new(|(a, b)| a + b);
        let out = block_on(fut_thread.call((1, 2)));
        assert_eq!(out, Some(3));
    }
}
