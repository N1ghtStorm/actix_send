use actix_send::prelude::*;

// Actor types
#[actor(no_static)]
pub struct MasterActor;

#[actor]
pub struct SlaveActor1;

#[actor]
pub struct SlaveActor2;

// message type
// message have to be able to clone it self as we are sending to potential multiple addresses.
#[derive(Clone)]
pub struct Message;

#[derive(Clone)]
pub struct Message2;

#[handler_v2]
impl SlaveActor1 {
    async fn handle_msg(&mut self, _msg1: Message) {
        println!("SlaveActor1: we received Message from our master actor");
    }

    async fn handle_msg2(&mut self, _msg1: Message2) {
        println!("SlaveActor1: we received Message2 from our master actor");
    }
}

// handler implement
#[handler_v2]
impl SlaveActor2 {
    async fn handle_msg(&mut self, _msg1: Message) {
        println!("SlaveActor2: we received Message from our master actor");
    }

    async fn handle_msg2(&mut self, _msg1: Message2) {
        println!("SlaveActor2: we received Message2 from our master actor");
    }
}

#[tokio::main]
async fn main() {
    let address_master: Address<MasterActor> = MasterActor::builder(|| async { MasterActor })
        // build our actor with allow_subscribe flag
        .allow_subscribe()
        .start()
        .await;

    let address_slave1 = SlaveActor1::builder(|| async { SlaveActor1 }).start().await;

    let address_slave2 = SlaveActor2::builder(|| async { SlaveActor2 }).start().await;

    // we subscribe our slave addresses to master.

    // We need to infer the message type we want to subscribe in the type signature.
    address_master
        .subscribe_with::<_, Message>(&address_slave1)
        .await;
    address_master
        .subscribe_with::<_, Message>(&address_slave2)
        .await;

    // We can infer different type for a given address.
    address_master
        .subscribe_with::<_, Message2>(&address_slave1)
        .await;
    address_master
        .subscribe_with::<_, Message2>(&address_slave2)
        .await;

    // We send a message to the subscribers.
    let res = address_master.send_subscribe(Message).await;

    /*
       result is a vector of message result regardless certain actor successfully or failed to
       handle the message.

       *. Limitation: The Ok part of the result can't return for now.
    */
    res.into_iter().for_each(|r| r.unwrap());

    address_master
        .send_subscribe(Message2)
        .await
        .into_iter()
        .for_each(|r| r.unwrap());
}
