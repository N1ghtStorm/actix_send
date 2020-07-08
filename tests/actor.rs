use std::time::Duration;

use actix_send::prelude::*;

use crate::my_actor::*;

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
        async fn handle(&mut self, _: DummyMessage1) -> u8 {
            assert_eq!("running1", self.state1);
            8
        }
    }

    #[handler]
    impl Handler for TestActor {
        async fn handle(&mut self, _: DummyMessage2) -> u16 {
            assert_eq!("running2", self.state2);
            16
        }
    }
}

#[tokio::test]
async fn basic() {
    let actor = test_actor();

    let address = actor.build().num(1).start();

    let msg = DummyMessage1 {
        from: "a simple test".to_string(),
    };

    let msg2 = DummyMessage2(1, 2);

    let res = address.send(msg).await.unwrap();
    let res2 = address.send(msg2).await.unwrap();

    assert_eq!(res, 8);
    assert_eq!(res2, 16);
}

#[tokio::test]
async fn weak_addr() {
    let actor = test_actor();

    let address = actor.build().start();

    let weak = address.downgrade();
    drop(address);

    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn active_count() {
    let actor = test_actor();

    let address = actor.build().num(8).start();

    let _ = tokio::time::delay_for(Duration::from_secs(1)).await;

    assert_eq!(address.current_active(), 8);
}

fn test_actor() -> TestActor {
    let state1 = String::from("running1");
    let state2 = String::from("running2");
    TestActor::create(|| TestActor { state1, state2 })
}
