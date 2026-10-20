use crate::frame::{Frame, LatestSlot};
use crate::report::{BridgeReport, Recovery, ResetResult};

pub const MAX_RESETS: u32 = 3;
pub const FRAME_WATCHDOG_SECS: f64 = 12.0;
pub const PLACEHOLDER_GRACE_SECS: f64 = 0.25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub node_name: String,
    pub test_source: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            node_name: "jieli_720p30".to_string(),
            test_source: false,
        }
    }
}

impl BridgeConfig {
    pub fn description(&self) -> &'static str {
        if self.test_source {
            "Jieli validation"
        } else {
            "Jieli 720p30"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetError {
    Unavailable(String),
    Failed(String),
    /// The helper found no Jieli device on the bus. This is a re-enumeration
    /// condition, not a failed reset; keep recovery open instead of exhausting
    /// the bounded reset budget.
    DeviceMissing(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    StartCapture { generation: u64, identity: String },
    StopCapture,
    StartPublisher,
    StopPublisher,
    BeginReset,
    ReleaseSleepInhibitor,
}

#[derive(Debug)]
pub enum FrameOutcome {
    Ignored,
    Accepted { displaced: Option<Frame> },
}

/// Pure supervisor model. It owns no GStreamer, PipeWire, D-Bus or OS objects;
/// callers execute the returned actions and feed results back as events.
pub struct BridgeState {
    config: BridgeConfig,

    capture_active: bool,
    capture_closing: bool,
    capture_starting: bool,
    close_failed: bool,
    capture_generation: u64,
    capture_sessions: u64,

    publisher_active: bool,
    publisher_starting: bool,
    publisher_sessions: u64,
    publisher_retry_at: f64,

    identity: Option<String>,
    current_identity: Option<String>,
    seen: Option<String>,
    blocked: Option<String>,

    started: f64,
    last_frame: Option<f64>,
    frames: u64,
    report_frames: u64,
    last_report: f64,
    fps: f64,

    sleeping: bool,
    sleep_inhibited: bool,
    sleep_monitor_error: Option<String>,

    recovery: Recovery,
    recovery_generation: u64,
    retries: u32,
    retry_at: f64,
    reset_pending: bool,
    reset_barrier: bool,
    reset_required: bool,
    reset_token: u64,
    settle_after: f64,
    settle_deadline: f64,
    settle_seen: Option<String>,
    last_reset: Option<ResetResult>,

    stopping: bool,
    error: Option<String>,
    latest: LatestSlot<Frame>,
}

impl BridgeState {
    pub fn new(config: BridgeConfig, _now: f64) -> Self {
        Self {
            config,
            capture_active: false,
            capture_closing: false,
            capture_starting: false,
            close_failed: false,
            capture_generation: 0,
            capture_sessions: 0,
            publisher_active: false,
            publisher_starting: false,
            publisher_sessions: 0,
            publisher_retry_at: 0.0,
            identity: None,
            current_identity: None,
            seen: None,
            blocked: None,
            started: 0.0,
            last_frame: None,
            frames: 0,
            report_frames: 0,
            last_report: 0.0,
            fps: 0.0,
            sleeping: false,
            sleep_inhibited: false,
            sleep_monitor_error: None,
            recovery: Recovery::Starting,
            recovery_generation: 0,
            retries: 0,
            retry_at: 0.0,
            reset_pending: false,
            reset_barrier: false,
            reset_required: false,
            reset_token: 0,
            settle_after: 0.0,
            settle_deadline: 0.0,
            settle_seen: None,
            last_reset: None,
            stopping: false,
            error: None,
            latest: LatestSlot::new(),
        }
    }

    pub fn config(&self) -> &BridgeConfig {
        &self.config
    }

    pub fn description(&self) -> &'static str {
        self.config.description()
    }

    pub fn recovery(&self) -> Recovery {
        self.recovery
    }

    pub fn retries(&self) -> u32 {
        self.retries
    }

    pub fn capture_active(&self) -> bool {
        self.capture_active
    }

    pub fn publisher_active(&self) -> bool {
        self.publisher_active
    }

    pub fn sleeping(&self) -> bool {
        self.sleeping
    }

    pub fn reset_pending(&self) -> bool {
        self.reset_pending
    }

    pub fn reset_barrier(&self) -> bool {
        self.reset_barrier
    }

    pub fn reset_required(&self) -> bool {
        self.reset_required
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_sleep_monitor_error(&mut self, error: Option<String>) {
        self.sleep_monitor_error = error;
    }

    pub fn take_latest(&mut self) -> Option<Frame> {
        self.latest.take()
    }

    pub fn stopping(&self) -> bool {
        self.stopping
    }

    /// One supervisor tick. `now` is monotonic seconds; `identity` is the
    /// current device identity; `inhibited` is true when a sleep lease exists
    /// (or the test source has no lease requirement).
    pub fn tick(&mut self, now: f64, identity: Option<&str>, inhibited: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.stopping {
            return actions;
        }
        self.current_identity = identity.map(str::to_owned);
        self.sleep_inhibited = inhibited;

        if !self.publisher_active && !self.publisher_starting && now >= self.publisher_retry_at {
            self.publisher_starting = true;
            actions.push(Action::StartPublisher);
        }

        if self.capture_active && self.identity.as_deref() != identity {
            actions.extend(self.recover(
                now,
                "Camera disconnected; waiting for USB recovery",
                true,
            ));
        }

        if let Some(blocked) = self.blocked.clone()
            && let Some(current) = identity
            && current != blocked
        {
            self.blocked = None;
            self.retries = 0;
            self.reset_required = self.reset_barrier;
            self.recovery = Recovery::Starting;
        }

        let ready = !self.sleeping
            && !self.capture_closing
            && !self.close_failed
            && !self.reset_pending
            && !self.capture_active;
        if ready
            && inhibited
            && !matches!(
                self.recovery,
                Recovery::HelperUnavailable | Recovery::Exhausted
            )
        {
            if self.recovery == Recovery::Settling {
                let start = identity.is_some()
                    && identity == self.settle_seen.as_deref()
                    && now >= self.settle_after;
                if start {
                    actions.extend(self.start_capture(now, identity.expect("checked above")));
                } else if now >= self.settle_deadline {
                    actions.extend(self.recover(
                        now,
                        "USB node did not stabilize after reset",
                        false,
                    ));
                }
                self.settle_seen = identity.map(str::to_owned);
            } else if now >= self.retry_at {
                if self.reset_required {
                    actions.extend(self.begin_reset(now));
                } else if let Some(current) = identity
                    && self.seen.as_deref() == Some(current)
                {
                    actions.extend(self.start_capture(now, current));
                }
            }
        }

        self.seen = identity.map(str::to_owned);

        if self.capture_active {
            let reference = self.last_frame.unwrap_or(self.started);
            if now - reference > FRAME_WATCHDOG_SECS {
                actions.extend(self.recover(
                    now,
                    "No decoded frames for 12 s; attempting bounded USB recovery",
                    false,
                ));
            } else if let Some(last_frame) = self.last_frame
                && now - last_frame < 0.5
                && now - self.started >= 10.0
                && self.frames >= 90
            {
                self.retries = 0;
                self.error = None;
                self.recovery = Recovery::Streaming;
            }

            if now - self.last_report >= 10.0 {
                let elapsed = now - self.last_report;
                self.fps = (self.frames - self.report_frames) as f64 / elapsed;
                self.report_frames = self.frames;
                self.last_report = now;
            }
        }

        actions
    }

    pub fn capture_started(
        &mut self,
        generation: u64,
        now: f64,
        result: Result<(), String>,
    ) -> Vec<Action> {
        if generation != self.capture_generation {
            return Vec::new();
        }
        self.capture_starting = false;
        match result {
            Ok(()) => {
                self.recovery = Recovery::WaitingFrames;
                Vec::new()
            }
            Err(error) => self.recover(
                now,
                &format!("Capture failed to enter PLAYING: {error}"),
                false,
            ),
        }
    }

    pub fn capture_start_failed(&mut self, generation: u64, now: f64, reason: &str) -> Vec<Action> {
        self.capture_started(generation, now, Err(reason.to_string()))
    }

    /// A running capture emitted an error/EOS; stop it and enter recovery.
    pub fn capture_error(&mut self, now: f64, reason: &str) -> Vec<Action> {
        self.recover(now, reason, false)
    }

    pub fn capture_closed(&mut self, result: Result<(), String>) -> Vec<Action> {
        self.capture_closing = false;
        let mut actions = Vec::new();
        if let Err(error) = result {
            self.close_failed = true;
            self.error = Some(format!(
                "Capture teardown failed: {error}; restart bridge required"
            ));
        }
        if self.sleeping && !self.reset_pending && !self.config.test_source {
            self.sleep_inhibited = false;
            actions.push(Action::ReleaseSleepInhibitor);
        }
        actions
    }

    pub fn on_frame(&mut self, generation: u64, now: f64, frame: Frame) -> FrameOutcome {
        if generation != self.capture_generation || self.sleeping || self.stopping {
            return FrameOutcome::Ignored;
        }
        self.frames += 1;
        self.last_frame = Some(now);
        FrameOutcome::Accepted {
            displaced: self.latest.put(frame),
        }
    }

    pub fn publisher_started(&mut self) {
        self.publisher_active = true;
        self.publisher_starting = false;
        self.publisher_sessions += 1;
    }

    pub fn publisher_failed(&mut self, now: f64, reason: &str) {
        self.publisher_active = false;
        self.publisher_starting = false;
        self.error = Some(format!("Publisher unavailable: {reason}"));
        self.publisher_retry_at = now + 8.0;
    }

    pub fn publisher_exited(&mut self, now: f64) {
        self.publisher_active = false;
        self.publisher_starting = false;
        self.publisher_retry_at = now + 1.0;
    }

    pub fn prepare_sleep(&mut self) -> Vec<Action> {
        self.sleeping = true;
        self.recovery_generation += 1;
        self.recovery = Recovery::Sleeping;
        self.stop_capture()
    }

    pub fn resume(&mut self, now: f64) -> Vec<Action> {
        self.sleeping = false;
        self.recover(now, "Resuming camera after system sleep", true)
    }

    /// Notify that logind's delay budget expired before capture teardown.
    pub fn sleep_deadline(&mut self) {
        if self.sleeping && self.capture_closing {
            self.error = Some(
                "Capture teardown exceeded logind sleep delay; recovery waits for teardown"
                    .to_string(),
            );
        }
    }

    pub fn reset_finished(
        &mut self,
        now: f64,
        result: Result<ResetResult, ResetError>,
    ) -> Vec<Action> {
        self.reset_pending = false;
        let mut actions = Vec::new();
        if self.sleeping && !self.capture_closing && !self.config.test_source {
            self.sleep_inhibited = false;
            actions.push(Action::ReleaseSleepInhibitor);
        }
        if self.reset_token != self.recovery_generation || self.sleeping || self.stopping {
            return actions;
        }

        match result {
            Err(ResetError::Unavailable(message)) => {
                self.recovery = Recovery::HelperUnavailable;
                self.blocked = self
                    .current_identity
                    .clone()
                    .or_else(|| self.identity.clone())
                    .or_else(|| Some("missing".to_string()));
                self.error = Some(message);
            }
            Err(ResetError::DeviceMissing(message)) => {
                self.retries = 0;
                self.recovery = Recovery::Backoff;
                self.retry_at = now + 5.0;
                self.error = Some(format!(
                    "Jieli camera not present; waiting for USB re-enumeration: {message}"
                ));
            }
            Err(ResetError::Failed(message)) => {
                actions.extend(self.recover(now, &format!("USB reset failed: {message}"), false));
            }
            Ok(result) if result.returncode != 0 => {
                let message = format!(
                    "USB reset helper returned {}: {}",
                    result.returncode, result.message
                );
                self.last_reset = Some(result);
                actions.extend(self.recover(now, &message, false));
            }
            Ok(result) => {
                self.last_reset = Some(result);
                self.reset_barrier = false;
                self.reset_required = false;
                self.recovery = Recovery::Settling;
                self.settle_after = now + 1.0;
                self.settle_deadline = now + 10.0;
                self.settle_seen = None;
            }
        }
        actions
    }

    pub fn shutdown(&mut self) -> Vec<Action> {
        self.stopping = true;
        let mut actions = self.stop_capture();
        actions.push(Action::StopPublisher);
        actions
    }

    pub fn report(&self, pid: u32, now: f64) -> BridgeReport {
        let placeholder = self.sleeping
            || !self.capture_active
            || self
                .last_frame
                .is_none_or(|last_frame| now - last_frame > PLACEHOLDER_GRACE_SECS);
        BridgeReport {
            pid,
            node_name: self.config.node_name.clone(),
            capture_active: self.capture_active,
            capture_closing: self.capture_closing,
            capture_starting: self.capture_starting,
            publisher_active: self.publisher_active,
            capture_sessions: self.capture_sessions,
            publisher_sessions: self.publisher_sessions,
            frames: self.frames,
            fps_10s: round_to(self.fps, 3),
            seconds_since_frame: self
                .last_frame
                .map(|last_frame| round_to(now - last_frame, 3)),
            sleeping: self.sleeping,
            recovery: self.recovery,
            retries: self.retries,
            retry_in_seconds: round_to((self.retry_at - now).max(0.0), 2),
            reset_pending: self.reset_pending,
            reset_barrier: self.reset_barrier,
            last_reset: self.last_reset.clone(),
            sleep_inhibited: self.sleep_inhibited,
            sleep_monitor_error: self.sleep_monitor_error.clone(),
            placeholder,
            error: self.error.clone(),
        }
    }

    fn recover(&mut self, now: f64, reason: &str, fresh: bool) -> Vec<Action> {
        self.error = Some(reason.to_string());
        self.recovery_generation += 1;
        if fresh {
            self.retries = 0;
            self.blocked = None;
        }
        self.reset_required = !self.config.test_source;
        self.retry_at = now + backoff(self.retries);
        self.recovery = Recovery::Backoff;
        self.stop_capture()
    }

    fn start_capture(&mut self, now: f64, identity: &str) -> Vec<Action> {
        self.capture_generation += 1;
        self.capture_active = true;
        self.identity = Some(identity.to_string());
        self.frames = 0;
        self.report_frames = 0;
        self.started = now;
        self.last_report = now;
        self.last_frame = None;
        self.fps = 0.0;
        self.capture_sessions += 1;
        self.capture_starting = true;
        self.recovery = Recovery::Opening;
        self.latest.clear();
        vec![Action::StartCapture {
            generation: self.capture_generation,
            identity: identity.to_string(),
        }]
    }

    fn stop_capture(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        let was_active = self.capture_active;
        self.capture_active = false;
        self.capture_generation += 1;
        self.capture_starting = false;
        self.latest.clear();
        if was_active {
            self.capture_closing = true;
            actions.push(Action::StopCapture);
        } else if self.sleeping
            && !self.capture_closing
            && !self.reset_pending
            && !self.config.test_source
        {
            self.sleep_inhibited = false;
            actions.push(Action::ReleaseSleepInhibitor);
        }
        actions
    }

    fn begin_reset(&mut self, _now: f64) -> Vec<Action> {
        if self.retries >= MAX_RESETS {
            self.recovery = Recovery::Exhausted;
            self.blocked = self
                .current_identity
                .clone()
                .or_else(|| self.identity.clone())
                .or_else(|| Some("missing".to_string()));
            self.error = Some(format!(
                "Camera recovery exhausted after {MAX_RESETS} resets; reconnect camera or restart bridge"
            ));
            return Vec::new();
        }
        self.retries += 1;
        self.reset_pending = true;
        self.reset_barrier = true;
        self.recovery = Recovery::Resetting;
        self.reset_token = self.recovery_generation;
        vec![Action::BeginReset]
    }
}

fn backoff(retries: u32) -> f64 {
    if retries == 0 {
        0.0
    } else {
        (8.0 * 2f64.powi(retries as i32 - 1)).min(32.0)
    }
}

fn round_to(value: f64, places: u32) -> f64 {
    let factor = 10f64.powi(places as i32);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FRAME_SIZE;

    fn frame(byte: u8) -> Frame {
        Frame::from_vec(vec![byte; FRAME_SIZE]).expect("valid frame")
    }

    fn config() -> BridgeConfig {
        BridgeConfig {
            node_name: "fixture".to_string(),
            test_source: false,
        }
    }

    fn state() -> BridgeState {
        let mut state = BridgeState::new(config(), 100.0);
        state.seen = Some("camera-A".to_string());
        state
    }

    fn start_generation(actions: &[Action]) -> Option<u64> {
        actions.iter().find_map(|action| match action {
            Action::StartCapture { generation, .. } => Some(*generation),
            _ => None,
        })
    }

    fn active() -> (BridgeState, u64) {
        let mut state = state();
        let actions = state.tick(100.0, Some("camera-A"), true);
        let generation = start_generation(&actions).expect("start capture");
        let actions = state.capture_started(generation, 100.0, Ok(()));
        assert!(actions.is_empty());
        let outcome = state.on_frame(generation, 100.0, frame(1));
        assert!(matches!(
            outcome,
            FrameOutcome::Accepted { displaced: None }
        ));
        (state, generation)
    }

    #[test]
    fn capture_error_enters_backoff_and_stops_capture() {
        let (mut state, _) = active();
        let actions = state.capture_error(100.5, "capture: boom");
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::StopCapture))
        );
        assert_eq!(state.recovery, Recovery::Backoff);
        assert_eq!(state.error.as_deref(), Some("capture: boom"));
    }

