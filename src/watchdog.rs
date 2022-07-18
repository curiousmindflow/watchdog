use std::{future::Future, time::Duration};

use tokio::{
    sync::mpsc::channel,
    time::{sleep, timeout, Sleep, Timeout},
};

#[derive()]
pub struct Watchdog {
    // timeout: Timeout<Sleep>,
}

impl Watchdog {
    pub async fn new() -> Self {
        // let something = timeout(
        //     Duration::from_millis(500),
        //     Watchdog::dummy_delay_future().await,
        // );
        // let a = something.await.unwrap();
        Self {}
    }

    // async fn dummy_delay_future() -> Sleep {
    //     sleep(Duration::from_millis(250))
    // }

    async fn rearm(&self) -> () {
        let (sender, receiver) = channel::<()>(1);
    }
}

// impl Future for Watchdog {
//     type Output = ();

//     fn poll(
//         self: std::pin::Pin<&mut Self>,
//         cx: &mut std::task::Context<'_>,
//     ) -> std::task::Poll<Self::Output> {
//         todo!()
//     }
// }
