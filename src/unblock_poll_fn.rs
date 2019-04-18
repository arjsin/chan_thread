use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct UnblockPollFn<F> {
    f: F,
}

impl<F> Unpin for UnblockPollFn<F> {}

pub fn unblock_poll_fn<T, F>(f: F) -> UnblockPollFn<F>
where
    F: FnMut(&mut Context) -> Poll<T>,
{
    UnblockPollFn { f }
}

impl<T, F> Future for UnblockPollFn<F>
where
    F: FnMut(&mut Context) -> Poll<T>,
{
    type Output = Poll<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context) -> Poll<Poll<T>> {
        Poll::Ready((&mut self.f)(context))
    }
}
