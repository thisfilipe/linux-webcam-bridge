//! Pure, system-independent logic for the Jieli camera bridge.
//!
//! This crate owns the contracts that do not need GStreamer, PipeWire, D-Bus,
//! USB or the filesystem: frame ownership, the neutral placeholder, partial
//! pipe framing, the report schema and (in later phases) the supervisor state
//! machine.

pub mod frame;
pub mod framing;
pub mod placeholder;
pub mod report;
pub mod state;

pub use frame::{
    FRAME_SIZE, Frame, FrameError, HEIGHT, LatestSlot, UV_HEIGHT, UV_SIZE, UV_WIDTH, WIDTH, Y_SIZE,
};
pub use framing::{FALLBACK_INTERVAL, FrameReader, STARVATION};
pub use placeholder::{
    CAMERA_UNAVAILABLE, PLACEHOLDER_SHA256, RECONNECTING, build_placeholder, placeholder,
};
pub use report::{BridgeReport, REPORT_KEYS, Recovery, ResetResult, remove_state_file};
pub use state::{Action, BridgeConfig, BridgeState, FrameOutcome, ResetError};
