use actix_send::prelude::*;

// Actor type
#[actor]
pub struct MyActor;

// message types
pub struct Message1;
pub struct Message2;
pub struct Message3;

// handler implement
#[handler_v2]
impl MyActor {
    async fn handle_msg1(&mut self, _: Message1) {}

    // the name of handle method is not important at all.
    // It's only used to make IDE happy when actually we transfer them to Actor::handle
    // method.
    async fn ra_ri_ru_rei_ro(&mut self, _: Message2) -> u8 {
        8
    }

    fn handle_blocking(&self, _: Message3) -> u16 {
        // We can use non async method to notify the macro we are asking for a blocking handle
        // method.
        16
    }
}

#[tokio::main]
async fn main() {
    // define actor creation
    let my_actor = MyActor::create(|| MyActor);

    // build and start actor.
    let my_actor: Address<MyActor> = my_actor.build().start();

    /*
       send messages to actor.

       No matter how we name the handle message method for a give <MessageType> in impl MyActor
       we can just call Address::send(<MessageType>).
    */
    let res1 = my_actor.send(Message1).await;

    assert_eq!((), res1.unwrap());

    let res2 = my_actor.send(Message2).await;

    assert_eq!(8, res2.unwrap());

    let res3 = my_actor.send(Message3).await;
    assert_eq!(16, res3.unwrap());

    println!("example finished successfully");
}
