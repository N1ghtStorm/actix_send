use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::task::{Context, Poll, Waker};

use futures::future::Either;
use parking_lot::Mutex;

use crate::actor::Actor;
use crate::builder::WeakSender;
use crate::context::ChannelMessage;
use crate::util::runtime;

// helper function for spawn a future on runtime and return a handler that can cancel it.
pub(crate) fn spawn_cancelable<F, FN, A>(f: F, cancel_now: bool, on_cancel: FN) -> FutureHandler<A>
where
    A: Actor,
    F: Future + Unpin + Send + 'static,
    <F as Future>::Output: Send,
    FN: FnOnce() + Send + 'static,
{
    let waker = Arc::new(Mutex::new(None));
    let state = Arc::new(AtomicBool::new(true));

    let finisher = FinisherFuture {
        state: state.clone(),
        waker: waker.clone(),
    };

    runtime::spawn(async move {
        let either = futures::future::select(finisher, f).await;

        if let Either::Left((_, f)) = either {
            on_cancel();
            if !cancel_now {
                let _ = f.await;
            }
        }
    });

    FutureHandler {
        state,
        waker,
        tx: None,
    }
}

pub(crate) fn spawn_cancelable_new<F, A>(
    f: F,
) -> (FutureHandler<A>, futures::future::Select<FinisherFuture, F>)
where
    A: Actor,
    F: Future + Unpin + Send + 'static,
    <F as Future>::Output: Send,
{
    let waker = Arc::new(Mutex::new(None));
    let state = Arc::new(AtomicBool::new(true));

    let finisher = FinisherFuture {
        state: state.clone(),
        waker: waker.clone(),
    };

    let future = futures::future::select(finisher, f);
    let handler = FutureHandler {
        state,
        waker,
        tx: None,
    };

    (handler, future)
}

// a future notified and polled by future_handler.
pub(crate) struct FinisherFuture {
    state: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Future for FinisherFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if !this.state.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        let mut waker = this.waker.lock();
        *waker = Some(cx.waker().clone());

        Poll::Pending
    }
}

pub struct FutureHandler<A>
where
    A: Actor,
{
    state: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
    tx: Option<(usize, WeakSender<A>)>,
}

impl<A> Clone for FutureHandler<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
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
        self.state.store(false, Ordering::Release);
        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
        // We remove the interval future with index as key.
        if let Some((index, tx)) = self.tx.as_ref() {
            if let Some(tx) = tx.upgrade() {
                let index = *index;
                runtime::spawn(async move {
                    let _ = tx.send(ChannelMessage::IntervalFutureRemove(index)).await;
                });
            }
        }
    }

    pub(crate) fn attach_tx(&mut self, index: usize, tx: WeakSender<A>) {
        self.tx = Some((index, tx));
    }
}
