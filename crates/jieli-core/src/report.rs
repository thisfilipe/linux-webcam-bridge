use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The 22 keys written by the live supervisor, in its insertion order.
pub const REPORT_KEYS: [&str; 22] = [
    "pid",
    "node_name",
    "capture_active",
    "capture_closing",
    "capture_starting",
    "publisher_active",
    "capture_sessions",
    "publisher_sessions",
    "frames",
    "fps_10s",
    "seconds_since_frame",
    "sleeping",
    "recovery",
    "retries",
    "retry_in_seconds",
    "reset_pending",
    "reset_barrier",
    "last_reset",
    "sleep_inhibited",
    "sleep_monitor_error",
    "placeholder",
    "error",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Recovery {
    Starting,
    Opening,
    WaitingFrames,
    Backoff,
    Sleeping,
    Resetting,
    Settling,
    Exhausted,
    HelperUnavailable,
    Streaming,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetResult {
    pub returncode: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeReport {
    pub pid: u32,
    pub node_name: String,
    pub capture_active: bool,
    pub capture_closing: bool,
    pub capture_starting: bool,
    pub publisher_active: bool,
    pub capture_sessions: u64,
    pub publisher_sessions: u64,
    pub frames: u64,
    pub fps_10s: f64,
    pub seconds_since_frame: Option<f64>,
    pub sleeping: bool,
    pub recovery: Recovery,
    pub retries: u32,
    pub retry_in_seconds: f64,
    pub reset_pending: bool,
    pub reset_barrier: bool,
    pub last_reset: Option<ResetResult>,
    pub sleep_inhibited: bool,
    pub sleep_monitor_error: Option<String>,
    pub placeholder: bool,
    pub error: Option<String>,
}

impl BridgeReport {
    pub fn new(pid: u32, node_name: impl Into<String>) -> Self {
        Self {
            pid,
            node_name: node_name.into(),
            capture_active: false,
            capture_closing: false,
            capture_starting: false,
            publisher_active: false,
            capture_sessions: 0,
            publisher_sessions: 0,
            frames: 0,
            fps_10s: 0.0,
            seconds_since_frame: None,
            sleeping: false,
            recovery: Recovery::Starting,
            retries: 0,
            retry_in_seconds: 0.0,
            reset_pending: false,
            reset_barrier: false,
            last_reset: None,
            sleep_inhibited: false,
            sleep_monitor_error: None,
            placeholder: true,
            error: None,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("BridgeReport serialization cannot fail")
    }

    /// Match the live `json.dumps(status) + "\n"` shape without fsync.
    ///
    /// The reference implementation uses a `.tmp` suffix; `with_extension` has the same
    /// behavior for the state filename. Rename makes the replacement atomic.
    pub fn write_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("tmp");
        let mut file = fs::File::create(&temporary)?;
        file.write_all(self.to_json().as_bytes())?;
        file.write_all(b"\n")?;
        drop(file);
        fs::rename(&temporary, path)
    }
}

pub fn remove_state_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn sample() -> BridgeReport {
        let mut report = BridgeReport::new(4242, "jieli_720p30");
        report.capture_active = true;
        report.capture_sessions = 2;
        report.publisher_active = true;
        report.publisher_sessions = 1;
        report.frames = 1234;
        report.fps_10s = 29.75;
        report.seconds_since_frame = Some(0.033);
        report.recovery = Recovery::Streaming;
        report.last_reset = Some(ResetResult {
            returncode: 0,
            message: "reset ok".to_string(),
        });
        report.sleep_inhibited = true;
        report.placeholder = false;
        report
    }

    #[test]
    fn writes_all_keys_in_live_order() {
        let json = sample().to_json();
        let mut cursor = 0;
        for key in REPORT_KEYS {
            let needle = format!("\"{key}\":");
            let found = json[cursor..]
                .find(&needle)
                .unwrap_or_else(|| panic!("missing key {key} after byte {cursor}: {json}"));
            cursor += found + needle.len();
        }
        // The nested last_reset object contributes two extra keys.
        assert_eq!(json.matches("\":").count(), REPORT_KEYS.len() + 2);
    }

    #[test]
    fn serializes_recovery_names() {
        assert_eq!(
            serde_json::to_string(&Recovery::WaitingFrames).unwrap(),
            "\"waiting-frames\""
        );
        assert_eq!(
            serde_json::to_string(&Recovery::HelperUnavailable).unwrap(),
            "\"helper-unavailable\""
        );
    }

    #[test]
    fn atomic_write_replaces_and_cleans_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("jieli-camera-bridge.json");
        let mut report = sample();

        report.write_atomic(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(!path.with_extension("tmp").exists());
        let parsed: BridgeReport = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, report);

        report.frames = 1235;
        report.write_atomic(&path).unwrap();
        let parsed: BridgeReport =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.frames, 1235);
        assert_eq!(parsed.node_name, "jieli_720p30");
    }

    #[test]
    fn remove_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        sample().write_atomic(&path).unwrap();
        remove_state_file(&path).unwrap();
        assert!(!path.exists());
        remove_state_file(&path).unwrap();
    }
}
