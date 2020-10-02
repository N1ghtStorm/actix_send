#[cfg(feature = "actix-runtime-mpsc")]
#[actix_rt::main]
async fn main() {
    use actix_send::prelude::*;

    // Actor type
    #[actor(no_send)]
    pub struct MyActor {
        state: std::rc::Rc<usize>,
    }

    // message types
    pub struct Message1;
    pub struct Message2;

    // handler implement
    #[handler_v2(no_send)]
    impl MyActor {
        async fn handle_msg1(&mut self, _msg1: Message1) {}

        // the name of handle method is not important at all.
        // It's only used to make IDE happy when actually we transfer them to Actor::handle
        // method.
        async fn ra_ri_ru_rei_ro(&mut self, _: Message2) -> u8 {
            8
        }
    }

    // define actor creation
    let builder = MyActor::builder(|| async {
        MyActor {
            state: std::rc::Rc::new(123),
        }
    });

    let arbiter = actix::Arbiter::new();

    // build and start actor.
    let address: Address<MyActor> = builder.start_with_arbiters(&[arbiter], None).await;

    /*
       send messages to actor.

       No matter how we name the handle message method for a give <MessageType> in impl MyActor
       we can just call Address::send(<MessageType>).
    */
    let res1 = address.send(Message1).await;

    assert_eq!((), res1.unwrap());

    let res2 = address.send(Message2).await;

    assert_eq!(8, res2.unwrap());

    println!("example finished successfully");
}

#[cfg(not(feature = "actix-runtime-mpsc"))]
fn main() {}
