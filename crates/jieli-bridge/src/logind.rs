use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use zbus::Proxy;

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_INTERFACE: &str = "org.freedesktop.login1.Manager";
const INHIBIT_WHAT: &str = "sleep";
const INHIBIT_WHO: &str = "Jieli camera bridge";
const INHIBIT_WHY: &str = "Close webcam before sleep";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SleepEvent {
    Inhibited,
    Prepare,
    Resume,
    Deadline,
    Error(String),
}

struct Shared {
    fd: Mutex<Option<OwnedFd>>,
    generation: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Self {
            fd: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    fn take_fd(&self) -> Option<OwnedFd> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.fd.lock().ok().and_then(|mut slot| slot.take())
    }
}

/// Owns the logind delay inhibitor on a dedicated thread and forwards events
/// to the supervisor loop. A missing inhibitor keeps the real capture closed.
pub struct SleepMonitor {
    events: Receiver<SleepEvent>,
    shared: Arc<Shared>,
    error: Option<String>,
    ended: bool,
}

impl SleepMonitor {
    pub fn start() -> Self {
        let (sender, events) = mpsc::channel();
        let shared = Arc::new(Shared::new());
        let thread_shared = Arc::clone(&shared);
        std::thread::spawn(move || run(sender, thread_shared));
        Self {
            events,
            shared,
            error: None,
            ended: false,
        }
    }

    /// Wait for a logind event or the supervisor tick timeout.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<SleepEvent> {
        if self.ended {
            std::thread::sleep(timeout);
            return None;
        }
        match self.events.recv_timeout(timeout) {
            Ok(event) => {
                match &event {
                    SleepEvent::Inhibited => self.error = None,
                    SleepEvent::Error(error) => self.error = Some(error.clone()),
                    _ => {}
                }
                Some(event)
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.ended = true;
                let error = "logind sleep monitor stopped".to_string();
                self.error = Some(error.clone());
                Some(SleepEvent::Error(error))
            }
        }
    }

    pub fn inhibited(&self) -> bool {
        self.shared
            .fd
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false)
    }

    /// Close the delay FD, allowing logind to proceed with suspend.
    pub fn release(&self) {
        if let Some(fd) = self.shared.take_fd() {
            drop(fd);
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Drop for SleepMonitor {
    fn drop(&mut self) {
        self.release();
    }
}

fn run(sender: Sender<SleepEvent>, shared: Arc<Shared>) {
    let result = monitor(sender.clone(), Arc::clone(&shared));
    if let Err(error) = result {
        shared.take_fd();
        let _ = sender.send(SleepEvent::Error(error));
    }
}

fn monitor(sender: Sender<SleepEvent>, shared: Arc<Shared>) -> Result<(), String> {
    let connection = zbus::block_on(zbus::Connection::system())
        .map_err(|error| format!("logind unavailable: {error}"))?;
    let proxy = zbus::block_on(Proxy::new(
        &connection,
        LOGIND_SERVICE,
        LOGIND_PATH,
        LOGIND_INTERFACE,
    ))
    .map_err(|error| format!("logind unavailable: {error}"))?;

    let limit_usec: u64 = zbus::block_on(proxy.get_property("InhibitDelayMaxUSec"))
        .map_err(|error| format!("logind did not provide InhibitDelayMaxUSec: {error}"))?;
    if limit_usec == 0 {
        return Err("logind did not provide InhibitDelayMaxUSec".to_string());
    }
    let limit = limit_usec as f64 / 1_000_000.0;

    acquire(&proxy, &shared, &sender)?;
    let mut signals = zbus::block_on(proxy.receive_signal("PrepareForSleep"))
        .map_err(|error| format!("logind signal unavailable: {error}"))?;

    loop {
        let Some(message) = zbus::block_on(signals.next()) else {
            return Err("logind PrepareForSleep stream ended".to_string());
        };
        let start: bool = message
            .body()
            .deserialize_ignore_signature()
            .map_err(|error| format!("invalid PrepareForSleep payload: {error}"))?;

        if start {
            let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = sender.send(SleepEvent::Prepare);
            spawn_deadline(Arc::clone(&shared), sender.clone(), limit, generation);
        } else {
            if let Some(fd) = shared.take_fd() {
                drop(fd);
            }
            let _ = sender.send(SleepEvent::Resume);
            acquire(&proxy, &shared, &sender)?;
        }
    }
}

fn acquire(proxy: &Proxy<'_>, shared: &Shared, sender: &Sender<SleepEvent>) -> Result<(), String> {
    let reply = zbus::block_on(proxy.call_method(
        "Inhibit",
        &(INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY, "delay"),
    ))
    .map_err(|error| format!("sleep delay inhibitor unavailable: {error}"))?;
    let fd: zbus::OwnedFd = reply
        .body()
        .deserialize()
        .map_err(|error| format!("sleep delay inhibitor FD unavailable: {error}"))?;
    let mut slot = shared
        .fd
        .lock()
        .map_err(|_| "sleep inhibitor slot poisoned".to_string())?;
    *slot = Some(fd.into());
    let _ = sender.send(SleepEvent::Inhibited);
    Ok(())
}

fn spawn_deadline(shared: Arc<Shared>, sender: Sender<SleepEvent>, limit: f64, generation: u64) {
    // Keep the live helper's safety margin: release before logind's deadline
    // even if capture teardown hangs.
    let budget = (limit - limit.min(0.1).min(limit * 0.1)).max(0.001);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs_f64(budget));
        if shared.generation.load(Ordering::SeqCst) == generation {
            let released = shared.fd.lock().ok().and_then(|mut slot| slot.take());
            if released.is_some() {
                let _ = sender.send(SleepEvent::Deadline);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    #[ignore = "requires the system logind and system D-Bus"]
    fn inhibitor_is_acquired_and_released() {
        let mut monitor = SleepMonitor::start();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut inhibited = false;
        while Instant::now() < deadline {
            if let Some(event) = monitor.recv_timeout(Duration::from_millis(200)) {
                match event {
                    SleepEvent::Inhibited => {
                        inhibited = true;
                        break;
                    }
                    SleepEvent::Error(error) => panic!("logind error: {error}"),
                    _ => {}
                }
            }
            if monitor.inhibited() {
                inhibited = true;
                break;
            }
        }
        assert!(inhibited, "logind did not grant a delay inhibitor");
        monitor.release();
        assert!(!monitor.inhibited());
    }
}
