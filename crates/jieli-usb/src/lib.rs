//! Root helper that resets exactly one verified Jieli 1224:2a25 USB device.
//!
//! The public binary is `jieli-usb-reset`. All decision logic is in pure
//! functions (`validate_target`, `validate_descriptor`, `validate_lock`,
//! `cooldown_allows`, discovery) so the safety properties are testable without
//! root, USB hardware or the real sysfs.

pub mod device;
pub mod entry;
pub mod error;
pub mod lock;
pub mod reset;
pub mod verify;

pub use device::{
    DEV_ROOT, Device, FileKind, LockStat, SYSFS_ROOT, TargetStat, USB_MAJOR, discover, discover_in,
    validate_descriptor, validate_lock, validate_target,
};
pub use entry::{EntryError, check_usage, effective_uid, entry};
pub use error::{HelperError, PID, VID};
pub use lock::{LOCK_PATH, MIN_INTERVAL, cooldown_allows, read_previous, write_timestamp};
pub use reset::{ResetOutcome, USBDEVFS_RESET, reset};
pub use verify::{VerifiedDevice, open_verified};
