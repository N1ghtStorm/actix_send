use std::future::Future;
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_channel::{unbounded, SendError};

use crate::actor::{Actor, ActorState, Handler};
use crate::address::Address;
use crate::context::{ActorContext, ContextMessage};
use crate::error::ActixSendError;
use crate::util::runtime;

pub struct Builder<A, F>
where
    A: Actor,
    F: Future<Output = A>,
{
    pub(crate) actor_builder: Box<dyn Fn() -> F>,
    pub config: Config,
}

#[derive(Clone)]
pub struct Config {
    pub num: usize,
    pub restart_on_err: bool,
    pub handle_delayed_on_shutdown: bool,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num: 1,
            restart_on_err: false,
            handle_delayed_on_shutdown: false,
            timeout: Duration::from_secs(10),
        }
    }
}

impl<A, F> Builder<A, F>
where
    A: Actor + Handler + 'static,
    F: Future<Output = A>,
{
    /// Build multiple actors with the num passed.
    ///
    /// Every actor instance would be a `Clone` of the original one.
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

    /// Start actor(s) with the Builder settings.
    pub async fn start(self) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender {
            inner: Arc::new(tx),
        };

        let state = ActorState::new(self.config);

        for _i in 0..num {
            let actor = (self.actor_builder)().await;

            ActorContext::new(tx.downgrade(), rx.clone(), actor, state.clone()).spawn_loop();
        }

        Address::new(tx, state)
    }

    /// Start actors on the given arbiter slice.
    ///
    /// Actors would try to spawn evenly on the given arbiters.
    #[cfg(feature = "actix-runtime")]
    pub async fn start_with_arbiter(self, arbiters: &[actix_rt::Arbiter]) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender {
            inner: Arc::new(tx),
        };

        let state = ActorState::new(self.config);

        let len = arbiters.len();

        for i in 0..num {
            let index = i % len;

            let actor = (self.actor_builder)().await;

            let ctx = ActorContext::new(tx.downgrade(), rx.clone(), actor, state.clone());

            arbiters
                .get(index)
                .expect("Vec<Arbiters> index overflow")
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

    pub(crate) async fn send_timeout(
        &self,
        msg: ContextMessage<A>,
        dur: Duration,
    ) -> Result<(), ActixSendError> {
        let fut = self.inner.send(msg);
        runtime::timeout(dur, fut).await??;
        Ok(())
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
