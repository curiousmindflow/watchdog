use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{
    sync::{
        mpsc::{channel, Receiver, Sender},
        Notify,
    },
    time::timeout,
};
use tracing::{event, instrument, span, Level};

#[derive(Debug, Clone)]
pub struct Watchdog {
    rearm_sender: Sender<Duration>,
    armed: Arc<AtomicBool>,
    notifier: Arc<Notify>,
}

impl Watchdog {
    pub fn new<T>(callback: T) -> Watchdog
    where
        T: FnMut() -> () + Send + 'static,
    {
        let (rearm_sender, rearm_receiver) = channel::<Duration>(1);
        let (timeout_sender, timeout_receiver) = channel::<()>(1);
        let armed = Arc::new(AtomicBool::new(false));
        let notifier = Arc::new(Notify::new());

        Watchdog::launch_core_task(
            rearm_receiver,
            armed.clone(),
            notifier.clone(),
            timeout_sender,
        );

        Watchdog::launch_closure_execution_task(callback, timeout_receiver);

        Watchdog {
            rearm_sender,
            armed,
            notifier,
        }
    }

    #[instrument]
    pub async fn rearm(&self, delay: Duration) -> () {
        self.rearm_sender.send(delay).await.unwrap();
        event!(Level::TRACE, "rearmed");
    }

    #[instrument]
    pub fn refresh(&self) {
        self.notifier.notify_waiters();
        event!(Level::TRACE, "refreshed");
    }

    #[instrument]
    pub async fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
        self.notifier.notify_waiters();
        event!(Level::TRACE, "disarmed");
    }

    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::SeqCst)
    }

    fn launch_core_task(
        mut rearm_receiver: Receiver<Duration>,
        armed: Arc<AtomicBool>,
        notifier: Arc<Notify>,
        timeout_sender: Sender<()>,
    ) {
        tokio::spawn(async move {
            let _span = span!(Level::TRACE, "wdg_core_task");
            let mut delay: Duration = Duration::from_millis(100);
            loop {
                if !armed.load(Ordering::SeqCst) {
                    event!(Level::TRACE, "arming...");
                    if let Some(new_delay) = rearm_receiver.recv().await {
                        delay = new_delay;
                        armed.store(true, Ordering::SeqCst);
                        event!(Level::TRACE, "armed");
                    } else {
                        break;
                    }
                }
                if let Err(_elapsed) = timeout(delay, notifier.notified()).await {
                    event!(Level::TRACE, "timeout occured");
                    if let Err(_send_error) = timeout_sender.send(()).await {
                        break;
                    }
                }
            }
            event!(Level::TRACE, "Exit");
        });
    }

    fn launch_closure_execution_task<T>(mut callback: T, mut receiver: Receiver<()>)
    where
        T: FnMut() -> () + Send + 'static,
    {
        tokio::spawn(async move {
            let _span = span!(Level::TRACE, "wdg_exec_task");
            while let Some(_) = receiver.recv().await {
                event!(Level::TRACE, "callback execution...");
                callback();
                event!(Level::TRACE, "callback executed");
            }
            event!(Level::TRACE, "Exit");
        });
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
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(100)).await;

        for _ in 0..5 {
            wdg.refresh();
        }

        drop(wdg);

        assert!(!asrt.lock().unwrap().is_triggered);
    }

    #[tokio::test]
    async fn with_timeout() {
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(100)).await;

        sleep(Duration::from_millis(110)).await;

        drop(wdg);

        assert!(asrt.lock().unwrap().is_triggered);
    }

    #[tokio::test]
    async fn without_timeout_parallel() {
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(100)).await;

        let wdg_clone = wdg.clone();
        tokio::spawn(async move {
            for _ in 0..5 {
                wdg_clone.refresh();
            }
        });

        drop(wdg);

        assert!(!asrt.lock().unwrap().is_triggered);
    }

    fn prepare() -> (Arc<Mutex<AssertStruct>>, Watchdog) {
        let asrt = Arc::new(Mutex::new(AssertStruct::default()));

        let asrt_clone = asrt.clone();
        let wdg = Watchdog::new(move || {
            let mut guard = asrt_clone.lock().unwrap();
            guard.is_triggered = true;
        });

        (asrt, wdg)
    }
}
