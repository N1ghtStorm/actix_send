//! actix_send is an actor pattern loosely based on the design of [actix](https://crates.io/crates/actix) that can be run on various runtime.
//!
//! # Limitation:
//!
//! - Generics in actor and message are not handled properly by macros.
//!
//! # Example:
//! ```rust
//! use actix_send::prelude::*;
//!
//! use my_actor::*;
//!
//! #[tokio::main]
//! async fn main() {
//!     let state = String::from("running");
//!
//!     // create an actor instance. The create function would return our Actor instance.
//!     let actor = MyActor::create(|| MyActor { state });
//!
//!     // build and start the actor(s).
//!     let address: Address<MyActor> = actor.build().start();
//!
//!     // construct new messages.
//!     let msg = MyMessage {
//!         from: "a simple test".to_string(),
//!     };
//!
//!     // use address to send messages to actor and await on result.
//!     let res: Result<u8, ActixSendError> = address.send(msg).await;
//! }
//!
//! /*  Implementation of actor */
//!
//! // put actor/message/handler in a mod
//! #[actor_mod]
//! pub mod my_actor {
//!     use super::*;
//!
//!     // our actor type
//!     #[actor]
//!     pub struct MyActor {
//!         pub state: String
//!     }
//!
//!     // our message type with it's associate result type
//!     #[message(result = "u8")]
//!     pub struct MyMessage {
//!         pub from: String,
//!     }
//!
//!     #[handler]
//!     impl Handler for MyActor {
//!         // The msg and handle's return type must match former message macro's result type.
//!         async fn handle(&mut self, _msg: MyMessage) -> u8 {
//!             8
//!         }
//!     }
//! }
//! ```
//! # Features
//! | Feature | Description | Extra dependencies | Default |
//! | ------- | ----------- | ------------------ | ------- |
//! | `default` | The same as `tokio-runtime` feature | [actix-send-macros](https://github.com/fakeshadow/actix_send)<br>[async-channel](https://crates.io/crates/async-channel)<br>[async-trait](https://crates.io/crates/async-trait)<br>[futures](https://crates.io/crates/futures)<br>[parking_lot](https://crates.io/crates/parking_lot)<br>[pin-project](https://crates.io/crates/pin-project) | yes |
//! | `tokio-runtime` | Enable support for the `tokio` crate. | [tokio](https://crates.io/crates/tokio) | yes |
//! | `async-std-runtime` | Enable support for the `async-std` crate. | [async-std](https://crates.io/crates/async-std)<br>[tokio](https://crates.io/crates/tokio) with `sync` feature | no |
//! | `actix-runtime` | Enable support for the `actix-rt` crate. | [actix-rt](https://crates.io/crates/actix-rt)<br>[tokio](https://crates.io/crates/tokio) with `sync` feature | no |

pub(crate) mod actor;
pub(crate) mod address;
pub(crate) mod builder;
pub(crate) mod context;
pub(crate) mod error;
pub(crate) mod interval;
pub(crate) mod object;
pub(crate) mod stream;
pub(crate) mod util;

pub mod prelude {
    pub use crate::actor::{Actor, Handler};
    pub use crate::address::{Address, MapResult, WeakAddress};
    pub use crate::error::ActixSendError;
    pub use crate::util::runtime::spawn_blocking as actix_send_blocking;
    pub use actix_send_macros::*;
    pub use async_trait::async_trait;
}

#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("Only one runtime can be enabled");
