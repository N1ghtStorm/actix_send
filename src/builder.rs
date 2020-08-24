use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use crate::actor::{Actor, ActorState, Handler};
use crate::address::Address;
use crate::context::{ActorContext, ContextMessage};
use crate::receiver::Receiver;
use crate::sender::Sender;
use crate::util::channel::{bounded, unbounded};

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
    inner: Box<dyn BuilderFnTrait<A>>,
}

impl<A> BuilderFnContainer<A> {
    async fn build(&self) -> A {
        self.inner.as_ref().build().await
    }
}

macro_rules! builder_impl {
    ($($send:ident)*) => {
            impl<A> BuilderFnContainer<A> {
            pub(crate) fn new<F, Fut>(f: F) -> Self
            where
                F: Fn() -> Fut $( + $send)* + 'static,
                Fut: Future<Output = A> $( + $send)* + 'static,
            {
                Self { inner: Box::new(f) }
            }
        }
    }
}

#[cfg(not(feature = "actix-runtime-local"))]
builder_impl!(Send);

#[cfg(feature = "actix-runtime-local")]
builder_impl!();

// A trait would call build method on our actor builder function
pub trait BuilderFnTrait<A> {
    fn build(&self) -> Pin<Box<dyn Future<Output = A> + '_>>;
}

impl<A, F, Fut> BuilderFnTrait<A> for F
where
    F: Fn() -> Fut,
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
    #[cfg(not(feature = "actix-runtime-local"))]
    pub async fn start(self) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded_channel::<ContextMessage<A>>();

        let state = ActorState::new(self.config);
        let mut subs = Vec::with_capacity(num);

        for i in 0..num {
            let actor = self.actor_builder.build().await;

            let rx_sub = match state.allow_broadcast() {
                true => {
                    let (tx_sub, rx_sub) = bounded::<ContextMessage<A>>(num);

                    subs.push(Sender::from(tx_sub));

                    Some(rx_sub)
                }
                false => None,
            };

            ActorContext::new(
                i,
                tx.downgrade(),
                rx.clone(),
                rx_sub.map(Into::into),
                actor,
                state.clone(),
            )
            .spawn_loop();
        }

        Address::new(tx, subs.into(), state)
    }

    #[cfg(feature = "actix-runtime-local")]
    pub async fn start(self) -> Address<A> {
        let num = self.config.num;

        assert_eq!(
            num, 1,
            "It doesn't make sense to construct multiple instances of actor on single thread."
        );

        let (tx, rx) = unbounded_channel::<ContextMessage<A>>();

        let state = ActorState::new(self.config);
        let mut subs = Vec::with_capacity(num);

        let actor = self.actor_builder.build().await;

        let rx_sub = match state.allow_broadcast() {
            true => {
                let (tx_sub, rx_sub) = bounded::<ContextMessage<A>>(num);

                subs.push(Sender::from(tx_sub));

                Some(rx_sub)
            }
            false => None,
        };

        ActorContext::new(
            0,
            tx.downgrade(),
            rx,
            rx_sub.map(Into::into),
            actor,
            state.clone(),
        )
        .spawn_loop();

        Address::new(tx, subs.into(), state)
    }

    /// Start actors on the given arbiter slice.
    ///
    /// Actors would try to spawn evenly on the given arbiters.
    #[cfg(feature = "actix-runtime")]
    pub async fn start_with_arbiter(self, arbiters: &[actix_rt::Arbiter]) -> Address<A> {
        let num = self.config.num;

        let (tx, rx) = unbounded_channel::<ContextMessage<A>>();

        let state = ActorState::new(self.config);
        let mut subs = Vec::with_capacity(num);

        let len = arbiters.len();

        for i in 0..num {
            let index = i % len;

            let actor = self.actor_builder.build().await;

            let rx_sub = match state.allow_broadcast() {
                true => {
                    let (tx_sub, rx_sub) = bounded::<ContextMessage<A>>(num);

                    subs.push(Sender::from(tx_sub));

                    Some(rx_sub)
                }
                false => None,
            };

            let ctx = ActorContext::new(
                i,
                tx.downgrade(),
                rx.clone().into(),
                rx_sub.map(Into::into),
                actor,
                state.clone(),
            );

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

fn unbounded_channel<A>() -> (Sender<A>, Receiver<A>) {
    let (tx, rx) = unbounded::<A>();

    (tx.into(), rx.into())
}
