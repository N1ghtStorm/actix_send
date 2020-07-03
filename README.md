### `actix_send` is an actor pattern loosely based on the design of [actix](https://crates.io/crates/actix).

### Difference from actix:
- Can run on any async runtime.
- Message is typed into actor and use static dispatch while actix using dynamic dispatch for message using trait object.(Dynamic dispatch is also provided by passing boxed future directly to actor an no message type boilerplate is required.)
- Rely heavily on proc macro to achieve static dispatch mentioned above for multiple messages on one actor and it also brings some boilerplate actix doesn't have.
- All messages and handle futures are forced to be `Send + 'static` so no thread local smart pointer can be used like actix.