    #[test]
    fn initial_report_is_starting_and_placeholder() {
        let state = BridgeState::new(config(), 100.0);
        let report = state.report(42, 100.0);
        assert_eq!(report.recovery, Recovery::Starting);
        assert!(report.placeholder);
        assert!(!report.capture_active);
        assert_eq!(report.frames, 0);
        assert_eq!(report.seconds_since_frame, None);
    }

    #[test]
    fn report_reflects_live_frames_and_placeholder_grace() {
        let (state, _) = active();
        let live = state.report(7, 100.1);
        assert!(live.capture_active);
        assert!(!live.placeholder);
        assert_eq!(live.frames, 1);
        assert_eq!(live.seconds_since_frame, Some(0.1));
        assert_eq!(live.capture_sessions, 1);
        assert_eq!(live.node_name, "fixture");
        assert_eq!(live.recovery, Recovery::WaitingFrames);

        let starved = state.report(7, 100.3);
        assert!(starved.placeholder);
        assert_eq!(starved.seconds_since_frame, Some(0.3));
    }

    #[test]
    fn starts_capture_only_after_identity_is_seen() {
        let mut state = BridgeState::new(config(), 100.0);
        let actions = state.tick(100.0, Some("camera-A"), true);
        assert!(start_generation(&actions).is_none());
        assert_eq!(state.seen.as_deref(), Some("camera-A"));
        let actions = state.tick(100.1, Some("camera-A"), true);
        assert!(start_generation(&actions).is_some());
        assert!(state.capture_active);
        assert_eq!(state.recovery, Recovery::Opening);
        assert_eq!(state.capture_sessions, 1);
    }

