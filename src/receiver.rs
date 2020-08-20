use core::pin::Pin;
use core::task::{Context, Poll};

use futures_util::stream::Stream;

use crate::error::ActixSendError;
use crate::util::channel::Receiver as AsyncChannelReceiver;

// A wrapper for crate::util::channel::Receiver so we have a unified abstraction for different
// channels

#[cfg(not(feature = "actix-runtime-local"))]
pub mod recv {
    use super::*;

    impl<M> Clone for Receiver<M> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    impl<M> Receiver<M> {
        pub(crate) async fn recv(&self) -> Result<M, ActixSendError> {
            self.inner.recv().await.map_err(|_| ActixSendError::Closed)
        }
    }
}

#[cfg(feature = "actix-runtime-local")]
pub mod recv {
    use super::*;
    use futures_util::stream::StreamExt;

    impl<M> Receiver<M> {
        pub(crate) async fn recv(&mut self) -> Result<M, ActixSendError> {
            self.inner.next().await.ok_or(ActixSendError::Closed)
        }
    }
}

pub struct Receiver<M> {
    pub(super) inner: AsyncChannelReceiver<M>,
}

impl<M> From<AsyncChannelReceiver<M>> for Receiver<M> {
    fn from(inner: AsyncChannelReceiver<M>) -> Self {
        Self { inner }
    }
}

impl<M> Stream for Receiver<M> {
    type Item = M;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.inner).poll_next(cx)
    }
}
