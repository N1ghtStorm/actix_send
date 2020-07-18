use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::Arc;

use futures_util::future::Either;

use crate::actor::Actor;
use crate::context::{ContextMessage, IntervalMessage};
use crate::sender::WeakSender;
use crate::util::lock::SimpleSpinLock;
use crate::util::runtime;

macro_rules! spawn_cancel {
    ($($send:ident)*) => {
        // helper function for spawn a future on runtime and return a handler that can cancel it.
        pub(crate) fn spawn_cancelable<F, A, FN, Fut>(f: F, on_ready: FN) -> FutureHandler<A>
        where
            A: Actor,
            F: Future + Unpin $( + $send)* + 'static,
            <F as Future>::Output: Send,
            FN: FnOnce(Either<((), F), (<F as Future>::Output, FinisherFuture)>) -> Fut
                $( + $send)*
                + 'static,
            Fut: Future<Output = ()> $( + $send)*,
        {
            let waker = Arc::new(SimpleSpinLock::new(None));

            let finisher = FinisherFuture {
                waker: waker.clone(),
            };

            let future = futures_util::future::select(finisher, f);
            let handler = FutureHandler {
                waker,
                tx: None,
            };

            runtime::spawn(async {
                let either = future.await;
                on_ready(either).await;
            });

            handler
        }
    };
}

#[cfg(not(feature = "actix-runtime"))]
spawn_cancel!(Send);

#[cfg(feature = "actix-runtime")]
spawn_cancel!();

// a future notified and polled by future_handler.
pub(crate) struct FinisherFuture {
    waker: Arc<SimpleSpinLock<Option<Waker>>>,
}

impl Future for FinisherFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.waker.lock() {
            None => Poll::Ready(()),
            Some(mut guard) => {
                *guard = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

pub struct FutureHandler<A>
where
    A: Actor,
{
    waker: Arc<SimpleSpinLock<Option<Waker>>>,
    tx: Option<(usize, WeakSender<ContextMessage<A>>)>,
}

impl<A> Clone for FutureHandler<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            waker: self.waker.clone(),
            tx: self.tx.as_ref().map(|(idx, sender)| (*idx, sender.clone())),
        }
    }
}

impl<A> FutureHandler<A>
where
    A: Actor + 'static,
{
    /// Cancel the future.
    pub fn cancel(&self) {
        if let Some(mut guard) = self.waker.lock() {
            let opt = guard.take();

            drop(guard);
            self.waker.close();

            if let Some(waker) = opt {
                waker.wake();
            }
        }

        // We remove the interval future with index as key.
        if let Some((index, tx)) = self.tx.as_ref() {
            if let Some(tx) = tx.upgrade() {
                let index = *index;
                runtime::spawn(async move {
                    let _ = tx
                        .send(ContextMessage::Interval(IntervalMessage::Remove(index)))
                        .await;
                });
            }
        }
    }

    pub(crate) fn attach_tx(&mut self, index: usize, tx: WeakSender<ContextMessage<A>>) {
        self.tx = Some((index, tx));
    }
}
