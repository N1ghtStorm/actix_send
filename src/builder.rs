use std::sync::{Arc, Weak};

use async_channel::{unbounded, SendError};

use crate::actor::{Actor, ActorState, Handler};
use crate::address::Address;
use crate::context::{ActorContext, ContextMessage};

pub struct Builder<A>
where
    A: Actor,
{
    pub(crate) actor: A,
    pub config: Config,
}

#[derive(Clone)]
pub struct Config {
    pub num: usize,
    pub restart_on_err: bool,
    pub handle_delayed_on_shutdown: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num: 1,
            restart_on_err: false,
            handle_delayed_on_shutdown: false,
        }
    }
}

impl<A> Builder<A>
where
    A: Actor + Handler + Clone,
{
    pub fn new(actor: A) -> Self {
        Self {
            actor,
            config: Default::default(),
        }
    }

    /// Build multiple actors with the num passed.
    ///
    /// Every actor instance would be a `Clone` of the original one.
    ///
    /// All the actors would steal work from a single `async-channel`.
    pub fn num(mut self, num: usize) -> Self {
        Self::check_num(num, 0);
        self.config.num = num;
        self
    }

    /// Notify the actor(s) to handle all delayed messages/futures before it's shutdown.
    pub fn handle_delayed_on_shutdown(mut self) -> Self {
        self.config.handle_delayed_on_shutdown = true;
        self
    }

    /// Notify the actor(s) to restart if it exits on error.
    pub fn restart_on_err(mut self) -> Self {
        self.config.restart_on_err = true;
        self
    }

    /// Start actor(s) with the Builder settings.
    pub fn start(self) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender {
            inner: Arc::new(tx),
        };

        let state = ActorState::new(self.config);

        if num > 1 {
            for _i in 0..num {
                ActorContext::new(
                    tx.downgrade(),
                    rx.clone(),
                    self.actor.clone(),
                    state.clone(),
                )
                .spawn_loop();
            }
        } else {
            ActorContext::new(tx.downgrade(), rx, self.actor, state.clone()).spawn_loop();
        }

        Address::new(tx, state)
    }

    /// Start actors on the given arbiter slice.
    ///
    /// Actors would try to spawn evenly on the given arbiters.
    #[cfg(feature = "actix-runtime")]
    pub fn start_with_arbiter(self, arbiters: &[actix_rt::Arbiter]) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender {
            inner: Arc::new(tx),
        };

        let state = ActorState::new(self.config);

        if num > 1 {
            let len = arbiters.len();

            for i in 0..num {
                let index = i % len;

                let ctx = ActorContext::new(
                    tx.downgrade(),
                    rx.clone(),
                    self.actor.clone(),
                    state.clone(),
                );

                arbiters
                    .get(index)
                    .expect("Vec<Arbiters> index overflow")
                    .exec_fn(|| ctx.spawn_loop());
            }
        } else {
            let ctx = ActorContext::new(tx.downgrade(), rx, self.actor, state.clone());

            arbiters
                .first()
                .expect("Vec<Arbiters> index overflow.")
                .exec_fn(|| ctx.spawn_loop());
        }

        Address::new(tx, state)
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}

// A wrapper for async_channel::sender.
// ToDo: remove this when we have a weak sender.
pub struct Sender<A>
where
    A: Actor,
{
    inner: Arc<async_channel::Sender<ContextMessage<A>>>,
}

impl<A> Clone for Sender<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A> Sender<A>
where
    A: Actor,
{
    pub(crate) fn downgrade(&self) -> WeakSender<A> {
        WeakSender {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub(crate) async fn send(
        &self,
        msg: ContextMessage<A>,
    ) -> Result<(), SendError<ContextMessage<A>>> {
        self.inner.send(msg).await
    }
}

pub struct WeakSender<A>
where
    A: Actor,
{
    inner: Weak<async_channel::Sender<ContextMessage<A>>>,
}

impl<A> Clone for WeakSender<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A> WeakSender<A>
where
    A: Actor,
{
    pub(crate) fn upgrade(&self) -> Option<Sender<A>> {
        Weak::upgrade(&self.inner).map(|inner| Sender { inner })
    }
}
