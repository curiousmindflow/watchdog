use std::time::Duration;

use tokio::{
    sync::mpsc::{channel, Sender},
    task::JoinHandle,
    time::timeout,
};

#[derive(Debug)]
pub struct Watchdog {
    handle: Option<JoinHandle<()>>,
    sender: Option<Sender<()>>,
}

impl Watchdog {
    pub fn new() -> Watchdog {
        let handle = None;
        let sender = None;
        Self { handle, sender }
    }

    async fn rearm<T>(&mut self, delay: Duration, mut closure: T) -> ()
    where
        T: FnMut() -> () + Send + 'static,
    {
        let (tx, mut rx) = channel::<()>(1);

        let handle = tokio::spawn(async move {
            loop {
                if let Err(_) = timeout(delay, rx.recv()).await {
                    closure();
                }
            }
        });

        let old_handle = self.handle.replace(handle);
        self.sender.replace(tx);

        if let Some(old_h) = old_handle {
            old_h.abort();
        }
    }

    async fn refresh(&self) {
        if let Some(ref tx) = self.sender {
            tx.send(()).await.unwrap();
        }
    }
}

#[cfg(test)]
mod watchdog_tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::time::sleep;

    use super::Watchdog;

    #[derive(Default)]
    struct AssertStruct {
        pub is_triggered: bool,
    }

    impl AssertStruct {
        pub fn trigger(&mut self) -> () {
            self.is_triggered = true;
        }
    }

    #[tokio::test]
    async fn without_timeout() {
        let (asrt, mut wdg) = prepare();

        let asrt_c = asrt.clone();
        wdg.rearm(Duration::from_millis(100), move || {
            asrt_c.lock().unwrap().trigger()
        })
        .await;

        for _ in 0..5 {
            wdg.refresh().await;
        }

        drop(wdg);

        assert!(!asrt.lock().unwrap().is_triggered);
    }

    #[tokio::test]
    async fn with_timeout() {
        let (asrt, mut wdg) = prepare();

        let asrt_c = asrt.clone();
        wdg.rearm(Duration::from_millis(100), move || {
            asrt_c.lock().unwrap().trigger()
        })
        .await;

        sleep(Duration::from_millis(110)).await;

        drop(wdg);

        assert!(asrt.lock().unwrap().is_triggered);
    }

    fn prepare() -> (Arc<Mutex<AssertStruct>>, Watchdog) {
        let asrt = Arc::new(Mutex::new(AssertStruct::default()));
        let wdg = Watchdog::new();
        (asrt, wdg)
    }
}
