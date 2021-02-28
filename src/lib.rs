//! actix_send is an actor pattern loosely based on the design of [actix](https://crates.io/crates/actix) that can be run on various runtime.
//!
//! # Limitation:
//!
//! - Generics in actor and message may not handled properly by macros.
//!
//! # Example:
//! ```rust
//! use actix_send::prelude::*;
//!
//! // Actor type
//! #[actor]
//! pub struct MyActor;
//!
//! // message types
//! pub struct Message1;
//! pub struct Message2(u128);
//! pub struct Message3 {
//!     from: String,
//!     content: Option<Vec<i64>>
//! }
//!
//! // handler implement
//! #[handler_v2]
//! impl MyActor {
//!     // the name of handle method is not important.
//!     // It's only used to make your IDE/editor happy when actually we transform them to
//!     // Handler trait and Handler::handle method.
//!     async fn ra_ri_ru_rei_ro(&mut self, _: Message1) -> u8 {
//!         8
//!     }
//!
//!     async fn handle(&mut self, _:Message2) -> u16 {
//!         16
//!     }
//!
//!     // This is anti-pattern and your IDE/editor would most likely complain.
//!     // But it's Ok to shadow name method and our #[handler_v2] macro would take care of it.
//!     async fn handle(&mut self, _:Message3) -> u8 {
//!         88
//!     }   
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!
//!     // define a builder for our actor. The async closure would return our Actor instance.
//!     let builder = MyActor::builder(|| async { MyActor });
//!
//!     // start actor(s).
//!     let address: Address<MyActor> = builder.start().await;
//!
//!     /*
//!        send messages to actor.
//!
//!        No matter how we name the handle method for a give <MessageType> in impl MyActor
//!        we can just call Address::send(<MessageType>) and according handle method will be called.
//!     */
//!
//!     let res1: Result<u8, ActixSendError> = address.send(Message1).await;
//!     let res2: Result<u16, ActixSendError> = address.send(Message2(9527)).await;
//!     let res3 = address.send(Message3 { from: String::from("wife"), content: None }).await;
//!
//!     assert_eq!(8, res1.unwrap());
//!     assert_eq!(16, res2.unwrap());
//!     assert_eq!(88, res3.unwrap());
//! }
//! ```
//!
//! # Example for actix style:
//! ```rust
//! use actix_send::prelude::*;
//!
//! use my_actor::*;
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
//!
//! #[tokio::main]
//! async fn main() {
//!     
//!     // define a builder of our actor. The async closure would return our Actor instance.
//!     let builder = MyActor::builder(|| async {
//!         let state = String::from("running");
//!         MyActor { state }
//!     });
//!
//!     // start the actor(s).
//!     let address: Address<MyActor> = builder.start().await;
//!
//!     // construct new message.
//!     let msg = MyMessage {
//!         from: "a simple test".to_string(),
//!     };
//!
//!     // use address to send messages to actor and await on result.
//!     let res: Result<u8, ActixSendError> = address.send(msg).await;
//! }
//! ```
//! # Features
//! | Feature | Description | Extra dependencies | Default |
//! | ------- | ----------- | ------------------ | ------- |
//! | `default` | The same as `tokio-runtime` feature | [actix-send-macros](https://github.com/fakeshadow/actix_send)<br>[async-trait](https://crates.io/crates/async-trait)<br>[futures_util](https://crates.io/crates/futures-util)<br>[pin-project](https://crates.io/crates/pin-project) | yes |
//! | `tokio-runtime` | Enable support for the `tokio` crate. | [async-channel](https://crates.io/crates/async-channel)<br>[tokio](https://crates.io/crates/tokio) | yes |
//! | `async-std-runtime` | Enable support for the `async-std` crate. | [async-channel](https://crates.io/crates/async-channel)<br>[async-std](https://crates.io/crates/async-std)<br>[tokio](https://crates.io/crates/tokio) with `sync` feature | no |
//! | `actix-runtime` | Enable support for the `actix-rt` crate. | [actix-rt](https://crates.io/crates/actix-rt)<br>[async-channel](https://crates.io/crates/async-channel)<br>[tokio](https://crates.io/crates/tokio) with `sync` feature | no |
//! | `actix-runtime-mpsc` | Enable support for mpsc actor for `actix-rt`. actor runs on single thread with a thread safe sender for message. | [actix-rt](https://crates.io/crates/actix-rt)<br>[tokio](https://crates.io/crates/tokio) with `sync` feature | no |

#![forbid(unsafe_code)]
#![deny(unused_variables)]

pub(crate) mod actor;
pub(crate) mod address;
pub(crate) mod builder;
pub(crate) mod context;
pub(crate) mod error;
pub(crate) mod interval;
pub(crate) mod object;
pub(crate) mod receiver;
pub(crate) mod sender;
pub(crate) mod stream;
pub(crate) mod subscribe;
pub(crate) mod util;

pub mod prelude {
    pub use crate::actor::{Actor, Handler};
    pub use crate::address::{Address, MapResult, WeakAddress};
    pub use crate::builder::Builder;
    pub use crate::error::ActixSendError;
    pub use crate::stream::{ActorSkipStream, ActorStream};
    pub use crate::util::runtime::spawn_blocking as actix_send_blocking;
    pub use actix_send_macros::*;
    pub use async_trait::async_trait;
}

pub use crate::builder::Builder;

#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("Only one runtime can be enabled");
