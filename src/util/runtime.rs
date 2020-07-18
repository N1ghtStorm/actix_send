use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

#[cfg(feature = "async-std-runtime")]
#[cfg(not(any(feature = "tokio-runtime", feature = "actix-runtime")))]
use futures_util::stream::StreamExt;

use crate::error::ActixSendError;

macro_rules! runtime_impl {
    (
        $spawn_fn: path,
        $delay_fn: path,
        $delay_ty: path,
        $interval_fn: path,
        $interval_ty: path,
        $timeout_fn: path,
        $tick_fn: ident
        $(, $send:ident)*
    ) => {
        pub(crate) fn spawn<Fut>(f: Fut)
        where
            Fut: Future + 'static $( + $send)*,
        {
            $spawn_fn(async move {
                let _ = f.await;
            });
        }

        pub(crate) fn delay_for(dur: Duration) -> $delay_ty {
            $delay_fn(dur)
        }

        pub(crate) fn interval(dur: Duration) -> $interval_ty {
            $interval_fn(dur)
        }

        pub(crate) async fn tick(interval: &mut $interval_ty) {
            let _ = interval.$tick_fn().await;
        }

        pub(crate) async fn timeout<Fut, R>(dur: Duration, fut: Fut) -> Result<R, ActixSendError>
        where
            Fut: Future<Output=R> $( + $send)*,
        {
            $timeout_fn(dur, fut).await.map_err(|_|ActixSendError::Timeout)
        }
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(any(feature = "async-std-runtime", feature = "actix-runtime")))]
runtime_impl!(
    tokio::spawn,
    tokio::time::delay_for,
    tokio::time::Delay,
    tokio::time::interval,
    tokio::time::Interval,
    tokio::time::timeout,
    tick,
    Send
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(any(feature = "tokio-runtime", feature = "actix-runtime")))]
runtime_impl!(
    async_std::task::spawn,
    smol::Timer::after,
    smol::Timer,
    async_std::stream::interval,
    async_std::stream::Interval,
    async_std::future::timeout,
    next,
    Send
);

#[cfg(feature = "actix-runtime")]
#[cfg(not(any(feature = "tokio-runtime", feature = "async-std-runtime")))]
runtime_impl!(
    actix_rt::spawn,
    actix_rt::time::delay_for,
    actix_rt::time::Delay,
    actix_rt::time::interval,
    actix_rt::time::Interval,
    actix_rt::time::timeout,
    tick
);

#[allow(unused_variables)]
pub async fn spawn_blocking<F, T>(f: F) -> Result<T, ActixSendError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    #[cfg(feature = "tokio-runtime")]
    #[cfg(not(any(feature = "async-std-runtime", feature = "actix-runtime")))]
    {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|_| ActixSendError::Blocking)
    }

    #[cfg(feature = "async-std-runtime")]
    #[cfg(not(any(feature = "tokio-runtime", feature = "actix-runtime")))]
    {
        Ok(async_std::task::spawn_blocking(f).await)
    }

    #[cfg(feature = "actix-runtime")]
    #[cfg(not(any(feature = "async-std-runtime", feature = "tokio-runtime")))]
    panic!("spawn_blocking does not work for actix-runtime.\r\nPlease use web::block directly in your handle method");
}

// from tokio::task::yield_now(). give control back to scheduler.
// We copy/paste this so we can use it on all runtime.
pub(crate) async fn yield_now() {
    struct YieldNow {
        yielded: bool,
    }

    impl Future for YieldNow {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.yielded {
                return Poll::Ready(());
            }

            self.yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    YieldNow { yielded: false }.await
}
