use crate::either::Either;
use crossbeam_channel as channel;
use futures::{channel::oneshot, prelude::*};
use std::thread;

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
pub struct FutureThread<S: Send, R: Send> {
    thread_state: Option<thread::JoinHandle<()>>,
    sender: Option<channel::Sender<(S, oneshot::Sender<R>)>>,
}

impl<S: Send + 'static, R: Send + 'static> FutureThread<S, R> {
    pub fn new<F>(closure: F) -> Self
    where
        F: FnMut(S) -> R + Send + 'static,
    {
        // channel of closure and 'sender of futures oneshot'
        let (sender, receiver) = channel::bounded::<(S, oneshot::Sender<R>)>(1);

        // spawn thread with crossbeam receiver
        let thread_state = Some(thread::spawn(move || work(receiver, closure)));
        FutureThread {
            thread_state,
            sender: Some(sender),
        }
    }

    pub fn call<'a>(&'a self, parameter: S) -> impl Future<Output = Option<R>> + 'a {
        let (sender, receiver) = oneshot::channel();
        match self.sender.as_ref().unwrap().send((parameter, sender)) {
            Ok(()) => Either::A(receiver.map(Result::ok)),
            Err(_) => Either::B(future::ready(None)), // TODO: Add strategy to recover thread panics
        }
    }
}

impl<S: Send, R: Send> Drop for FutureThread<S, R> {
    fn drop(&mut self) {
        drop(self.sender.take());
        self.thread_state.take().map(thread::JoinHandle::join);
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