    #[test]
    fn streaming_after_warmup_clears_retries_and_error() {
        let (mut state, generation) = active();
        state.retries = 2;
        state.error = Some("old".to_string());
        state.started = 100.0;
        state.last_frame = Some(110.9);
        state.frames = 90;
        let _ = state.tick(111.0, Some("camera-A"), true);
        assert_eq!(state.recovery, Recovery::Streaming);
        assert_eq!(state.retries, 0);
        assert_eq!(state.error, None);
        assert_eq!(state.capture_generation, generation);
    }

    #[test]
    fn disconnect_stops_capture_but_keeps_publisher() {
        let (mut state, _) = active();
        state.publisher_started();
        let actions = state.tick(105.0, Some("camera-B"), true);
        assert!(actions.contains(&Action::StopCapture));
        assert!(state.publisher_active);
        assert!(!state.capture_active);
        assert!(state.capture_closing);
        assert!(state.reset_required);
        assert_eq!(state.recovery, Recovery::Backoff);
    }

    #[test]
    fn reset_waits_for_teardown_and_settles() {
        let (mut state, _) = active();
        state.recover(105.0, "no frames", true);
        let closed = state.capture_closed(Ok(()));
        assert!(closed.is_empty());
        let actions = state.tick(105.1, Some("camera-A"), true);
        assert!(actions.contains(&Action::BeginReset));
        assert!(state.reset_pending);
        assert_eq!(state.retries, 1);

        let actions = state.reset_finished(
            105.2,
            Ok(ResetResult {
                returncode: 0,
                message: "ok".to_string(),
            }),
        );
        assert!(actions.is_empty());
        assert_eq!(state.recovery, Recovery::Settling);
        assert!(state.last_reset.is_some());

        let actions = state.tick(105.3, Some("camera-A"), true);
        assert!(start_generation(&actions).is_none());
        let actions = state.tick(106.4, Some("camera-A"), true);
        assert!(start_generation(&actions).is_some());
    }

