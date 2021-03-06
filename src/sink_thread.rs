use futures::{
    channel::mpsc::{self, SendError},
    executor::block_on,
    prelude::*,
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    thread,
};

#[inline]
fn work<I, F, P, S>(mut receiver: mpsc::Receiver<S>, init: I, mut closure: F)
where
    I: FnOnce() -> P + Send + 'static,
    F: FnMut(&mut P, S) + Send + 'static,
{
    let mut p = init();
    while let Some(v) = block_on(receiver.next()) {
        closure(&mut p, v);
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
#[derive(Clone)]
pub struct SinkThread<S: Send> {
    thread: Option<Arc<thread::JoinHandle<()>>>,
    sender: mpsc::Sender<S>,
}

impl<S: Send + 'static> SinkThread<S> {
    pub fn new<I, F, P>(init: I, closure: F) -> Self
    where
        I: FnOnce() -> P + Send + 'static,
        F: FnMut(&mut P, S) + Send + 'static,
    {
        // channel of closure and 'sender of futures oneshot'
        let (sender, receiver) = mpsc::channel::<S>(1);

        // spawn thread with crossbeam receiver
        let thread = Some(Arc::new(thread::spawn(move || {
            work(receiver, init, closure)
        })));
        SinkThread { thread, sender }
    }
}

impl<S: Send> Sink<S> for SinkThread<S> {
    type Error = SinkThreadError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sender)
            .poll_ready(cx)
            .map_err(Into::into)
    }

    fn start_send(mut self: Pin<&mut Self>, item: S) -> Result<(), Self::Error> {
        Pin::new(&mut self.sender)
            .start_send(item)
            .map_err(Into::into)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sender)
            .poll_flush(cx)
            .map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sender)
            .poll_close(cx)
            .map_err(Into::into)
    }
}

impl<S: Send> Drop for SinkThread<S> {
    fn drop(&mut self) {
        self.thread = self.thread.take().and_then(|thread| {
            let thread = Arc::try_unwrap(thread);
            match thread {
                Ok(thread) => {
                    self.sender.disconnect();
                    thread
                        .join()
                        .expect("unable to join thread; expected thread panic");
                    None
                }
                Err(thread) => Some(thread),
            }
        });
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crossbeam_channel as channel;
    use futures::executor::block_on;

    #[test]
    fn smoke() {
        let (sender, receiver) = channel::bounded(1);
        let mut sink_thread = SinkThread::new(|| 1, move |&mut a, x| sender.send(a + x).unwrap());
        let mut sink_thread2 = sink_thread.clone();
        block_on(sink_thread2.send(1)).unwrap();
        block_on(sink_thread.send(2)).unwrap();
        assert_eq!(receiver.recv(), Ok(2));
        assert_eq!(receiver.recv(), Ok(3));
    }
}
