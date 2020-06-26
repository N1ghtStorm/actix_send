use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use futures::channel::oneshot::Sender as OneshotSender;
use parking_lot::Mutex;
use tokio::sync::Mutex as AsyncMutex;

use crate::actors::{Actor, Message};
use crate::interval::IntervalFuture;
use crate::util::constant::{ACTIVE, SHUT};

pub(crate) struct ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    pub(crate) state: Arc<AtomicUsize>,
    pub(crate) interval_futures: Arc<AsyncMutex<Vec<IntervalFuture<A>>>>,
    #[allow(clippy::type_complexity)]
    pub(crate) messages: Arc<
        Mutex<
            VecDeque<(
                Option<OneshotSender<<A::Message as Message>::Result>>,
                A::Message,
            )>,
        >,
    >,
}

impl<A> ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(AtomicUsize::new(ACTIVE)),
            interval_futures: Arc::new(AsyncMutex::new(Vec::new())),
            messages: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl<A> Clone for ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            interval_futures: self.interval_futures.clone(),
            messages: self.messages.clone(),
        }
    }
}

impl<A> Drop for ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    fn drop(&mut self) {
        self.state.store(SHUT, Ordering::SeqCst);
    }
}
