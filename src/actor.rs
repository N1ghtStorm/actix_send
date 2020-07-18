use core::future::Future;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::builder::{Builder, BuilderFnContainer, Config};
use crate::interval::IntervalFutureSet;
use crate::util::future_handle::FutureHandler;

pub trait Actor
where
    Self: Sized + Send,
{
    type Message: Send;
    type Result: Send;

    /// define a new builder for an new set of actor(s) with the async closure.
    fn builder<F, Fut>(f: F) -> Builder<Self>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = Self> + Send + 'static,
    {
        Builder {
            actor_builder: BuilderFnContainer::new(f),
            config: Default::default(),
        }
    }

    /// Called when actor starts.
    ///
    /// *. This would apply to every single instance of actor(s)
    ///
    /// *. This would apply to restart process if `Builder::restart_on_err` is set to true
    #[allow(unused_variables)]
    fn on_start(&mut self) {}

    /// Called before actor stop. Actor's context would be passed as argument.
    ///
    /// *. This would apply to every single instance of actor(s)
    #[allow(unused_variables)]
    fn on_stop(&mut self) {}
}

// a marker bit for lower bit of usize. can be used to notify if address is dropping.
const MARKER: usize = 1;
// do not replace marker bit when we doing add or sub.
const UNIT: usize = 1 << 1;

// A state shared between a set of actors.
pub(crate) struct ActorState<A>
where
    A: Actor,
{
    // The count of actors we actually spawned.
    // With the last bit as MARKER to indicate if the address is dropping.
    active: Arc<AtomicUsize>,
    // Actors delayed and interval task handlers. Used to drop all tasks when actor is shutdown.
    handlers: Arc<Mutex<Vec<FutureHandler<A>>>>,
    // All the interval future objects are stored in this set.
    pub(crate) interval_futures: IntervalFutureSet<A>,
    // config for setting inherent from Builder.
    config: Config,
}

impl<A> Clone for ActorState<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            active: self.active.clone(),
            handlers: self.handlers.clone(),
            interval_futures: self.interval_futures.clone(),
            config: self.config.clone(),
        }
    }
}

impl<A> ActorState<A>
where
    A: Actor + 'static,
{
    pub(crate) fn new(config: Config) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            handlers: Default::default(),
            interval_futures: Default::default(),
            config,
        }
    }

    pub(crate) fn push_handler(&self, handler: Vec<FutureHandler<A>>) {
        // Only push the handler if actors not shutdown.
        if self.is_running() {
            self.handlers.lock().unwrap().extend(handler);
        }
    }

    pub(crate) fn restart_on_err(&self) -> bool {
        self.config.restart_on_err
    }

    pub(crate) fn handle_delay_on_shutdown(&self) -> bool {
        self.config.handle_delayed_on_shutdown
    }

    pub(crate) fn allow_broadcast(&self) -> bool {
        self.config.allow_broadcast
    }

    pub(crate) fn allow_subscribe(&self) -> bool {
        self.config.allow_subscribe
    }

    pub(crate) fn inc_active(&self) -> bool {
        self.modify_active(|active| active + UNIT)
    }

    pub(crate) fn dec_active(&self) -> bool {
        self.modify_active(|active| active - UNIT)
    }

    fn modify_active<F>(&self, mut f: F) -> bool
    where
        F: FnMut(usize) -> usize,
    {
        let mut active = self.active.load(Ordering::Relaxed);
        loop {
            if active & MARKER != 0 {
                return false;
            }

            let new = f(active);

            match self.active.compare_exchange_weak(
                active,
                new,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(a) => active = a,
            }
        }
    }

    pub(crate) fn current_active(&self) -> usize {
        let state = self.active.load(Ordering::SeqCst);
        state >> 1
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.config.timeout
    }

    pub(crate) fn shutdown(&self) {
        // We write marker to the last bit of active usize.
        self.active.fetch_or(MARKER, Ordering::Relaxed);
        // cancel all the actors future handlers for delayed and interval tasks.
        for handler in self.handlers.lock().unwrap().iter() {
            handler.cancel();
        }
    }

    fn is_running(&self) -> bool {
        self.active.load(Ordering::Relaxed) & MARKER == 0
    }
}

#[async_trait]
pub trait Handler
where
    Self: Actor,
{
    async fn handle(&mut self, msg: Self::Message) -> Self::Result;
}
