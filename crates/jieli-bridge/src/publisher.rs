use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use jieli_core::frame::Frame;

struct PublisherProcess {
    child: Child,
    writer: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

/// Owns the publisher child and the depth-1 frame channel to its stdin.
pub struct PublisherManager {
    process: Option<PublisherProcess>,
    tx: Arc<Mutex<Option<SyncSender<Frame>>>>,
}

impl PublisherManager {
    pub fn new(tx: Arc<Mutex<Option<SyncSender<Frame>>>>) -> Self {
        Self { process: None, tx }
    }

    pub fn start(&mut self, node_name: &str, description: &str) -> Result<(), String> {
        if self.process.is_some() {
            return Ok(());
        }
        let executable = publisher_path()?;
        let mut child = Command::new(&executable)
            .arg("--node-name")
            .arg(node_name)
            .arg("--description")
            .arg(description)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("publisher unavailable: {error}"))?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("publisher stdin unavailable".to_string());
        };

        let (sender, receiver) = sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let writer = spawn_writer(stdin, receiver, stop.clone());
        *self
            .tx
            .lock()
            .map_err(|_| "publisher frame slot poisoned".to_string())? = Some(sender);
        self.process = Some(PublisherProcess {
            child,
            writer: Some(writer),
            stop,
        });
        Ok(())
    }

    /// Reap a publisher that exited on its own. Returns true when it was reaped.
    pub fn poll_exit(&mut self) -> bool {
        let exited = match self.process.as_mut() {
            Some(process) => match process.child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("Publisher exited: {status}");
                    true
                }
                Ok(None) => false,
                Err(error) => {
                    eprintln!("publisher poll failed: {error}");
                    false
                }
            },
            None => false,
        };
        if exited {
            self.cleanup(false);
        }
        exited
    }

    pub fn stop(&mut self) {
        self.cleanup(true);
    }

    fn cleanup(&mut self, terminate: bool) {
        let Some(mut process) = self.process.take() else {
            return;
        };
        if let Ok(mut slot) = self.tx.lock() {
            *slot = None;
        }
        process.stop.store(true, Ordering::Relaxed);

        if terminate {
            unsafe {
                libc::kill(process.child.id() as libc::pid_t, libc::SIGTERM);
            }
            for _ in 0..30 {
                match process.child.try_wait() {
                    Ok(Some(_)) => break,
                    _ => thread::sleep(Duration::from_millis(100)),
                }
            }
            let _ = process.child.kill();
            let _ = process.child.wait();
        }

        if let Some(writer) = process.writer.take() {
            let _ = writer.join();
        }
    }
}

impl Drop for PublisherManager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn publisher_path() -> Result<std::path::PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("cannot locate bridge binary: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "bridge binary has no parent directory".to_string())?;
    let publisher = directory.join("jieli-publisher");
    if !publisher.is_file() {
        return Err(format!(
            "publisher binary not found next to bridge: {}",
            publisher.display()
        ));
    }
    Ok(publisher)
}

fn spawn_writer(
    mut stdin: ChildStdin,
    receiver: Receiver<Frame>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(frame) => {
                    if stdin.write_all(frame.as_bytes()).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}
