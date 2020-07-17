use core::future::Future;
use core::time::Duration;

use async_channel::{bounded, unbounded};

use crate::actor::{Actor, ActorState, Handler};
use crate::address::Address;
use crate::context::{ActorContext, ContextMessage};
use crate::sender::Sender;

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
    pub allow_subscribe: bool,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            num: 1,
            restart_on_err: false,
            handle_delayed_on_shutdown: false,
            allow_subscribe: false,
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

    /// Allow other actors subscribe to our actors.
    ///
    /// Default is false
    pub fn allow_subscribe(mut self) -> Self {
        self.config.allow_subscribe = true;
        self
    }

    /// Start actor(s) with the Builder settings.
    pub async fn start(self) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender::from(tx);

        let state = ActorState::new(self.config);
        let mut subs = Vec::with_capacity(num);

        for i in 0..num {
            let actor = (self.actor_builder)().await;

            let (tx_sub, rx_sub) = bounded::<ContextMessage<A>>(num);

            subs.push(tx_sub);

            ActorContext::new(i, tx.downgrade(), rx.clone(), rx_sub, actor, state.clone())
                .spawn_loop();
        }

        Address::new(tx, subs.into(), state)
    }

    /// Start actors on the given arbiter slice.
    ///
    /// Actors would try to spawn evenly on the given arbiters.
    #[cfg(feature = "actix-runtime")]
    pub async fn start_with_arbiter(self, arbiters: &[actix_rt::Arbiter]) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded::<ContextMessage<A>>();
        let tx = Sender::from(tx);

        let state = ActorState::new(self.config);
        let mut subs = Vec::with_capacity(num);

        let len = arbiters.len();

        for i in 0..num {
            let index = i % len;

            let actor = (self.actor_builder)().await;

            let (tx_sub, rx_sub) = bounded::<ContextMessage<A>>(num);

            subs.push(tx_sub);

            let ctx =
                ActorContext::new(i, tx.downgrade(), rx.clone(), rx_sub, actor, state.clone());

            arbiters
                .get(index)
                .expect("Vec<Arbiters> index overflow")
                .exec_fn(|| ctx.spawn_loop());
        }

        Address::new(tx, subs.into(), state)
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}
