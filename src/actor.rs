use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::builder::{Builder, Config};
use crate::interval::IntervalFutureSet;
use crate::util::future_handle::FutureHandler;

pub trait Actor
where
    Self: Sized + Send,
{
    type Message: Send;
    type Result: Send;

    fn build(self) -> Builder<Self> {
        Builder {
            actor: self,
            config: Default::default(),
        }
    }

    /// Create an new actor with the closure.
    fn create<F>(f: F) -> Self
    where
        F: FnOnce() -> Self,
    {
        f()
    }

    /// Called when actor starts.
    ///
    /// *. This would apply to every single instance of actor(s)
    ///
    /// 8. This would apply to restart process if `Builder::restart_on_err` is set to true
    fn on_start(&mut self) {}

    /// Called before actor stop. Actor's context would be passed as argument.
    ///
    /// *. This would apply to every single instance of actor(s)
    #[allow(unused_variables)]
    fn on_stop(&mut self) {}
}

// a marker bit to notify if the Address is dropping.
const MARKER: usize = 1;

// A state shared between a set of actors.
pub(crate) struct ActorState<A>
where
    A: Actor + 'static,
{
    // The count of actors we actually spawned.
    // With the last bit as MARKER to indicate if the address is dropping.
    active: Arc<AtomicUsize>,
    // All the actors context loop handlers. Can be used to shutdown any actor.
    handlers: Arc<Mutex<Vec<FutureHandler<A>>>>,
    // All the interval futures are store in a hashmap
    pub(crate) interval_futures: IntervalFutureSet<A>,
    // config for setting inherent from Builder.
    config: Config,
}

impl<A> Clone for ActorState<A>
where
    A: Actor + 'static,
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
        self.handlers.lock().extend(handler);
    }

    pub(crate) fn restart_on_err(&self) -> bool {
        self.config.restart_on_err
    }

    pub(crate) fn handle_delay_on_shutdown(&self) -> bool {
        self.config.handle_delayed_on_shutdown
    }

    pub(crate) fn inc_active(&self) -> bool {
        self.modify_active(|active| ((active >> 1) + 1) << 1)
    }

    pub(crate) fn dec_active(&self) -> bool {
        self.modify_active(|active| ((active >> 1) - 1) << 1)
    }

    fn modify_active<F>(&self, mut f: F) -> bool
    where
        F: FnMut(usize) -> usize,
    {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active & MARKER != 0 {
                return false;
            }

            let new = f(active);

            if self.active.compare_and_swap(active, new, Ordering::Release) == active {
                return true;
            }

            std::thread::yield_now();
            active = self.active.load(Ordering::Acquire);
        }
    }

    // pub(crate) fn current_active(&self) -> usize {
    //     let state = self.active.load(Ordering::SeqCst);
    //     state >> 1
    // }

    pub(crate) fn shutdown(&self) {
        // We write marker to the last bit of active usize.
        self.active.fetch_or(MARKER, Ordering::SeqCst);
        // cancel all the actors context loop.
        // ToDo: figure a way to graceful shutdown. Maybe we should leave on actor active.
        for handler in self.handlers.lock().iter() {
            handler.cancel();
        }
    }
}

#[async_trait::async_trait]
pub trait Handler
where
    Self: Actor,
{
    async fn handle(&mut self, msg: Self::Message) -> Self::Result;
}
