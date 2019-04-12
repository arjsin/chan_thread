use futures::prelude::*;
use futures::task::{Poll, Context};
use std::pin::Pin;

/// Combines two different types of futures but same output in either fashion without allocation
pub enum Either<A, B> {
    A(A),
    B(B),
}

impl<A, B> Future for Either<A, B>
where
    A: Future + Unpin,
    B: Future<Output = A::Output> + Unpin,
{
    type Output = A::Output;

    fn poll(self: Pin<&mut Self>, context: &mut Context) -> Poll<A::Output> {
        match *self.get_mut() {
            Either::A(ref mut a) => Pin::new(a).poll(context),
            Either::B(ref mut b) => Pin::new(b).poll(context),
        }
    }
}
