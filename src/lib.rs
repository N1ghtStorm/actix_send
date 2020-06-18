//! actix_send is an actor pattern loosely based on the design of [actix](https://crates.io/crates/actix) that can be run on various runtime.
//!
//! # Limitation:
//! - Features are minimal with only message processing.
//! - Generics in actor and message are not handled properly by macros.
//! - All types and futures must be `Send + 'static`.
//!
//! # Example:
//! ```rust
//! use actix_send::prelude::*;
//! use async_trait::async_trait;
//!
//! // construct a new actor.
//! #[actor]
//! struct MyActor {
//!     state: String,
//! }
//!
//! // construct a new message with a result type.
//! // message macro must be placed above other marco attributes
//! #[message(result = "Option<MyResult>")]
//! #[derive(Debug)]
//! struct MyMessage {
//!     from: String,
//!     content: String,
//! }
//!
//! // dummy result type for MyMessage
//! #[derive(Debug)]
//! struct MyResult(u32);
//!
//! // impl MyMessage handler for MyActor
//! // handler attribute must be placed above async_trait.
//! #[handler]
//! #[async_trait]
//! impl Handler for MyActor {
//!     // The msg and handle's return type must match former message macro's result type.
//!     async fn handle(&mut self, msg: MyMessage) -> Option<MyResult> {
//!         println!(
//!             "Actor state is: {}\r\n\r\nGot Message from: {}\r\n\r\nContent: {}",
//!             self.state,
//!             msg.from.as_str(),
//!             msg.content.as_str()
//!         );
//!
//!         Some(MyResult(123))
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     // create an actor instance. The args passed to create function are in the same order of your Actor's struct fields.
//!     let state = String::from("running");
//!     let actor = MyActor::create(state);
//!
//!     // build and start the actor.
//!     let address = actor.build().start();
//!
//!     // use address to send message to actor and await on result.
//!     let result = address
//!         .send(MyMessage {
//!             from: "actix-send".to_string(),
//!             content: "a simple test".to_string(),
//!        })
//!         .await;
//!
//!    println!("We got result for message: {:?}", result);
//! }
//! ```
//! # Features
//! | Feature | Description | Extra dependencies | Default |
//! | ------- | ----------- | ------------------ | ------- |
//! | `default` | The same as `tokio-runtime` feature | [async-channel]("https://github.com/stjepang/async-channel")<br>[async-trait](https://crates.io/crates/async-trait)<br>[futures-channel](https://crates.io/crates/futures-channel) | yes |
//! | `tokio-runtime` | Enable support for the `tokio` crate. | [tokio](https://crates.io/crates/tokio) | yes |
//! | `async-std-runtime` | Enable support for the `async-std` crate. | [async-std](https://crates.io/crates/async-std)<br>[tokio](https://crates.io/crates/tokio) with `tokio/sync` feature | no |

pub(crate) mod actors;
pub(crate) mod util;

pub mod prelude {
    pub use crate::actors::{Actor, Handler, Message};
    pub use actix_send_macros::*;
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
mod test_actor {
    use super::prelude::*;
    use async_trait::async_trait;

    #[actor]
    struct MyActor {
        state1: String,
        state2: String,
    }

    #[message(result = "Option<MyResult>")]
    struct MyMessage {
        from: String,
        content: String,
    }

    #[derive(Debug)]
    struct MyResult;

    #[handler]
    #[async_trait]
    impl Handler for MyActor {
        // The msg and handle's return type must match former message macro's result type.
        async fn handle(&mut self, msg: MyMessage) -> Option<MyResult> {
            assert_eq!("actix-send", msg.from.as_str());
            assert_eq!("a simple test", msg.content.as_str());

            assert_eq!("state1", self.state1.as_str());
            assert_eq!("state2", self.state2.as_str());

            Some(MyResult)
        }
    }

    #[tokio::test]
    async fn run() {
        let state1 = String::from("state1");
        let state2 = String::from("state2");
        let act = MyActor::create(state1, state2);

        let address = act.build().start();

        let result = address
            .send(MyMessage {
                from: "actix-send".to_string(),
                content: "a simple test".to_string(),
            })
            .await;

        assert!(result.is_ok());
    }
}

#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("Only one runtime can be enabled");
