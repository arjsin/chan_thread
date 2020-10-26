use super::FutureThreadError;
use futures::{
    channel::{mpsc, oneshot},
    prelude::*,
    task::Waker,
};
use std::{
    collections::VecDeque,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

pub enum Receivers<R> {
    NotPolled,
    Receivers(VecDeque<Option<oneshot::Receiver<R>>>),
    Waker(Waker),
}

impl<R> Receivers<R> {
    fn add(&mut self, receiver: Option<oneshot::Receiver<R>>) {
        match self {
            // if not polled yet, set up receivers
            Receivers::NotPolled => {
                let mut r = VecDeque::new();
                r.push_front(receiver);
                *self = Receivers::Receivers(r);
            }
            // if receivers are already there, just push
            Receivers::Receivers(ref mut r) => {
                r.push_front(receiver);
            }
            // if waker, then setup receivers and notify waker
            Receivers::Waker(_) => {
                use std::mem;

                let mut r = VecDeque::new();
                r.push_front(receiver);
                let waker = mem::replace(self, Receivers::Receivers(r));
                if let Receivers::Waker(waker) = waker {
                    waker.wake();
                } else {
                    unreachable!("BUG: waker was already found, now changed to something else!");
                }
            }
        }
    }
}

pub struct TransformFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    closure: A,
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
    receivers: Receivers<R>,
    _phantom: PhantomData<&'a P>,
}

impl<'a, A, P, R> TransformFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Clone + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    pub fn new(closure: A, sender: mpsc::Sender<Box<dyn FnOnce() + Send>>) -> Self {
        TransformFuture {
            closure,
            sender,
            receivers: Receivers::NotPolled,
            _phantom: PhantomData,
        }
    }
}

impl<'a, A, P, R> Sink<P> for TransformFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Clone + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    type Error = FutureThreadError;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sender)
            .poll_ready(cx)
            .map_err(Into::into)
    }

    fn start_send(mut self: Pin<&mut Self>, item: P) -> Result<(), Self::Error> {
        let (remote_sender, receiver) = oneshot::channel();
        self.receivers.add(Some(receiver));
        let closure = self.closure.clone();
        let c = || closure(item, remote_sender);
        Pin::new(&mut self.sender)
            .start_send(Box::new(c))
            .map_err(Into::into)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sender)
            .poll_flush(cx)
            .map_err(Into::into)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.receivers.add(None);
        Pin::new(&mut self.sender)
            .poll_close(cx)
            .map_err(Into::into)
    }
}

impl<'a, A, P, R> Stream for TransformFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Unpin + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    type Item = R;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // We need to have receivers ready to make progress here

        let (poll, set_waker) = match self.receivers {
            // No receivers yet and we have not registered to wake up, let's register
            Receivers::NotPolled => (Poll::Pending, true),
            // Receivers are available, so poll back and remove if progress was made
            Receivers::Receivers(ref mut receivers) => {
                let (poll, set_waker) = match receivers.back_mut() {
                    Some(Some(receiver)) => {
                        let val = Pin::new(receiver).poll(cx).map(|x| x.ok());
                        match (receivers.len(), &val) {
                            (1, &Poll::Ready(_)) => (val, true), // only set_waker, pop is unnecessary
                            (_, &Poll::Ready(_)) => {
                                let _ = receivers.pop_back().expect("BUG: unable to pop");
                                (val, false) // pop when len > 1, pop is needed
                            }
                            (_, &Poll::Pending) => (Poll::Pending, false), // when pending, do nothing
                        }
                    }
                    Some(None) => (Poll::Ready(None), false),
                    None => unreachable!("BUG: improper state handling, NotPolled was expected"),
                };

                (poll, set_waker)
            }
            Receivers::Waker(_) => (Poll::Pending, false),
        };
        if set_waker {
            self.receivers = Receivers::Waker(cx.waker().clone())
        }
        poll
    }
}
