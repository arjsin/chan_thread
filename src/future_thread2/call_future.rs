use super::{FutureThread, FutureThreadError};
use futures::{channel::oneshot, prelude::*};
use std::marker::PhantomData;

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

    pub async fn call(&mut self, parameter: P) -> Result<R, FutureThreadError> {
        let (remote_sender, receiver) = oneshot::channel();
        let closure = self.closure.clone();
        let c = || closure(parameter, remote_sender);
        match self.thread.sender.send(Box::new(c)).await {
            Ok(()) => receiver
                .await
                .map_err(|_| FutureThreadError::InternallyCancelled), // TODO: Add strategy to recover thread panics
            Err(e) => Err(e.into()),
        }
    }
}
