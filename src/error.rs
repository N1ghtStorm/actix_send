use std::fmt::{Debug, Formatter, Result as FmtResult};

use async_channel::SendError;
use futures::channel::oneshot::Canceled;

use crate::actor::Actor;
use crate::context::ChannelMessage;

pub enum ActixSendError {
    Canceled,
    Closed,
    Blocking,
    TypeCast,
}

impl Debug for ActixSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let mut fmt = f.debug_struct("ActixSendError");

        match self {
            ActixSendError::TypeCast => fmt
                .field("cause", &"TypeCast")
                .field(
                    "description",
                    &"Failed to downcast Message's result type from Actor::Result",
                )
                .finish(),
            ActixSendError::Canceled => fmt
                .field("cause", &"Canceled")
                .field(
                    "description",
                    &"Oneshot channel is closed before we send anything through it",
                )
                .finish(),
            ActixSendError::Closed => fmt
                .field("cause", &"Closed")
                .field("description", &"Actor's message channel is closed")
                .finish(),
            ActixSendError::Blocking => fmt
                .field("cause", &"Blocking")
                .field("description", &"Failed to run blocking code")
                .finish(),
        }
    }
}

impl From<Canceled> for ActixSendError {
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<A> From<SendError<ChannelMessage<A>>> for ActixSendError
where
    A: Actor,
{
    fn from(_err: SendError<ChannelMessage<A>>) -> Self {
        ActixSendError::Closed
    }
}
