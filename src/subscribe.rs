use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::time::Duration;

use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use crate::actor::Actor;
use crate::context::{ContextMessage, InstantMessage};
use crate::error::ActixSendError;
use crate::object::AnyObjectContainer;
use crate::sender::WeakSender;
use crate::util::runtime;

// subscribe hold a vector of trait objects which are boxed Subscriber that contains
// Sender<ContextMessage<Actor>> and an associate message type.
macro_rules! subscribe {
    ($($send:ident)*) => {
        pub(crate) struct Subscribe {
            inner: Arc<AsyncMutex<Vec<Box<dyn SubscribeTrait $( + $send)*>>>>,
        }

        impl Subscribe {
            pub(crate) async fn lock(&self) -> AsyncMutexGuard<'_, Vec<Box<dyn SubscribeTrait $( + $send)*>>> {
                self.inner.lock().await
            }
        }

        // we make Subscriber to trait object so they are not bound to Actor and Message Type.
        #[allow(clippy::type_complexity)]
        pub(crate) trait SubscribeTrait {
            fn send(
                &self,
                // Input message type have to be boxed too.
                msg: AnyObjectContainer,
                timeout: Duration,
            ) -> Pin<Box<dyn Future<Output = Option<Result<(), ActixSendError>>> $( + $send)* + '_>>;
        }

        #[allow(clippy::type_complexity)]
        impl<A, M> SubscribeTrait for Subscriber<A, M>
        where
            A: Actor + 'static,
            M: Send + Into<A::Message> + 'static,
        {
            fn send(
                &self,
                mut msg: AnyObjectContainer,
                timeout: Duration,
            ) -> Pin<Box<dyn Future<Output = Option<Result<(), ActixSendError>>> $( + $send)* + '_>> {
                Box::pin(async move {
                    // We downcast message trait object to the Message type of WeakSender.

                    let msg = msg.unpack::<M>()?;
                    let res = self._send(msg, timeout).await;
                    Some(res)
                })
            }
        }
    }
}

#[cfg(not(feature = "actix-runtime-local"))]
subscribe!(Send);

#[cfg(feature = "actix-runtime-local")]
subscribe!();

impl Default for Subscribe {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl Clone for Subscribe {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Subscribe {
    pub(crate) async fn push<A, M>(&self, sender: WeakSender<ContextMessage<A>>)
    where
        A: Actor + 'static,
        M: Send + Into<A::Message> + 'static,
    {
        self.lock().await.push(Box::new(Subscriber {
            sender,
            _message: PhantomData::<JoinHandle<M>>,
        }));
    }
}

struct Subscriber<A, M>
where
    A: Actor + 'static,
    M: Send + 'static,
{
    sender: WeakSender<ContextMessage<A>>,
    _message: PhantomData<JoinHandle<M>>,
}

impl<A, M> Subscriber<A, M>
where
    A: Actor + 'static,
    M: Send + Into<A::Message> + 'static,
{
    async fn _send(&self, msg: M, timeout: Duration) -> Result<(), ActixSendError> {
        let sender = self.sender.upgrade().ok_or(ActixSendError::Closed)?;
        let f = sender.send(ContextMessage::Instant(InstantMessage::Static(
            None,
            msg.into(),
        )));

        runtime::timeout(timeout, f).await??;

        Ok(())
    }
}
