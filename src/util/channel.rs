pub(crate) use channel_inner::{
    bounded, oneshot_channel, unbounded, OneShotReceiver, OneShotSender, Receiver, Sender,
};

#[cfg(not(any(feature = "actix-runtime-mpsc")))]
pub(crate) mod channel_inner {
    pub(crate) use async_channel::{Receiver, Sender};
    pub(crate) use tokio::sync::oneshot::{
        channel as oneshot_channel, Receiver as OneShotReceiver, Sender as OneShotSender,
    };

    pub(crate) fn bounded<A>(cap: usize) -> (Sender<A>, Receiver<A>) {
        async_channel::bounded(cap)
    }

    pub(crate) fn unbounded<A>() -> (Sender<A>, Receiver<A>) {
        async_channel::unbounded()
    }
}

#[cfg(feature = "actix-runtime-mpsc")]
pub(crate) mod channel_inner {
    pub(crate) use tokio::sync::{
        mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender},
        oneshot::{
            channel as oneshot_channel, Receiver as OneShotReceiver, Sender as OneShotSender,
        },
    };

    pub(crate) fn bounded<A>(_cap: usize) -> (Sender<A>, Receiver<A>) {
        tokio::sync::mpsc::unbounded_channel()
    }

    pub(crate) fn unbounded<A>() -> (Sender<A>, Receiver<A>) {
        tokio::sync::mpsc::unbounded_channel()
    }
}
