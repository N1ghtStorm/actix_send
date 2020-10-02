## Job of this macro set:
- ### Generate boilerplate implementation for actors.<br> This includes:
  - `Actor` trait implementation for actors.
  - `Handler` trait implementation for actors.(Including `async_trait` implementation)

- ### Generate type conversion for actors.<br> This includes:
  - `ActorMessage` enum contains all message types as variant.
  - `ActorMessageResult` enum contain all message types' result types as variant.
  - `From` trait for converting all variants to `ActorMessage`
  - `MapResult` trait for map all result variants to the original message's result type