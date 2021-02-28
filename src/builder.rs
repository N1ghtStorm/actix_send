use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use crate::actor::{Actor, ActorState, Handler};
use crate::address::Address;
use crate::context::{ActorContext, ContextMessage};
use crate::receiver::Receiver;
use crate::sender::Sender;
use crate::util::channel::unbounded;
use std::sync::Arc;

pub struct Builder<A>
where
    A: Actor,
{
    pub actor_builder: BuilderFnContainer<A>,
    pub config: Config,
}

// A container for builder function of actor instance.
// We box the function into a trait object to make it easier to work with for less type signatures.
pub struct BuilderFnContainer<A> {
    inner: Arc<dyn BuilderFnTrait<A> + Send + Sync>,
}

impl<A> Clone for BuilderFnContainer<A> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A> BuilderFnContainer<A> {
    pub(crate) fn new<F, Fut>(f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = A> + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    async fn build(&self) -> A {
        Arc::clone(&self.inner).build().await
    }
}

// A trait would call build method on our actor builder function
pub trait BuilderFnTrait<A> {
    fn build(&self) -> Pin<Box<dyn Future<Output = A> + '_>>;
}

impl<A, F, Fut> BuilderFnTrait<A> for F
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = A>,
{
    fn build(&self) -> Pin<Box<dyn Future<Output = A> + '_>> {
        Box::pin(async move { self().await })
    }
}

#[derive(Clone)]
pub struct Config {
    pub num: usize,
    pub restart_on_err: bool,
    pub handle_delayed_on_shutdown: bool,
    pub allow_broadcast: bool,
    pub allow_subscribe: bool,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num: 1,
            restart_on_err: false,
            handle_delayed_on_shutdown: false,
            allow_broadcast: false,
            allow_subscribe: false,
            timeout: Duration::from_secs(10),
        }
    }
}

impl<A> Builder<A>
where
    A: Actor + Handler + 'static,
{
    /// Build multiple actors with the num passed.
    ///
    /// All the actors would steal work from a single `async-channel`.
    ///
    /// Default is 1
    pub fn num(mut self, num: usize) -> Self {
        Self::check_num(num, 0);
        self.config.num = num;
        self
    }

    /// Notify the actor(s) to handle all delayed messages/futures before it's shutdown.
    ///
    /// Default is false.
    pub fn handle_delayed_on_shutdown(mut self) -> Self {
        self.config.handle_delayed_on_shutdown = true;
        self
    }

    /// Notify the actor(s) to restart if it exits on error.
    ///
    /// Default is false
    pub fn restart_on_err(mut self) -> Self {
        self.config.restart_on_err = true;
        self
    }

    /// Set the timeout of sending a message.
    ///
    /// Default is 10 seconds
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.config.timeout = duration;
        self
    }

    /// Allow broadcasting a message to all actor instance of one address.
    ///
    /// Default is false
    pub fn allow_broadcast(mut self) -> Self {
        self.config.allow_broadcast = true;
        self
    }

    /// Allow other actors bounded to a different address subscribe to our address.
    ///
    /// Default is false
    pub fn allow_subscribe(mut self) -> Self {
        self.config.allow_subscribe = true;
        self
    }

    /// Start actor(s) with the Builder settings.
    pub async fn start(self) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded_channel::<ContextMessage<A>>();

        let state = ActorState::new(self.config);
        let mut broadcast_senders = Vec::with_capacity(num);

        match num {
            1 => {
                let actor = self.actor_builder.build().await;

                let broadcast_receiver = state.broadcast_receiver(&mut broadcast_senders, num);
                ActorContext::new(
                    0,
                    tx.downgrade(),
                    rx,
                    broadcast_receiver,
                    actor,
                    state.clone(),
                )
                .spawn_loop();

                Address::new(tx, broadcast_senders.into(), state)
            }
            _ => {
                for i in 0..num {
                    let actor = self.actor_builder.build().await;

                    let broadcast_receiver = state.broadcast_receiver(&mut broadcast_senders, num);

                    ActorContext::new(
                        i,
                        tx.downgrade(),
                        rx.clone(),
                        broadcast_receiver,
                        actor,
                        state.clone(),
                    )
                    .spawn_loop();
                }

                Address::new(tx, broadcast_senders.into(), state)
            }
        }
    }

    /// Start actors on the given arbiter slice.
    ///
    /// Actors would try to spawn evenly on the given arbiters.
    #[cfg(any(feature = "actix-runtime", feature = "actix-runtime-mpsc"))]
    pub async fn start_with_arbiters(
        self,
        arbiters: &[actix_rt::Arbiter],
        index: Option<usize>,
    ) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded_channel::<ContextMessage<A>>();

        let state = ActorState::new(self.config);
        let mut broadcast_senders = Vec::with_capacity(num);

        match num {
            1 => {
                let builder = self.actor_builder.clone();

                let index = index.unwrap_or(0);

                arbiters
                    .get(index)
                    .expect("Vec<Arbiters> index overflow")
                    .spawn_fn({
                        let tx = tx.downgrade();
                        let state = state.clone();
                        move || {
                            actix_rt::spawn(async move {
                                let actor = builder.build().await;
                                let ctx = ActorContext::new(0, tx, rx.into(), None, actor, state);
                                ctx.spawn_loop();
                            });
                        }
                    });

                Address::new(tx, broadcast_senders.into(), state)
            }
            _ => {
                let len = arbiters.len();
                for i in 0..num {
                    let index = i % len;

                    let builder = self.actor_builder.clone();

                    let broadcast_receiver = state.broadcast_receiver(&mut broadcast_senders, num);

                    arbiters
                        .get(index)
                        .expect("Vec<Arbiters> index overflow")
                        .spawn_fn({
                            let tx = tx.downgrade();
                            let rx = rx.clone();
                            let state = state.clone();
                            move || {
                                actix_rt::spawn(async move {
                                    let actor = builder.build().await;

                                    ActorContext::new(
                                        i,
                                        tx,
                                        rx.into(),
                                        broadcast_receiver,
                                        actor,
                                        state,
                                    )
                                    .spawn_loop();
                                });
                            }
                        });
                }

                Address::new(tx, broadcast_senders.into(), state)
            }
        }
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}

fn unbounded_channel<A>() -> (Sender<A>, Receiver<A>) {
    let (tx, rx) = unbounded::<A>();

    (tx.into(), rx.into())
}
