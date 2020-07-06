use crate::my_actor::*;
use actix_send::prelude::*;

#[actor_mod]
pub mod my_actor {
    use super::*;

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
async fn basic() {
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
