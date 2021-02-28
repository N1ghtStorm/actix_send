use core::pin::Pin;
use core::task::{Context, Poll};

use futures_util::stream::Stream;

use crate::error::ActixSendError;
use crate::util::channel::Receiver as AsyncChannelReceiver;

// A wrapper for crate::util::channel::Receiver so we have a unified abstraction for different
// channels

#[cfg(not(feature = "actix-runtime-mpsc"))]
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

    impl<M> Stream for Receiver<M> {
        type Item = M;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Pin::new(&mut self.get_mut().inner).poll_next(cx)
        }
    }
}

#[cfg(feature = "actix-runtime-mpsc")]
pub mod recv {
    use super::*;

    impl<M> Receiver<M> {
        pub(crate) async fn recv(&mut self) -> Result<M, ActixSendError> {
            self.inner.recv().await.ok_or(ActixSendError::Closed)
        }
    }

    impl<M> Clone for Receiver<M> {
        fn clone(&self) -> Self {
            panic!("You should not call clone on a mpsc receiver");
        }
    }

    impl<M> Stream for Receiver<M> {
        type Item = M;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.get_mut().inner.poll_recv(cx)
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