    #[test]
    fn reset_budget_is_bounded_and_exhaustion_blocks() {
        let mut state = state();
        state.reset_required = true;
        state.identity = Some("camera-A".to_string());
        for attempt in 1..=MAX_RESETS {
            let actions = state.tick(state.retry_at, Some("camera-A"), true);
            assert!(actions.contains(&Action::BeginReset));
            let actions = state.reset_finished(
                state.retry_at,
                Ok(ResetResult {
                    returncode: 1,
                    message: "fail".to_string(),
                }),
            );
            assert!(actions.is_empty());
            assert_eq!(state.retries, attempt);
            assert!(state.reset_required);
            assert_eq!(state.recovery, Recovery::Backoff);
        }
        let actions = state.tick(state.retry_at, Some("camera-A"), true);
        assert!(!actions.contains(&Action::BeginReset));
        assert_eq!(state.recovery, Recovery::Exhausted);
        assert!(state.blocked.is_some());
    }

    #[test]
    fn absent_device_does_not_exhaust_the_reset_budget() {
        let mut state = state();
        state.reset_required = true;
        state.identity = Some("camera-A".to_string());
        for _ in 0..10 {
            let actions = state.tick(state.retry_at, Some("camera-A"), true);
            assert!(actions.contains(&Action::BeginReset));
            let actions = state.reset_finished(
                state.retry_at,
                Err(ResetError::DeviceMissing(
                    "Expected exactly one Jieli 1224:2a25; found 0".to_string(),
                )),
            );
            assert!(actions.is_empty());
            assert_eq!(state.retries, 0);
            assert_eq!(state.recovery, Recovery::Backoff);
            assert!(state.reset_required);
            assert!(
                state
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("waiting for USB"))
            );
        }
    }

    #[test]
    fn helper_unavailable_is_terminal_until_device_changes() {
        let mut state = state();
        state.reset_required = true;
        state.identity = Some("camera-A".to_string());
        let _ = state.tick(100.0, Some("camera-A"), true);
        state.reset_finished(
            100.0,
            Err(ResetError::Unavailable("not authorized".to_string())),
        );
        assert_eq!(state.recovery, Recovery::HelperUnavailable);
        for offset in 1..=10 {
            let _ = state.tick(100.0 + offset as f64 * 100.0, Some("camera-A"), true);
        }
        assert_eq!(state.recovery, Recovery::HelperUnavailable);
        assert_eq!(state.retries, 1);
    }

    #[test]
    fn sleep_stops_capture_and_releases_after_teardown() {
        let (mut state, _) = active();
        state.publisher_started();
        let actions = state.prepare_sleep();
        assert!(actions.contains(&Action::StopCapture));
        assert!(state.sleeping);
        assert_eq!(state.recovery, Recovery::Sleeping);
        let actions = state.capture_closed(Ok(()));
        assert!(actions.contains(&Action::ReleaseSleepInhibitor));
        assert!(state.publisher_active);
        assert!(state.report(1, 110.0).placeholder);
    }

    #[test]
    fn sleep_deadline_reports_teardown_hang() {
        let (mut state, _) = active();
        state.prepare_sleep();
        state.sleep_deadline();
        assert!(
            state
                .error
                .as_deref()
                .is_some_and(|error| error.contains("logind sleep delay"))
        );
    }

    #[test]
    fn resume_reinitializes_budget_but_keeps_publication() {
        let (mut state, _) = active();
        state.publisher_started();
        state.retries = 3;
        let _ = state.prepare_sleep();
        let _ = state.capture_closed(Ok(()));
        let actions = state.resume(120.0);
        assert!(actions.is_empty());
        assert!(!state.sleeping);
        assert_eq!(state.retries, 0);
        assert!(state.reset_required);
        assert!(state.publisher_active);
    }

    #[test]
    fn stale_frame_generation_is_ignored() {
        let (mut state, generation) = active();
        let before = state.frames;
        let outcome = state.on_frame(generation - 1, 101.0, frame(2));
        assert!(matches!(outcome, FrameOutcome::Ignored));
        assert_eq!(state.frames, before);
    }

    #[test]
    fn publisher_exit_restarts_after_one_second() {
        let mut state = state();
        state.publisher_started();
        assert!(state.publisher_active);
        state.publisher_exited(100.0);
        let actions = state.tick(100.5, Some("camera-A"), true);
        assert!(!actions.contains(&Action::StartPublisher));
        let actions = state.tick(101.0, Some("camera-A"), true);
        assert!(actions.contains(&Action::StartPublisher));
    }

    #[test]
    fn publisher_spawn_failure_retries_after_eight_seconds() {
        let mut state = state();
        let actions = state.tick(100.0, Some("camera-A"), true);
        assert!(actions.contains(&Action::StartPublisher));
        state.publisher_failed(100.0, "missing element");
        assert!(!state.publisher_active);
        assert_eq!(
            state.error.as_deref(),
            Some("Publisher unavailable: missing element")
        );

        let actions = state.tick(107.9, Some("camera-A"), true);
        assert!(!actions.contains(&Action::StartPublisher));
        let actions = state.tick(108.0, Some("camera-A"), true);
        assert!(actions.contains(&Action::StartPublisher));
    }

    #[test]
    fn failed_capture_teardown_blocks_reset_and_reopen() {
        let (mut state, _) = active();
        state.recover(105.0, "no frames", true);
        state.capture_closed(Err("stuck".to_string()));
        assert!(state.close_failed);
        assert!(state.error.as_deref().unwrap().contains("teardown failed"));

        let actions = state.tick(106.0, Some("camera-A"), true);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::BeginReset | Action::StartCapture { .. }))
        );
    }

    #[test]
    fn stale_reset_after_sleep_cannot_reopen() {
        let mut state = state();
        state.reset_required = true;
        state.identity = Some("camera-A".to_string());
        let _ = state.tick(100.0, Some("camera-A"), true);
        assert!(state.reset_pending);
        let _ = state.prepare_sleep();
        let actions = state.reset_finished(
            100.1,
            Ok(ResetResult {
                returncode: 0,
                message: "late".to_string(),
            }),
        );
        assert!(!state.reset_pending);
        assert_eq!(state.recovery, Recovery::Sleeping);
        assert!(state.reset_required);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::StartCapture { .. }))
        );
    }

    #[test]
    fn no_frames_watchdog_attempts_recovery() {
        let (mut state, _) = active();
        state.started = 87.0;
        state.last_frame = None;
        let actions = state.tick(100.0, Some("camera-A"), true);
        assert!(actions.contains(&Action::StopCapture));
        assert!(state.reset_required);
        assert!(state.capture_closing);
    }

    #[test]
    fn physical_reenumeration_releases_exhaustion() {
        let mut state = state();
        state.blocked = Some("camera-old".to_string());
        state.recovery = Recovery::Exhausted;
        state.retries = 3;
        let actions = state.tick(100.0, Some("camera-A"), true);
        assert_eq!(state.retries, 0);
        assert!(state.blocked.is_none());
        assert!(start_generation(&actions).is_some());
    }

    #[test]
    fn timeout_barrier_survives_physical_reenumeration() {
        let mut state = state();
        state.blocked = Some("camera-old".to_string());
        state.recovery = Recovery::Exhausted;
        state.retries = 3;
        state.reset_barrier = true;
        let actions = state.tick(100.0, Some("camera-A"), true);
        assert!(state.reset_pending);
        assert!(state.reset_required);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::StartCapture { .. }))
        );
    }

    #[test]
    fn known_recovery_can_reset_without_v4l2_node() {
        let mut state = state();
        state.reset_required = true;
        let actions = state.tick(100.0, None, true);
        assert!(actions.contains(&Action::BeginReset));
        assert!(state.reset_pending);
    }

    #[test]
    fn old_frame_burst_cannot_erase_recovery_budget() {
        let (mut state, _) = active();
        state.started = 89.0;
        state.last_frame = Some(93.0);
        state.frames = 100;
        state.retries = 2;
        let _ = state.tick(100.0, Some("camera-A"), true);
        assert_eq!(state.retries, 2);
        assert_ne!(state.recovery, Recovery::Streaming);
    }

    #[test]
    fn test_source_does_not_rename_the_node_and_needs_no_lease() {
        let mut state = BridgeState::new(
            BridgeConfig {
                node_name: "fixture".to_string(),
                test_source: true,
            },
            100.0,
        );
        assert_eq!(state.description(), "Jieli validation");
        state.seen = Some("synthetic".to_string());
        let actions = state.tick(100.0, Some("synthetic"), true);
        assert!(start_generation(&actions).is_some());
        assert_eq!(state.config().node_name, "fixture");
        assert!(!state.reset_required);
    }
}
