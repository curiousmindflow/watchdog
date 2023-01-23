use std::{
    fmt::Debug,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use chrono::Utc;
use tokio::{sync::Notify, task::JoinHandle, time::timeout};
use tracing::{event, info_span, instrument, Instrument, Level};

use crate::error::Error;

#[derive(Debug, Clone)]
pub struct Watchdog {
    inner: Arc<Mutex<WatchdogInner>>,
}

impl Watchdog {
    #[instrument]
    pub fn new() -> Self {
        let inner = WatchdogInner {
            timer_jh: None,
            callback_exec_jh: None,
            refresh_notifier: Arc::new(Notify::new()),
            trigger_notifier: Arc::new(Notify::new()),
        };

        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    /// Register the callback that will be called if the watchdog trigger
    ///
    #[instrument(skip_all)]
    pub fn register_callback(&self, callback: impl FnMut(i64) -> () + Send + 'static) -> () {
        let mut inner_guard = self.lock_inner();
        let exec_jh = WatchdogInner::launch_callback_executor_task(
            inner_guard.trigger_notifier.clone(),
            Box::new(callback),
        );
        inner_guard.callback_exec_jh.replace(exec_jh);
    }

    /// Rearm the watchdog
    ///
    #[instrument(skip_all)]
    pub fn rearm(&self, delay: Duration) -> Result<(), Error> {
        let mut inner_guard = self.lock_inner();
        if !inner_guard.has_timer() {
            let jh = inner_guard.launch_timer(delay);
            let _old_jh = inner_guard.timer_jh.replace(jh);
            return Ok(());
        }
        return Err(Error::NoTimerStarted);
    }

    /// Disarm the watchdog
    ///
    #[instrument(skip_all)]
    pub fn disarm(&self) -> Result<(), Error> {
        let mut inner_guard = self.lock_inner();
        if inner_guard.has_timer() {
            let _jh = inner_guard.timer_jh.take();
            return Ok(());
        }
        return Err(Error::NoTimerStarted);
    }

    /// Refresh the watchdog, resetting it's internal counter
    ///
    #[instrument(skip_all)]
    pub fn refresh(&self) -> Result<(), Error> {
        let inner_guard = self.lock_inner();
        if inner_guard.has_timer() {
            inner_guard.refresh_notifier.notify_waiters();
            return Ok(());
        }
        return Err(Error::NoTimerStarted);
    }

    #[instrument(skip_all)]
    fn lock_inner(&self) -> MutexGuard<WatchdogInner> {
        self.inner.lock().expect("WatchdogInner mutex lock failed")
    }
}

#[derive()]
pub struct WatchdogInner {
    timer_jh: Option<JoinHandle<()>>,
    callback_exec_jh: Option<JoinHandle<()>>,
    refresh_notifier: Arc<Notify>,
    trigger_notifier: Arc<Notify>,
}

impl WatchdogInner {
    #[instrument(skip_all)]
    pub(crate) fn has_timer(&self) -> bool {
        self.timer_jh.is_some()
    }

    #[instrument(skip_all)]
    pub(crate) fn launch_timer(&self, delay: Duration) -> JoinHandle<()> {
        let refresh_notifier = self.refresh_notifier.clone();
        let trigger_notifier = self.trigger_notifier.clone();
        tokio::spawn(
            async move {
                loop {
                    if let Err(_) = timeout(delay, refresh_notifier.notified()).await {
                        trigger_notifier.notify_waiters();
                        break;
                    }
                }
            }
            .instrument(info_span!("timer")),
        )
    }

    #[instrument(skip_all)]
    pub(crate) fn launch_callback_executor_task(
        trigger_notifier: Arc<Notify>,
        mut callback: Box<dyn FnMut(i64) -> () + Send + 'static>,
    ) -> JoinHandle<()> {
        tokio::spawn(
            async move {
                loop {
                    trigger_notifier.notified().await;
                    let now = Utc::now().timestamp();
                    callback(now);
                    event!(Level::TRACE, "watchdog triggered callback")
                }
            }
            .instrument(info_span!("callback_executor_task")),
        )
    }
}

impl Debug for WatchdogInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchdogInner")
            .field("timer_jh", &self.timer_jh)
            .field("refresh_notifier", &self.refresh_notifier)
            .field("trigger_notifier", &self.trigger_notifier)
            .finish()
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

    #[derive(Default, Clone)]
    struct AssertStruct {
        pub trigger: Arc<Mutex<Option<i64>>>,
    }

    impl AssertStruct {
        pub fn trigger(&self, time: i64) -> () {
            self.trigger.lock().unwrap().replace(time);
        }

        pub fn is_triggered(&self) -> bool {
            self.trigger.lock().unwrap().is_some()
        }
    }

    #[tokio::test]
    async fn no_timeout_no_refresh() {
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(200)).unwrap();
        // wdg.rearm(Duration::from_secs(5)).unwrap();

        // sleep(Duration::from_secs(200)).await;
        assert!(!asrt.is_triggered());
    }

    #[tokio::test]
    async fn no_timeout_with_refresh() {
        let (asrt, wdg) = prepare();

        // arming the watchdog
        wdg.rearm(Duration::from_millis(200)).unwrap();

        // waiting 150 micros, then in 50 micros the watchdog should trigger
        sleep(Duration::from_millis(150)).await;

        // before it trigger, it's refreshed, so it's resetted to 200 micros
        wdg.refresh().unwrap();

        // waiting again 150 micro to prove that the initial 200 micros (timer started at the 'rearm' call) are passed
        sleep(Duration::from_millis(150)).await;

        // because we have refresh the watchdog before the due time, it didn't triggered
        assert!(!asrt.is_triggered());
    }

    #[tokio::test]
    async fn with_timeout_no_refresh() {
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(200)).unwrap();

        sleep(Duration::from_millis(250)).await;

        assert!(asrt.is_triggered());
    }

    #[tokio::test]
    async fn with_timeout_with_refresh() {
        let (asrt, wdg) = prepare();

        wdg.rearm(Duration::from_millis(200)).unwrap();

        sleep(Duration::from_millis(150)).await;

        assert!(!asrt.is_triggered());

        wdg.refresh().unwrap();

        sleep(Duration::from_millis(250)).await;

        assert!(asrt.is_triggered());
    }

    fn prepare() -> (AssertStruct, Watchdog) {
        let _asrt = AssertStruct::default();
        let wdg = Watchdog::new();

        let asrt = _asrt.clone();
        wdg.register_callback(move |now| {
            asrt.trigger(now);
        });
        (_asrt, wdg)
    }
}
