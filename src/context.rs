use async_channel::Sender;

use crate::actors::{Actor, ChannelMessage, Message};
use crate::interval::IntervalFuture;
use crate::util::future_handle::FutureHandler;

pub(crate) struct ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    pub(crate) tx: Sender<ChannelMessage<A>>,
    pub(crate) actor: Option<A>,
    pub(crate) delayed_handlers: Vec<FutureHandler>,
    pub(crate) interval_futures: Vec<IntervalFuture<A>>,
}

impl<A> ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    pub(crate) fn new(tx: Sender<ChannelMessage<A>>, actor: A) -> Self {
        Self {
            tx,
            actor: Some(actor),
            delayed_handlers: Vec::new(),
            interval_futures: Vec::new(),
        }
    }
}

// We use the delayed handler to cancel all delayed messages that are not processed.
impl<A> Drop for ActorContext<A>
where
    A: Actor + Send,
    A::Message: Message + Send,
{
    fn drop(&mut self) {
        for handler in self.delayed_handlers.iter() {
            handler.cancel();
        }
    }
}
