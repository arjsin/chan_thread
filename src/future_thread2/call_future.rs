use super::FutureThreadError;
use futures::{
    channel::{mpsc, oneshot},
    prelude::*,
};
use std::marker::PhantomData;

pub struct CallFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + 'static,
    P: Send + 'static,
    R: Send + 'static,
{
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
    closure: A,
    _phantom: PhantomData<(&'a P, R)>,
}

impl<'a, A, P, R> CallFuture<'a, A, P, R>
where
    A: FnOnce(P, oneshot::Sender<R>) + Send + Clone + 'static,
    P: Send + 'static,
    R: Send + 'static,
{

    pub(crate) fn new(sender: mpsc::Sender<Box<dyn FnOnce() + Send>>, closure: A) -> Self {
        let _phantom = PhantomData;
        CallFuture {sender, closure, _phantom}
    }

    pub async fn call(&mut self, parameter: P) -> Result<R, FutureThreadError> {
        let (remote_sender, receiver) = oneshot::channel();
        let closure = self.closure.clone();
        let c = || closure(parameter, remote_sender);
        match self.sender.send(Box::new(c)).await {
            Ok(()) => receiver
                .await
                .map_err(|_| FutureThreadError::InternallyCancelled), // TODO: Add strategy to recover thread panics
            Err(e) => Err(e.into()),
        }
    }
}
