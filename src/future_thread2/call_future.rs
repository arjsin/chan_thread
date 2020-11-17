use super::{FutureThread, FutureThreadError, FutureThreadResult};
use futures::{channel::oneshot, prelude::*};
use std::{marker::PhantomData, pin::Pin};

pub trait Callable<P, R> {
    fn call(&self, parameter: P) -> Pin<Box<dyn Future<Output = FutureThreadResult<R>> + Send>>;
}

pub struct CallFuture<A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    thread: FutureThread,
    closure: A,
    _phantom: PhantomData<(P, R)>,
}

impl<A, P, R> CallFuture<A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Clone + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    pub(crate) fn new(thread: FutureThread, closure: A) -> Self {
        let _phantom = PhantomData;
        CallFuture {
            thread,
            closure,
            _phantom,
        }
    }
}

impl<A, P, R> Callable<P, R> for CallFuture<A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Clone + 'static,
    P: Send,
    R: Send,
{
    fn call(&self, parameter: P) -> Pin<Box<dyn Future<Output = FutureThreadResult<R>> + Send>> {
        let (remote_sender, receiver) = oneshot::channel();
        let closure = self.closure.clone();
        let c = || closure(parameter, remote_sender);
        let mut sender = self.thread.sender.clone();
        let fut = async move {
            match sender.send(Box::new(c)).await {
                Ok(()) => receiver
                    .await
                    .map_err(|_| FutureThreadError::InternallyCancelled), // TODO: Add strategy to recover thread panics
                Err(e) => Err(e.into()),
            }
        };
        Box::pin(fut)
    }
}
