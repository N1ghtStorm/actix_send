## Job of this macro set:
- ### Generate boilerplate implementation for actors.<br> This includes:
  - `Actor::create` method for creating actor instance.
  - `BoxedIntervalFuture` implementation for actors.
  - `Actor` trait implementation for actors.  
  - `Message` trait implementation for actors.
  - `Handler` trait implementation for actors.(Including `async_trait` implementation)

- ### Generate type convertion for actors.<br> This includes:
  - `ActorMessage` enum contains all message types as variant.
  - `ActorMessageResult` enum contain all message types' result types as variant.
  - `From` trait for converting all variants to `ActorMessage` and from `ActorMessageResult`(to actual result types)