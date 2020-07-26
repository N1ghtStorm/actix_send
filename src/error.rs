use core::fmt::{Debug, Display, Formatter, Result as FmtResult};

use async_channel::SendError;
use tokio::sync::oneshot::error::RecvError;

pub enum ActixSendError {
    Canceled,
    Closed,
    Timeout,
    Blocking,
    TypeCast,
    Subscribe,
    Broadcast,
}

impl Debug for ActixSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let mut fmt = f.debug_struct("ActixSendError");

        match self {
            ActixSendError::TypeCast => fmt.field("cause", &"TypeCast").field(
                "description",
                &"Failed to downcast from dyn Any to a concrete type",
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
            ActixSendError::Subscribe => fmt
                .field("cause", &"Subscribe")
                .field("description", &"This address can not be subscribed"),
            ActixSendError::Broadcast => fmt
                .field("cause", &"Broadcast")
                .field("description", &"This address can not broadcasting"),
        };

        fmt.finish()
    }
}

impl Display for ActixSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "({})", self)
    }
}

impl From<RecvError> for ActixSendError {
    fn from(_err: RecvError) -> Self {
        ActixSendError::Canceled
    }
}

impl<M> From<SendError<M>> for ActixSendError {
    fn from(_err: SendError<M>) -> Self {
        ActixSendError::Closed
    }
}

impl std::error::Error for ActixSendError {}
