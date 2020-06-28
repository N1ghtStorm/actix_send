use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures::channel::oneshot::Sender as OneshotSender;

use crate::actors::{Actor, Handler, Message};
use crate::interval::IntervalFuture;
use crate::util::future_handle::{spawn_cancelable, FutureHandler};
use crate::util::runtime;

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

// the gut of the event loop of an actor.
pub(crate) fn spawn_loop<A>(
    actor: A,
    tx: Sender<ChannelMessage<A>>,
    rx: Receiver<ChannelMessage<A>>,
) -> FutureHandler
where
    A: Actor + Handler + 'static,
    A::Message: Message + Send + 'static,
    <A::Message as Message>::Result: Send,
{
    let mut ctx: ActorContext<A> = ActorContext::new(tx, actor);

    let fut = async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                ChannelMessage::Instant(tx, msg) => {
                    let res = ctx.actor.as_mut().unwrap().handle(msg).await;
                    if let Some(tx) = tx {
                        let _ = tx.send(res);
                    }
                }
                ChannelMessage::Delayed(msg, dur) => {
                    let tx = ctx.tx.clone();
                    let delayed_handler = spawn_cancelable(
                        Box::pin(async move {
                            runtime::delay_for(dur).await;
                            let _ = tx.send(ChannelMessage::Instant(None, msg)).await;
                        }),
                        true,
                        || {},
                    );
                    ctx.delayed_handlers.push(delayed_handler);
                }
                ChannelMessage::IntervalFuture(idx) => {
                    if let Some(fut) = ctx.interval_futures.get_mut(idx) {
                        let act = ctx.actor.take().unwrap_or_else(|| panic!("Actor is gone"));
                        let act = fut.handle(act).await;
                        ctx.actor = Some(act);
                    }
                }
                ChannelMessage::Interval(tx, interval_future, dur) => {
                    // push interval future to context and get it's index
                    // ToDo: For now we don't have a way to remove the interval future if it's canceled using handler.
                    ctx.interval_futures.push(interval_future);
                    let index = ctx.interval_futures.len() - 1;

                    // construct the interval future
                    let mut interval = runtime::interval(dur);
                    let ctx_tx = ctx.tx.clone();
                    let interval_loop = Box::pin(async move {
                        loop {
                            let _ = runtime::tick(&mut interval).await;
                            let _ = ctx_tx.send(ChannelMessage::IntervalFuture(index)).await;
                        }
                    });

                    // spawn a cancelable future and use the handler to execute the cancellation.
                    let handler = spawn_cancelable(interval_loop, true, || {});

                    let _ = tx.send(handler);
                }
            }
        }
    };

    // ToDo: We should define on_cancel here so channel would reject new messages and handle all remaining instant messages.
    spawn_cancelable(Box::pin(fut), true, || {})
}

pub(crate) enum ChannelMessage<A>
where
    A: Actor,
    A::Message: Message,
{
    Instant(
        Option<OneshotSender<<A::Message as Message>::Result>>,
        A::Message,
    ),
    Delayed(A::Message, Duration),
    Interval(OneshotSender<FutureHandler>, IntervalFuture<A>, Duration),
    IntervalFuture(usize),
}
