use core::fmt::{Debug, Formatter, Result as FmtResult};
use core::time::Duration;

use async_channel::Receiver;
use futures_channel::oneshot::Sender as OneshotSender;
use futures_util::StreamExt;

use crate::actor::{Actor, ActorState, Handler};
use crate::builder::WeakSender;
use crate::object::{FutureObjectContainer, FutureResultObjectContainer};
use crate::util::future_handle::{spawn_cancelable, FutureHandler};
use crate::util::runtime;

// ActorContext would hold actor instance local state and the actor itself.
// State shared by actors are stored in ActorState.
pub(crate) struct ActorContext<A>
where
    A: Actor + Handler + 'static,
{
    id: usize,
    generation: usize,
    tx: WeakSender<A>,
    rx: Receiver<ContextMessage<A>>,
    rx_sub: Receiver<ContextMessage<A>>,
    manual_shutdown: bool,
    actor: A,
    state: ActorState<A>,
}

impl<A> ActorContext<A>
where
    A: Actor + Handler,
{
    pub(crate) fn new(
        id: usize,
        tx: WeakSender<A>,
        rx: Receiver<ContextMessage<A>>,
        rx_sub: Receiver<ContextMessage<A>>,
        actor: A,
        state: ActorState<A>,
    ) -> Self {
        Self {
            id,
            generation: 0,
            tx,
            rx,
            rx_sub,
            manual_shutdown: false,
            actor,
            state,
        }
    }

    fn delayed_msg(&self, msg: ContextMessage<A>, dur: Duration) {
        if let Some(tx) = self.tx.upgrade() {
            let handle_delay_on_shutdown = self.state.handle_delay_on_shutdown();

            let handler = spawn_cancelable(
                Box::pin(runtime::delay_for(dur)),
                move |either| async move {
                    if let futures_util::future::Either::Left(_) = either {
                        if !handle_delay_on_shutdown {
                            return;
                        }
                    }
                    let _ = tx.send(msg).await;
                },
            );

            self.state.push_handler(vec![handler]);
        }
    }

    pub(crate) fn spawn_loop(mut self) {
        runtime::spawn(async {
            self.actor.on_start();
            self.state.inc_active();

            let mut select = futures_util::stream::select(self.rx.clone(), self.rx_sub.clone());

            while let Some(msg) = select.next().await {
                match msg {
                    ContextMessage::ManualShutDown(tx) => {
                        if tx.send(self.state()).is_ok() {
                            self.manual_shutdown = true;
                            break;
                        }
                    }
                    ContextMessage::Instant(tx, msg) => {
                        let res = self.actor.handle(msg).await;
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                    ContextMessage::InstantDynamic(tx, mut fut) => {
                        let res = fut.handle(&mut self.actor).await;
                        if let Some(tx) = tx {
                            let _ = tx.send(res);
                        }
                    }
                    ContextMessage::Delayed(msg, dur) => {
                        self.delayed_msg(ContextMessage::Instant(None, msg), dur)
                    }
                    ContextMessage::DelayedDynamic(fut, dur) => {
                        self.delayed_msg(ContextMessage::InstantDynamic(None, fut), dur)
                    }
                    ContextMessage::IntervalFutureRun(idx) => {
                        let mut guard = self.state.interval_futures.lock().await;
                        if let Some(fut) = guard.get_mut(&idx) {
                            let _ = fut.handle(&mut self.actor).await;
                        }
                    }
                    ContextMessage::IntervalFutureRemove(idx) => {
                        let _ = self.state.interval_futures.remove(idx).await;
                    }
                    ContextMessage::IntervalFutureRegister(tx, interval_future, dur) => {
                        // insert interval future to context and get it's index
                        let index = self.state.interval_futures.insert(interval_future).await;

                        // construct the interval future
                        let mut interval = runtime::interval(dur);
                        let ctx_tx = self.tx.clone();
                        let interval_loop = Box::pin(async move {
                            loop {
                                let _ = runtime::tick(&mut interval).await;
                                match ctx_tx.upgrade() {
                                    Some(tx) => {
                                        let _ =
                                            tx.send(ContextMessage::IntervalFutureRun(index)).await;
                                    }
                                    None => break,
                                }
                            }
                        });

                        // spawn a cancelable future and use the handler to execute the cancellation.
                        let mut interval_handler = spawn_cancelable(interval_loop, |_| async {});

                        // we attach the index of interval future and a tx of our channel to handler.
                        interval_handler.attach_tx(index, self.tx.clone());

                        self.state.push_handler(vec![interval_handler.clone()]);

                        let _ = tx.send(interval_handler);
                    }
                }
            }

            // dec_active will return false if the actors are already shutdown.
            if self.state.dec_active() && self.state.restart_on_err() && !self.manual_shutdown {
                self.generation += 1;
                return self.spawn_loop();
            };

            self.actor.on_stop();
        });
    }

    fn state(&self) -> ActorContextState {
        ActorContextState {
            id: self.id,
            generation: self.generation,
        }
    }
}

pub struct ActorContextState {
    id: usize,
    generation: usize,
}

impl Debug for ActorContextState {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("ActorContextState")
            .field("id", &self.id)
            .field("generation(Times restarted)", &self.generation)
            .finish()
    }
}

// The message type bridge the Address and Actor context.
pub(crate) enum ContextMessage<A>
where
    A: Actor,
{
    ManualShutDown(OneshotSender<ActorContextState>),
    Instant(Option<OneshotSender<A::Result>>, A::Message),
    InstantDynamic(
        Option<OneshotSender<FutureResultObjectContainer>>,
        FutureObjectContainer<A>,
    ),
    Delayed(A::Message, Duration),
    DelayedDynamic(FutureObjectContainer<A>, Duration),
    IntervalFutureRegister(
        OneshotSender<FutureHandler<A>>,
        FutureObjectContainer<A>,
        Duration,
    ),
    IntervalFutureRun(usize),
    IntervalFutureRemove(usize),
}
