use super::FutureThreadError;
use futures::{channel::mpsc, prelude::*};
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

enum State {
    Running,
    CloseSending,
    Closing,
    Closed,
}

pub struct TransformFuture<'a, A, P, R>
where
    A: FnOnce(Option<P>) + Send + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    closure: A,
    closure_sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
    return_receiver: mpsc::Receiver<R>,
    state: State,
    _phantom: PhantomData<&'a P>,
}

impl<'a, A, P, R> TransformFuture<'a, A, P, R>
where
    A: FnOnce(Option<P>) + Send + Clone + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    pub fn new(
        closure_sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
        return_receiver: mpsc::Receiver<R>,
        closure: A,
    ) -> Self {
        TransformFuture {
            closure,
            closure_sender,
            return_receiver,
            state: State::Running,
            _phantom: PhantomData,
        }
    }
}

impl<'a, A, P, R> Sink<P> for TransformFuture<'a, A, P, R>
where
    A: FnOnce(Option<P>) + Send + Clone + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    type Error = FutureThreadError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.closure_sender)
            .poll_ready(cx)
            .map_err(Into::into)
    }

    fn start_send(mut self: Pin<&mut Self>, item: P) -> Result<(), Self::Error> {
        let closure = self.closure.clone();

        let c = || closure(Some(item));
        Pin::new(&mut self.closure_sender)
            .start_send(Box::new(c))
            .map_err(Into::into)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.closure_sender)
            .poll_flush(cx)
            .map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        loop {
            match self.state {
                State::Running => {
                    self.state = State::CloseSending;
                }
                State::CloseSending => {
                    let poll = Pin::new(&mut self.closure_sender).poll_ready(cx)?;
                    match poll {
                        Poll::Ready(()) => {
                            self.state = State::Closing;
                            let closure = self.closure.clone();

                            let c = || closure(None);
                            let res = Pin::new(&mut self.closure_sender)
                                .start_send(Box::new(c))
                                .map_err(Into::into);
                            if res.is_err() {
                                break Poll::Ready(res);
                            }
                        }
                        Poll::Pending => break Poll::Pending,
                    }
                }
                State::Closing => {
                    let poll = Pin::new(&mut self.closure_sender).poll_close(cx)?;
                    match poll {
                        Poll::Ready(()) => {
                            self.state = State::Closed;
                            break Poll::Ready(Ok(()));
                        }
                        Poll::Pending => break Poll::Pending,
                    }
                }
                State::Closed => break Poll::Ready(Ok(())),
            }
        }
    }
}

impl<'a, A, P, R> Stream for TransformFuture<'a, A, P, R>
where
    A: FnOnce(Option<P>) + Send + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    type Item = R;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.return_receiver).poll_next(cx)
    }
}
