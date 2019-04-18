use futures::{
    channel::mpsc::{self, SendError},
    executor::block_on,
    prelude::*,
};
use log::error;
use std::{
    pin::Pin,
    task::{Context, Poll},
    thread,
};

#[inline]
fn work<F, S>(mut receiver: mpsc::Receiver<S>, mut closure: F)
where
    F: FnMut(S) + Send + 'static,
{
    while let Some(v) = block_on(receiver.next()) {
        closure(v);
    }
}

#[derive(Debug, Clone)]
pub enum SinkThreadError {
    Full,
    Disconnected,
}

impl From<SendError> for SinkThreadError {
    fn from(e: SendError) -> Self {
        if e.is_full() {
            SinkThreadError::Full
        } else {
            SinkThreadError::Disconnected
        }
    }
}

/// Creates a thread to repeatedly receive value
pub struct SinkThread<S: Send>(Option<Inner<S>>);

impl<S: Send> Unpin for SinkThread<S> {}

struct Inner<S: Send> {
    thread: thread::JoinHandle<()>,
    sender: mpsc::Sender<S>,
}

impl<S: Send + 'static> SinkThread<S> {
    pub fn new<F>(closure: F) -> Self
    where
        F: FnMut(S) + Send + 'static,
    {
        // channel of closure and 'sender of futures oneshot'
        let (sender, receiver) = mpsc::channel::<S>(1);

        // spawn thread with crossbeam receiver
        let thread = thread::spawn(move || work(receiver, closure));
        SinkThread(Some(Inner { thread, sender }))
    }
}

impl<S: Send + Unpin> Sink<S> for SinkThread<S> {
    type SinkError = SinkThreadError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::SinkError>> {
        match self.0 {
            Some(Inner { ref mut sender, .. }) => {
                Pin::new(sender).poll_ready(cx).map_err(Into::into)
            }
            None => Poll::Ready(Err(SinkThreadError::Disconnected)),
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::SinkError> {
        match self.0 {
            Some(Inner { ref mut sender, .. }) => {
                Pin::new(sender).start_send(item).map_err(Into::into)
            }
            None => Err(SinkThreadError::Disconnected),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::SinkError>> {
        match self.0 {
            Some(Inner { ref mut sender, .. }) => {
                Pin::new(sender).poll_flush(cx).map_err(Into::into)
            }
            None => Poll::Ready(Err(SinkThreadError::Disconnected)),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::SinkError>> {
        match self.0 {
            Some(Inner { ref mut sender, .. }) => {
                Pin::new(sender).poll_close(cx).map_err(Into::into)
            }
            None => Poll::Ready(Err(SinkThreadError::Disconnected)),
        }
    }
}

impl<S: Send> Drop for SinkThread<S> {
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
    use crossbeam_channel as channel;

    #[test]
    fn smoke() {
        let (sender, receiver) = channel::bounded(1);
        let mut sink_thread = SinkThread::new(move |x| sender.send(x).unwrap());
        block_on(sink_thread.send(1)).unwrap();
        block_on(sink_thread.send(2)).unwrap();
        assert_eq!(receiver.recv(), Ok(1));
        assert_eq!(receiver.recv(), Ok(2));
    }
}
