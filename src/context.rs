use std::time::Duration;

use async_channel::Receiver;
use futures::channel::oneshot::Sender as OneshotSender;

use crate::actor::{Actor, ActorState, Handler};
use crate::builder::WeakSender;
use crate::object::{FutureObjectContainer, FutureResultObjectContainer};
use crate::util::future_handle::{spawn_cancelable, spawn_cancelable_new, FutureHandler};
use crate::util::runtime;

// ActorContext would hold actor instance local state.
// State shared by actors are stored in ActorState.
pub struct ActorContext<A>
where
    A: Actor + 'static,
{
    pub(crate) tx: WeakSender<A>,
    pub(crate) actor: A,
    pub(crate) state: ActorState<A>,
}

impl<A> ActorContext<A>
where
    A: Actor,
{
    pub(crate) fn new(tx: WeakSender<A>, actor: A, state: ActorState<A>) -> Self {
        Self { tx, actor, state }
    }

    fn delayed_msg(&self, msg: ChannelMessage<A>, dur: Duration) {
        if let Some(tx) = self.tx.upgrade() {
            let (handler, future) = spawn_cancelable_new(Box::pin(runtime::delay_for(dur)));

            let handle_delay_on_shutdown = self.state.handle_delay_on_shutdown();
            runtime::spawn(async move {
                let either = future.await;
                if let futures::future::Either::Left(_) = either {
                    if !handle_delay_on_shutdown {
                        return;
                    }
                }
                let _ = tx.send(msg).await;
            });

            self.state.push_handler(vec![handler]);
        }
    }
}

// the gut of the event loop of an actor.
pub(crate) fn spawn_loop<A>(
    actor: A,
    tx: WeakSender<A>,
    rx: Receiver<ChannelMessage<A>>,
    state: ActorState<A>,
) where
    A: Actor + Handler + 'static,
{
    let mut ctx: ActorContext<A> = ActorContext::new(tx, actor, state);

    let fut = async move {
        ctx.actor.on_start();

        while let Ok(msg) = rx.recv().await {
            match msg {
                ChannelMessage::Instant(tx, msg) => {
                    let res = ctx.actor.handle(msg).await;
                    if let Some(tx) = tx {
                        let _ = tx.send(res);
                    }
                }
                ChannelMessage::InstantDynamic(tx, mut fut) => {
                    let res = fut.handle(&mut ctx.actor).await;
                    if let Some(tx) = tx {
                        let _ = tx.send(res);
                    }
                }
                ChannelMessage::Delayed(msg, dur) => {
                    ctx.delayed_msg(ChannelMessage::Instant(None, msg), dur)
                }
                ChannelMessage::DelayedDynamic(fut, dur) => {
                    ctx.delayed_msg(ChannelMessage::InstantDynamic(None, fut), dur)
                }
                ChannelMessage::IntervalFuture(idx) => {
                    let mut guard = ctx.state.interval_futures.lock().await;
                    if let Some(fut) = guard.get_mut(&idx) {
                        let _ = fut.handle(&mut ctx.actor).await;
                    }
                }
                ChannelMessage::IntervalFutureRemove(idx) => {
                    let _ = ctx.state.interval_futures.remove(idx).await;
                }
                ChannelMessage::Interval(tx, interval_future, dur) => {
                    // insert interval future to context and get it's index
                    let index = ctx.state.interval_futures.insert(interval_future).await;

                    // construct the interval future
                    let mut interval = runtime::interval(dur);
                    let ctx_tx = ctx.tx.clone();
                    let interval_loop = Box::pin(async move {
                        loop {
                            let _ = runtime::tick(&mut interval).await;
                            match ctx_tx.upgrade() {
                                Some(tx) => {
                                    let _ = tx.send(ChannelMessage::IntervalFuture(index)).await;
                                }
                                None => break,
                            }
                        }
                    });

                    // spawn a cancelable future and use the handler to execute the cancellation.
                    let mut interval_handler = spawn_cancelable(interval_loop, true, || {});

                    // we attach the index of interval future and a tx of our channel to handler.
                    interval_handler.attach_tx(index, ctx.tx.clone());

                    ctx.state.push_handler(vec![interval_handler.clone()]);

                    let _ = tx.send(interval_handler);
                }
            }
        }
        <A as Actor>::on_stop(ctx.actor);

        // dec_active will return false if the actors are already shutdown.
        // if state.dec_active() && state.restart_on_err() {};
    };

    runtime::spawn(fut);
}

pub(crate) enum ChannelMessage<A>
where
    A: Actor,
{
    Instant(Option<OneshotSender<A::Result>>, A::Message),
    InstantDynamic(
        Option<OneshotSender<FutureResultObjectContainer>>,
        FutureObjectContainer<A>,
    ),
    Delayed(A::Message, Duration),
    DelayedDynamic(FutureObjectContainer<A>, Duration),
    Interval(
        OneshotSender<FutureHandler<A>>,
        FutureObjectContainer<A>,
        Duration,
    ),
    IntervalFuture(usize),
    IntervalFutureRemove(usize),
}
