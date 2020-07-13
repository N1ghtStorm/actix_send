use std::fmt::{Debug, Formatter, Result as FmtResult};

use async_channel::SendError;
use futures_channel::oneshot::Canceled;

use crate::actor::Actor;
use crate::context::ContextMessage;

pub enum ActixSendError {
    Canceled,
    Closed,
    Timeout,
    Blocking,
    TypeCast,
}

impl Debug for ActixSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let mut fmt = f.debug_struct("ActixSendError");

        match self {
            ActixSendError::TypeCast => fmt.field("cause", &"TypeCast").field(
                "description",
                &"Failed to downcast Message's result type from Actor::Result",
            ),
            ActixSendError::Timeout => fmt
                .field("cause", &"Timeout")
                .field("description", &"Could not process the message in time"),
            ActixSendError::Canceled => fmt.field("cause", &"Canceled").field(
                "description",
                &"Oneshot channel is closed before we send anything through it",
            ),
            ActixSendError::Closed => fmt
                .field("cause", &"Closed")
                .field("description", &"Actor's message channel is closed"),
            ActixSendError::Blocking => fmt
                .field("cause", &"Blocking")
                .field("description", &"Failed to run blocking code"),
        };

        fmt.finish()
    }
}

impl From<Canceled> for ActixSendError {
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<A> From<SendError<ContextMessage<A>>> for ActixSendError
where
    A: Actor,
{
    fn from(_err: SendError<ContextMessage<A>>) -> Self {
        ActixSendError::Closed
    }
}
