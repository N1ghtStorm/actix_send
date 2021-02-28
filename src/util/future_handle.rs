use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use futures_util::future::Either;

use crate::actor::Actor;
use crate::context::{ContextMessage, IntervalMessage};
use crate::sender::WeakSender;
use crate::util::{
    runtime,
    smart_pointer::{Lock, RefCounter},
};

macro_rules! spawn_cancel {
    ($($send:ident)*) => {
        // helper function for spawn a future on runtime and return a handler that can cancel it.
        pub(crate) fn spawn_cancelable<F, A, FN, Fut>(f: F, on_ready: FN) -> FutureHandler<A>
        where
            A: Actor,
            F: Future $( + $send)* + 'static,
            <F as Future>::Output: $($send)*,
            FN: FnOnce(Either<((), Pin<Box<F>>), (<F as Future>::Output, FinisherFuture)>) -> Fut
                $( + $send)*
                + 'static,
            Fut: Future<Output = ()> $( + $send)*,
        {
            let waker = RefCounter::new(Lock::new((false, None)));

            let finisher = FinisherFuture {
                waker: waker.clone(),
            };

            let f = Box::pin(f);

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

#[cfg(not(any(feature = "actix-runtime", feature = "actix-runtime-mpsc")))]
spawn_cancel!(Send);

#[cfg(any(feature = "actix-runtime", feature = "actix-runtime-mpsc"))]
spawn_cancel!();

// a future notified and polled by future_handler.
pub(crate) struct FinisherFuture {
    waker: RefCounter<Lock<(bool, Option<Waker>)>>,
}

impl Future for FinisherFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let mut guard = this.waker.lock();
        if guard.0 {
            Poll::Ready(())
        } else {
            guard.1 = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

pub struct FutureHandler<A>
where
    A: Actor,
{
    waker: RefCounter<Lock<(bool, Option<Waker>)>>,
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
        let mut guard = self.waker.lock();
        guard.0 = true;
        let opt = guard.1.take();

        drop(guard);

        if let Some(waker) = opt {
            waker.wake();
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
