use async_channel::unbounded;

use crate::address::Address;
use crate::context::{spawn_loop, ChannelMessage};
use crate::interval::IntervalFutureSet;

pub struct Builder<A>
where
    A: Actor,
{
    actor: A,
    num: usize,
}

impl<A> Builder<A>
where
    A: Actor + Handler + Clone + 'static,
    A::Message: Send + 'static,
    A::Result: Send,
{
    /// Build multiple actors with the num passed.
    ///
    /// All the actors would steal work from a single `async-channel`.
    pub fn num(mut self, num: usize) -> Self {
        Self::check_num(num, 0);
        self.num = num;
        self
    }

    pub fn start(self) -> Address<A> {
        let num = self.num;

        let (tx, rx) = unbounded::<ChannelMessage<A>>();

        let mut handlers = Vec::new();

        let interval_futures = IntervalFutureSet::new();

        if num > 1 {
            for _ in 0..num {
                let handler = spawn_loop(
                    self.actor.clone(),
                    tx.clone(),
                    rx.clone(),
                    interval_futures.clone(),
                );
                handlers.push(handler);
            }
        } else {
            let handler = spawn_loop(self.actor, tx.clone(), rx, interval_futures);
            handlers.push(handler);
        }

        Address::new(tx, handlers)
    }

    fn check_num(num: usize, target: usize) {
        assert!(
            num > target,
            "The number of actors must be larger than {}",
            target
        );
    }
}

pub trait Actor
where
    Self: Sized + Send,
{
    type Message;
    type Result;

    fn build(self) -> Builder<Self> {
        Builder {
            actor: self,
            num: 1,
        }
    }

    fn create<F>(f: F) -> Self
    where
        F: FnOnce() -> Self,
    {
        f()
    }
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
