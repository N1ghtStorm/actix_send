// use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::Arc;

use async_channel::{unbounded, SendError, Sender};
use async_trait::async_trait;
use futures_channel::oneshot::{
    channel,
    Canceled,
    // Receiver,
    Sender as OneshotSender,
};
use tokio::sync::Mutex;

use crate::util::runtime;

// pub struct Context<A, M>
// where
//     A: Actor<M>,
//     M: Message,
// {
//     mail: MailBox<A, M>,
// }
//
// impl<A, M> Default for Context<A, M>
// where
//     A: Actor<M>,
//     M: Message,
// {
//     fn default() -> Self {
//         Self {
//             mail: MailBox {
//                 msg: AddressReceiver {
//                     inner: Arc::new(Inner {
//                         message_queue: Mutex::new(VecDeque::new()),
//                         _p: PhantomData,
//                     }),
//                 },
//             },
//         }
//     }
// }
//
// pub struct MailBox<A, M>
// where
//     A: Actor<M>,
//     M: Message,
// {
//     msg: AddressReceiver<A, M>,
// }
//
// pub struct AddressReceiver<A, M>
// where
//     A: Actor<M>,
//     M: Message,
// {
//     inner: Arc<Inner<A, M>>,
// }
//
// struct Inner<A, M>
// where
//     A: Actor<M>,
//     M: Message,
// {
//     message_queue: Mutex<VecDeque<M>>,
//     _p: PhantomData<A>,
// }

pub struct Builder<A, M>
where
    A: Actor<M>,
    M: Message,
{
    actor: A,
    num: usize,
    _p: PhantomData<M>,
}

impl<A, M> Builder<A, M>
where
    A: Actor<M> + Handler<M> + 'static,
    M: Message + Send + 'static,
    M::Result: Send,
{
    /// Build multiple actors with the num passed.
    ///
    /// All the actors share the same state and would steal work from a single async-channel(MPMC queue).
    pub fn num(mut self, num: usize) -> Self {
        assert!(num > 0, "The number of actors must be larger than 0");
        self.num = num;
        self
    }

    pub fn start(mut self) -> Address<M> {
        let (tx, rx) = unbounded::<(OneshotSender<M::Result>, M)>();

        let num = self.num;

        if num > 1 {
            let actor = Arc::new(Mutex::new(self.actor));

            for _ in 0..num {
                let actor = actor.clone();
                let rx = rx.clone();
                runtime::spawn(async move {
                    while let Ok((tx, msg)) = rx.recv().await {
                        let mut act = actor.lock().await;
                        let res = act.handle(msg).await;
                        let _ = tx.send(res);
                    }
                });
            }
        } else {
            runtime::spawn(async move {
                while let Ok((tx, msg)) = rx.recv().await {
                    let res = self.actor.handle(msg).await;
                    let _ = tx.send(res);
                }
            });
        }

        Address { tx }
    }
}

#[derive(Debug)]
pub enum ActixSendError<M>
where
    M: Message,
{
    Canceled,
    Closed(M),
}

impl<M> From<Canceled> for ActixSendError<M>
where
    M: Message,
{
    fn from(_err: Canceled) -> Self {
        ActixSendError::Canceled
    }
}

impl<M> From<SendError<(OneshotSender<M::Result>, M)>> for ActixSendError<M>
where
    M: Message,
{
    fn from(t: SendError<(OneshotSender<M::Result>, M)>) -> Self {
        ActixSendError::Closed((t.0).1)
    }
}

pub struct Address<M>
where
    M: Message,
{
    tx: Sender<(OneshotSender<M::Result>, M)>,
}

impl<M> Address<M>
where
    M: Message,
{
    /// Message will be returned in `ActixSendError::Closed(Message)` if the actor is already closed.
    pub async fn send(&self, msg: M) -> Result<M::Result, ActixSendError<M>> {
        let (tx, rx) = channel::<M::Result>();
        self.tx.send((tx, msg)).await?;

        Ok(rx.await?)
    }
}

pub trait Actor<M>
where
    M: Message,
    Self: Sized + Send,
{
    fn build(self) -> Builder<Self, M> {
        Builder {
            actor: self,
            num: 1,
            _p: PhantomData,
        }
    }
}

pub trait Message: Send {
    type Result;
}

#[async_trait]
pub trait Handler<M>
where
    M: Message,
    Self: Actor<M>,
{
    async fn handle(&mut self, msg: M) -> M::Result;
}
