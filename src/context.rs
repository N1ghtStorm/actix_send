use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures::channel::oneshot::Sender as OneshotSender;

use crate::actors::{Actor, Handler};
use crate::interval::{IntervalFuture, IntervalFutureSet};
use crate::util::future_handle::{spawn_cancelable, FutureHandler};
use crate::util::runtime;

pub(crate) struct ActorContext<A>
where
    A: Actor + Send + 'static,
    A::Message: Send,
    A::Result: Send,
{
    pub(crate) tx: Sender<ChannelMessage<A>>,
    pub(crate) actor: Option<A>,
    pub(crate) delayed_handlers: Vec<FutureHandler<A>>,
    pub(crate) interval_futures: IntervalFutureSet<A>,
}

impl<A> ActorContext<A>
where
    A: Actor + Send,
    A::Message: Send,
    A::Result: Send,
{
    pub(crate) fn new(
        tx: Sender<ChannelMessage<A>>,
        actor: A,
        interval_futures: IntervalFutureSet<A>,
    ) -> Self {
        Self {
            tx,
            actor: Some(actor),
            delayed_handlers: Vec::new(),
            interval_futures,
        }
    }
}

// We use the delayed handler to cancel all delayed messages that are not processed.
impl<A> Drop for ActorContext<A>
where
    A: Actor + Send + 'static,
    A::Message: Send,
    A::Result: Send,
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
    interval_futures: IntervalFutureSet<A>,
) -> FutureHandler<A>
where
    A: Actor + Handler + 'static,
    A::Message: Send + 'static,
    A::Result: Send,
{
    let mut ctx: ActorContext<A> = ActorContext::new(tx, actor, interval_futures);

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
                    let mut guard = ctx.interval_futures.lock().await;
                    if let Some(fut) = guard.get_mut(&idx) {
                        let act = ctx.actor.take().unwrap_or_else(|| panic!("Actor is gone"));
                        let act = fut.handle(act).await;
                        ctx.actor = Some(act);
                    }
                }
                ChannelMessage::IntervalFutureRemove(idx) => {
                    let _ = ctx.interval_futures.remove(idx).await;
                }
                ChannelMessage::Interval(tx, interval_future, dur) => {
                    // insert interval future to context and get it's index
                    let index = ctx.interval_futures.insert(interval_future).await;

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
                    let mut handler = spawn_cancelable(interval_loop, true, || {});

                    // we attach the index of interval future and a tx of our channel to handler.
                    handler.attach_tx(index, ctx.tx.clone());

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
    A::Message: Send,
    A::Result: Send,
{
    Instant(Option<OneshotSender<A::Result>>, A::Message),
    Delayed(A::Message, Duration),
    Interval(OneshotSender<FutureHandler<A>>, IntervalFuture<A>, Duration),
    IntervalFuture(usize),
    IntervalFutureRemove(usize),
}