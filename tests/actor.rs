use core::time::Duration;

use actix_send::prelude::*;
use actix_send::Builder;

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
        async fn handle(&mut self, msg123: DummyMessage1) -> u8 {
            assert_eq!("running1", self.state1);
            assert_eq!("a simple test", msg123.from.as_str());
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
    let address = test_actor_builder().num(1).start().await;

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
    let address = test_actor_builder().start().await;

    let weak = address.downgrade();
    drop(address);

    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn active_count() {
    let address = test_actor_builder().num(8).start().await;

    let _ = tokio::time::delay_for(Duration::from_secs(1)).await;

    assert_eq!(address.current_active(), 8);

    let _ = address.close_one().await;
    let _ = address.close_one().await;
    let _ = address.close_one().await;

    assert_eq!(address.current_active(), 5);
}

fn test_actor_builder() -> Builder<TestActor> {
    TestActor::builder(|| async {
        let state1 = String::from("running1");
        let state2 = String::from("running2");

        TestActor { state1, state2 }
    })
}
