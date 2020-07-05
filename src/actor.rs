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
    fn on_start(&mut self) {}

    /// Called before actor stop. Actor's context would be passed as argument.
    ///
    /// *. This would apply to every single instance of actor(s)
    #[allow(unused_variables)]
    fn on_stop(actor: Self) {}
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
            active: Arc::new(AtomicUsize::new(config.num << 1)),
            handlers: Default::default(),
            interval_futures: Default::default(),
            config,
        }
    }

    pub(crate) fn push_handler(&self, handler: Vec<FutureHandler<A>>) {
        self.handlers.lock().extend(handler);
    }

    // pub(crate) fn restart_on_err(&self) -> bool {
    //     self.config.restart_on_err
    // }

    pub(crate) fn handle_delay_on_shutdown(&self) -> bool {
        self.config.handle_delayed_on_shutdown
    }

    // pub(crate) fn inc_active(&self) -> bool {
    //     let mut active = self.active.load(Ordering::Acquire);
    //     loop {
    //         // We are shutdown so we return with false
    //         if active & MARKER != 0 {
    //             return false;
    //         }
    //
    //         let new = ((active >> 1) + 1) << 1;
    //
    //         // there could be other thread trying to
    //         if self.active.compare_and_swap(active, new, Ordering::Release) == active {
    //             return true;
    //         }
    //
    //         std::thread::yield_now();
    //         active = self.active.load(Ordering::Acquire);
    //     }
    // }

    // pub(crate) fn dec_active(&self) -> bool {
    //     let mut active = self.active.load(Ordering::Acquire);
    //     loop {
    //         // if active & MARKER != 0 {
    //         //     return false;
    //         // }
    //
    //         let new = ((active >> 1) - 1) << 1;
    //
    //         if self.active.compare_and_swap(active, new, Ordering::Release) == active {
    //             return true;
    //         }
    //         std::thread::yield_now();
    //         active = self.active.load(Ordering::Acquire);
    //     }
    // }
    //
    // pub(crate) fn current_active(&self) -> usize {
    //     let state = self.active.load(Ordering::Acquire);
    //     state >> 1
    // }

    // pub(crate) fn set_active(&self, active: usize) {
    //     self.active.store(active << 1, Ordering::Release);
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

    // pub(crate) fn is_running(&self) -> bool {
    //     self.active.load(Ordering::Acquire) & MARKER == 0
    // }

    // pub(crate) fn restart(self) {
    //     if self.config.restart_on_err {}
    // }
}

#[async_trait::async_trait]
pub trait Handler
where
    Self: Actor,
{
    async fn handle(&mut self, msg: Self::Message) -> Self::Result;
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
pub mod test_actor {
    use crate::prelude::*;

    #[actor_mod]
    pub mod my_actor {
        use crate::prelude::*;

        #[actor]
        pub struct TestActor {
            pub state1: String,
            pub state2: String,
        }

        #[message(result = "u8")]
        pub struct DummyMessage1 {
            pub from: String,
        }

        #[message(result = "u16")]
        pub struct DummyMessage2(pub u32, pub usize);

        #[handler]
        impl Handler for TestActor {
            // The msg and handle's return type must match former message macro's result type.
            async fn handle(&mut self, msg: DummyMessage1) -> u8 {
                assert_eq!("running1", self.state1);
                8
            }
        }

        #[handler]
        impl Handler for TestActor {
            async fn handle(&mut self, msg: DummyMessage2) -> u16 {
                assert_eq!("running2", self.state2);
                16
            }
        }
    }

    #[tokio::test]
    async fn test() {
        use super::test_actor::my_actor::*;

        let state1 = String::from("running1");
        let state2 = String::from("running2");
        let actor = TestActor::create(|| TestActor { state1, state2 });

        // build and start the actor(s).
        let address: Address<TestActor> = actor.build().num(1).start();

        // construct a new message instance and convert it to a MessageObject
        let msg = DummyMessage1 {
            from: "a simple test".to_string(),
        };

        let msg2 = DummyMessage2(1, 2);

        // use address to send message object to actor and await on result.
        let res = address.send(msg).await.unwrap();

        let res2 = address.send(msg2).await.unwrap();

        assert_eq!(res, 8);
        assert_eq!(res2, 16);
    }
}
