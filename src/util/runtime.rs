use std::future::Future;
use std::time::Duration;

use crate::error::ActixSendError;

macro_rules! spawn {
    (
        $spawn_fn: path,
        $delay_fn: path,
        $interval_fn: path,
        $interval_ty: path,
        $tick_fn: ident
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
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
spawn!(
    tokio::spawn,
    tokio::time::delay_for,
    tokio::time::interval,
    tokio::time::Interval,
    tick
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
spawn!(
    async_std::task::spawn,
    async_std::task::sleep,
    async_std::stream::interval,
    async_std::stream::Interval,
    next
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
use futures::StreamExt;

pub async fn spawn_blocking<F, T>(f: F) -> Result<T, ActixSendError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    #[cfg(feature = "tokio-runtime")]
    #[cfg(not(feature = "async-std-runtime"))]
    {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|_| ActixSendError::Blocking)
    }

    #[cfg(feature = "async-std-runtime")]
    #[cfg(not(feature = "tokio-runtime"))]
    {
        Ok(async_std::task::spawn_blocking(f).await)
    }
}
