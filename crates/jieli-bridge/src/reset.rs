use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use jieli_core::ResetResult;
use jieli_core::state::ResetError;

pub const SUDO: &str = "/usr/bin/sudo";
pub const HELPER: &str = "/usr/local/libexec/jieli-usb-reset";
pub const TIMEOUT: Duration = Duration::from_secs(12);

/// Match the live supervisor's classification of helper output.
pub fn classify(returncode: i32, stdout: &str, stderr: &str) -> Result<ResetResult, ResetError> {
    let text: String = format!("{stderr}{stdout}")
        .trim()
        .chars()
        .take(1000)
        .collect();
    if returncode != 0
        && (text.contains("sudo:") || text.to_ascii_lowercase().contains("permission denied"))
    {
        return Err(ResetError::Unavailable(format!(
            "Reset helper not authorized: {text}"
        )));
    }
    if returncode != 0 && text.contains("found 0") {
        return Err(ResetError::DeviceMissing(text));
    }
    Ok(ResetResult {
        returncode,
        message: text,
    })
}

/// Run the narrowly scoped helper through non-interactive sudo.
///
/// This blocks for up to 12 seconds and is meant to run on a worker thread.
pub fn run() -> Result<ResetResult, ResetError> {
    if !Path::new(HELPER).is_file() {
        return Err(ResetError::Unavailable(format!(
            "Reset helper missing: {HELPER}"
        )));
    }

    let child = Command::new(SUDO)
        .arg("-n")
        .arg(HELPER)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ResetError::Unavailable(format!("sudo or reset helper missing: {error}"))
        })?;
    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = sender.send(output);
    });

    match receiver.recv_timeout(TIMEOUT) {
        Ok(Ok(output)) => {
            let _ = worker.join();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            classify(
                output.status.code().unwrap_or(-1),
                stdout.as_ref(),
                stderr.as_ref(),
            )
        }
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(ResetError::Failed(format!(
                "reset helper could not be read: {error}"
            )))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // The helper holds its own lock and may still be in the ioctl.
            // Kill the sudo process; the state keeps the reset barrier until
            // a later successful reset clears it.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            let _ = worker.join();
            Err(ResetError::Failed(
                "reset helper exceeded twelve seconds".to_string(),
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = worker.join();
            Err(ResetError::Failed(
                "reset helper worker disappeared".to_string(),
            ))
        }
    }
}

/// Non-blocking handle used by the supervisor loop.
#[derive(Default)]
pub struct ResetManager {
    receiver: Option<mpsc::Receiver<Result<ResetResult, ResetError>>>,
    started: Option<Instant>,
}

impl ResetManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin(&mut self) {
        if self.receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(run());
        });
        self.receiver = Some(receiver);
        self.started = Some(Instant::now());
    }

    pub fn is_running(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.started.map(|start| start.elapsed())
    }

    pub fn poll(&mut self) -> Option<Result<ResetResult, ResetError>> {
        let receiver = self.receiver.as_ref()?;
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver = None;
                self.started = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                self.started = None;
                Some(Err(ResetError::Failed(
                    "reset helper worker disappeared".to_string(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_keeps_the_helper_message() {
        assert_eq!(
            classify(0, "reenumerated\n", ""),
            Ok(ResetResult {
                returncode: 0,
                message: "reenumerated".to_string()
            })
        );
    }

    #[test]
    fn sudo_authorization_failure_is_unavailable() {
        match classify(1, "", "sudo: a password is required\n") {
            Err(ResetError::Unavailable(message)) => {
                assert!(message.contains("not authorized"));
                assert!(message.contains("sudo:"));
            }
            other => panic!("unexpected {other:?}"),
        }
        match classify(1, "Permission denied", "") {
            Err(ResetError::Unavailable(_)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn absent_device_is_distinct_from_a_failed_reset() {
        assert_eq!(
            classify(
                1,
                "",
                "Expected exactly one Jieli 1224:2a25; found 0\n"
            ),
            Err(ResetError::DeviceMissing(
                "Expected exactly one Jieli 1224:2a25; found 0".to_string()
            ))
        );
    }

    #[test]
    fn ordinary_nonzero_status_is_returned_to_the_state_machine() {
        assert_eq!(
            classify(19, "", "reset failed\n"),
            Ok(ResetResult {
                returncode: 19,
                message: "reset failed".to_string()
            })
        );
    }
}
