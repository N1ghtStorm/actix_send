use core::fmt::{Debug, Formatter, Result as FmtResult};
use core::time::Duration;

use futures_util::StreamExt;

use crate::actor::{Actor, ActorState, Handler};
use crate::object::{AnyObjectContainer, FutureObjectContainer};
use crate::receiver::Receiver;
use crate::sender::WeakSender;
use crate::util::{
    channel::OneShotSender,
    future_handle::{spawn_cancelable, FutureHandler},
    runtime,
};
use futures_util::stream::{select, Select};

// ActorContext would hold actor instance local state and the actor itself.
// State shared by actors are stored in ActorState.
pub(crate) struct ActorContext<A>
where
    A: Actor + Handler + 'static,
{
    id: usize,
    generation: usize,
    tx: WeakSender<ContextMessage<A>>,
    rx: Option<Recv<A>>,
    selector: Option<Select<Recv<A>, Recv<A>>>,
    manual_shutdown: bool,
    actor: A,
    state: ActorState<A>,
}

type Recv<A> = Receiver<ContextMessage<A>>;

impl<A> ActorContext<A>
where
    A: Actor + Handler,
{
    pub(crate) fn new(
        id: usize,
        tx: WeakSender<ContextMessage<A>>,
        rx: Receiver<ContextMessage<A>>,
        broadcast_receiver: Option<Receiver<ContextMessage<A>>>,
        actor: A,
        state: ActorState<A>,
    ) -> Self {
        // When rx_sub is Some we take both rx and rx_sub and construct selector field.
        let (rx, selector) = match broadcast_receiver {
            Some(broadcast_receiver) => (None, Some(select(rx, broadcast_receiver))),
            None => (Some(rx), None),
        };

        Self {
            id,
            generation: 0,
            tx,
            rx,
            selector,
            manual_shutdown: false,
            actor,
            state,
        }
    }

    // return true if we want to break the streaming loop
    async fn handle_msg(&mut self, msg: ContextMessage<A>) -> bool {
        match msg {
            ContextMessage::Instant(msg) => self.handle_instant_msg(msg).await,
            ContextMessage::Delayed(msg) => self.handle_delayed_msg(msg),
            ContextMessage::Interval(msg) => self.handle_interval_msg(msg).await,
            ContextMessage::ManualShutDown(tx) => {
                if tx.send(self.state()).is_ok() {
                    self.manual_shutdown = true;
                    return true;
                }
            }
        }

        false
    }

    async fn handle_instant_msg(&mut self, msg: InstantMessage<A>) {
        match msg {
            InstantMessage::Static(tx, msg) => {
                let res = self.actor.handle(msg).await;
                if let Some(tx) = tx {
                    let _ = tx.send(res);
                }
            }
            InstantMessage::Dynamic(tx, mut fut) => {
                let res = fut.handle(&mut self.actor).await;
                if let Some(tx) = tx {
                    let _ = tx.send(res);
                }
            }
        }
    }

    fn handle_delayed_msg(&self, msg: DelayedMessage<A>) {
        let (msg, dur) = match msg {
            DelayedMessage::Static(msg, dur) => (
                ContextMessage::Instant(InstantMessage::Static(None, msg)),
                dur,
            ),
            DelayedMessage::Dynamic(fut, dur) => (
                ContextMessage::Instant(InstantMessage::Dynamic(None, fut)),
                dur,
            ),
        };

        if let Some(tx) = self.tx.upgrade() {
            let handle_delay_on_shutdown = self.state.handle_delay_on_shutdown();

            let handler = spawn_cancelable(runtime::delay_for(dur), move |either| async move {
                if let futures_util::future::Either::Left(_) = either {
                    if !handle_delay_on_shutdown {
                        return;
                    }
                }
                let _ = tx.send(msg).await;
            });

            self.state.push_handler(vec![handler]);
        }
    }

    async fn handle_interval_msg(&mut self, msg: IntervalMessage<A>) {
        match msg {
            IntervalMessage::Run(idx) => {
                let mut guard = self.state.interval_futures.lock().await;
                if let Some(fut) = guard.get_mut(&idx) {
                    let _ = fut.handle(&mut self.actor).await;
                }
            }
            IntervalMessage::Remove(idx) => {
                let _ = self.state.interval_futures.remove(idx).await;
            }
            IntervalMessage::Register(tx, interval_future, dur) => {
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
                                let _ = tx
                                    .send(ContextMessage::Interval(IntervalMessage::Run(index)))
                                    .await;
                            }
                            None => break,
                        }
                        runtime::yield_now().await;
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

    pub(crate) fn spawn_loop(mut self) {
        runtime::spawn(async {
            self.actor.on_start().await;
            self.state.inc_active();

            match self.selector.is_some() {
                true => {
                    while let Some(msg) = self.selector.as_mut().unwrap().next().await {
                        let should_break = self.handle_msg(msg).await;

                        if should_break {
                            break;
                        }

                        runtime::yield_now().await;
                    }
                }
                false => {
                    while let Ok(msg) = self.rx.as_mut().unwrap().recv().await {
                        let should_break = self.handle_msg(msg).await;

                        if should_break {
                            break;
                        }

                        runtime::yield_now().await;
                    }
                }
            };

            // dec_active will return false if the actors are already shutdown.
            if self.state.dec_active() && self.state.restart_on_err() && !self.manual_shutdown {
                self.generation += 1;
                return self.spawn_loop();
            };

            self.actor.on_stop().await;
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
    ManualShutDown(OneShotSender<ActorContextState>),
    Instant(InstantMessage<A>),
    Delayed(DelayedMessage<A>),
    Interval(IntervalMessage<A>),
}

// variants of interval future request
pub(crate) enum IntervalMessage<A>
where
    A: Actor,
{
    Register(
        OneShotSender<FutureHandler<A>>,
        FutureObjectContainer<A>,
        Duration,
    ),
    Run(usize),
    Remove(usize),
}

// variants of instant request
pub(crate) enum InstantMessage<A>
where
    A: Actor,
{
    Static(Option<OneShotSender<A::Result>>, A::Message),
    Dynamic(
        Option<OneShotSender<AnyObjectContainer>>,
        FutureObjectContainer<A>,
    ),
}

// variants of delayed request
pub(crate) enum DelayedMessage<A>
where
    A: Actor,
{
    Static(A::Message, Duration),
    Dynamic(FutureObjectContainer<A>, Duration),
}
