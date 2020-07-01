use std::future::Future;
use std::time::Duration;

use crate::error::ActixSendError;

macro_rules! spawn {
    (
        $spawn_fn: path,
        $delay_fn: path,
        $interval_fn: path,
        $interval_ty: path,
        $tick_fn: ident,
        $spawn_blocking_fn: path
    ) => {
        pub(crate) fn spawn<Fut>(f: Fut)
        where
            Fut: Future + Send + 'static,
            Fut::Output: Send + 'static,
        {
            $spawn_fn(f);
        }

        pub(crate) async fn delay_for(dur: Duration) {
            let _ = $delay_fn(dur).await;
        }

        pub(crate) fn interval(dur: Duration) -> $interval_ty {
            $interval_fn(dur)
        }

        pub(crate) async fn tick(interval: &mut $interval_ty) {
            let _ = interval.$tick_fn().await;
        }

        pub async fn spawn_blocking<F, T>(f: F) -> Result<T, ActixSendError>
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            $spawn_blocking_fn(f)
                .await
                .map_err(|_| ActixSendError::Blocking)
        }
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
spawn!(
    tokio::spawn,
    tokio::time::delay_for,
    tokio::time::interval,
    tokio::time::Interval,
    tick,
    tokio::task::spawn_blocking
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
spawn!(
    async_std::task::spawn,
    async_std::task::sleep,
    async_std::stream::interval,
    async_std::stream::Interval,
    next,
    async_std::task::spawn_blocking
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
use futures::StreamExt;
