use crate::unblock_poll_fn;
use futures::{channel::mpsc, executor::block_on, prelude::*};
use log::error;
use std::{
    pin::Pin,
    task::{Context, Poll},
    thread,
};

#[inline]
fn work<F1, F2, S, R>(
    mut sender: mpsc::Sender<R>,
    mut initial: S,
    mut busy_task: F1,
    mut idle_task: F2,
) where
    F1: FnMut(&mut S) -> Option<R> + Send + 'static,
    F2: FnMut(&mut S) -> Option<()> + Send + 'static,
    R: Send + 'static,
    S: Send + 'static,
{
    loop {
        // check if sender is ready
        let v = match block_on(unblock_poll_fn(|w| sender.poll_ready(w))) {
            Poll::Ready(Ok(())) => {
                busy_task(&mut initial).and_then(|v| block_on(sender.send(v)).ok())
            }
            Poll::Ready(Err(_)) => break,
            Poll::Pending => idle_task(&mut initial),
        };
        // break in case of send error or None from any closure
        if v.is_none() {
            break;
        }
    }
}

pub struct StreamThread<T>(Option<Inner<T>>);

struct Inner<T> {
    thread: thread::JoinHandle<()>,
    receiver: mpsc::Receiver<T>,
}

impl<R> StreamThread<R>
where
    R: Send + 'static,
{
    pub fn new<F1, F2, S>(initial: S, busy_task: F1, idle_task: F2) -> Self
    where
        F1: FnMut(&mut S) -> Option<R> + Send + 'static,
        F2: FnMut(&mut S) -> Option<()> + Send + 'static,
        S: Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(1);
        let thread = thread::spawn(move || work(sender, initial, busy_task, idle_task));
        StreamThread(Some(Inner { thread, receiver }))
    }
}

impl<T> Stream for StreamThread<T> {
    type Item = T;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<Option<Self::Item>> {
        match self.0 {
            Some(Inner {
                ref mut receiver, ..
            }) => Pin::new(receiver).poll_next(context),
            None => Poll::Ready(None), // assume this stream closing
        }
    }
}

impl<T> Drop for StreamThread<T> {
    fn drop(&mut self) {
        match self.0.take() {
            Some(Inner { thread, receiver }) => {
                drop(receiver);
                thread.join().unwrap_or_else(|_| {
                    error!("BUG: StreamThread's thread unable to join. Possible Thread panic!");
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
        let mut stream_thread = StreamThread::new(
            0,
            |x| {
                *x += 1;
                Some(*x)
            },
            |_| Some(()),
        );
        assert_eq!(block_on(stream_thread.next()), Some(1));
        assert_eq!(block_on(stream_thread.next()), Some(2));
    }
}
