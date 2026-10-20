use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use crate::device::{Device, discover};
use crate::error::{HelperError, io_message};
use crate::lock::{self, LOCK_PATH, ResetLock, acquire};
use crate::verify::{VerifiedDevice, open_verified};

/// `_IO('U', 20)`, the value used by the live helper (`0x5514`).
pub const USBDEVFS_RESET: libc::c_ulong = 0x5514;
const ENODEV: i32 = libc::ENODEV;
const REENUMERATION_TIMEOUT_SECONDS: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    Completed,
    Reenumerated,
}

pub fn reset() -> Result<ResetOutcome, HelperError> {
    let lock = acquire(Path::new(LOCK_PATH))?;
    reset_locked_with(&lock, discover, open_verified, ioctl_reset)
}

/// The safety-critical order is fixed here and exercised by injected tests:
/// cooldown before any device access, one verified fd, one reset ioctl.
fn reset_locked_with<D, O, I>(
    lock: &ResetLock,
    discover_device: D,
    open_device: O,
    reset_ioctl: I,
) -> Result<ResetOutcome, HelperError>
where
    D: Fn() -> Result<Device, HelperError>,
    O: Fn(&Device) -> Result<VerifiedDevice, HelperError>,
    I: Fn(&VerifiedDevice) -> std::io::Result<()>,
{
    let previous = lock::read_previous(&lock.fd)?;
    let now = monotonic_seconds();
    lock::cooldown_allows(previous, now)?;

    let device = discover_device()?;
    let verified = open_device(&device)?;
    lock::write_timestamp(&lock.fd, now)?;

    match reset_ioctl(&verified) {
        Ok(()) => {
            println!(
                "Jieli USB reset completed at {:03}/{:03}",
                device.bus, device.address
            );
            Ok(ResetOutcome::Completed)
        }
        Err(error) if error.raw_os_error() == Some(ENODEV) => {
            wait_for_reenumeration(&device, verified.inode, error)
        }
        Err(error) => Err(HelperError::Os(io_message(&error))),
    }
}

fn ioctl_reset(device: &VerifiedDevice) -> std::io::Result<()> {
    let result = unsafe { libc::ioctl(device.fd.as_raw_fd(), USBDEVFS_RESET, 0) };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wait_for_reenumeration(
    before: &Device,
    before_inode: u64,
    original: std::io::Error,
) -> Result<ResetOutcome, HelperError> {
    let deadline = monotonic_seconds() + REENUMERATION_TIMEOUT_SECONDS;
    while monotonic_seconds() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        let Some(current) = current_candidate(before, before_inode) else {
            continue;
        };
        match open_verified(&current) {
            Ok(verified) => {
                drop(verified);
                println!(
                    "Jieli reenumerated at {:03}/{:03}; capture must validate recovery",
                    current.bus, current.address
                );
                return Ok(ResetOutcome::Reenumerated);
            }
            Err(_) => continue,
        }
    }
    Err(HelperError::Os(io_message(&original)))
}

fn current_candidate(before: &Device, before_inode: u64) -> Option<Device> {
    use std::os::unix::fs::MetadataExt;

    let current = discover().ok()?;
    let inode = std::fs::metadata(&current.path).ok()?.ino();
    if reenumeration_detected(before_inode, inode, before, &current) {
        Some(current)
    } else {
        None
    }
}

/// The live helper treats a changed device-node inode or a changed bus/address
/// as evidence that the device reenumerated after `ENODEV`.
pub fn reenumeration_detected(
    before_inode: u64,
    current_inode: u64,
    before: &Device,
    current: &Device,
) -> bool {
    current_inode != before_inode || current.bus != before.bus || current.address != before.address
}

pub fn monotonic_seconds() -> f64 {
    let mut timestamp: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timestamp) };
    timestamp.tv_sec as f64 + timestamp.tv_nsec as f64 / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use rustix::fs::{Mode, OFlags, SeekFrom};

    use super::*;

    fn lock_file(directory: &tempfile::TempDir) -> ResetLock {
        let path = directory.path().join("lock");
        let fd = rustix::fs::open(
            path.as_path(),
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .expect("lock file");
        ResetLock { fd }
    }

    fn fake_device() -> Device {
        Device {
            name: "fixture".to_string(),
            bus: 8,
            address: 15,
            path: PathBuf::from("/dev/null"),
        }
    }

    fn fake_verified() -> VerifiedDevice {
        VerifiedDevice {
            fd: rustix::fs::open("/dev/null", OFlags::RDWR, Mode::empty()).expect("/dev/null"),
            inode: 42,
        }
    }

    #[test]
    fn reset_request_matches_usbdevfs_reset() {
        assert_eq!(USBDEVFS_RESET, 0x5514);
    }

    #[test]
    fn monotonic_clock_advances() {
        let first = monotonic_seconds();
        assert!(first > 0.0);
        let second = monotonic_seconds();
        assert!(second >= first);
    }

    #[test]
    fn cooldown_blocks_before_any_device_access() {
        let directory = tempfile::tempdir().unwrap();
        let lock = lock_file(&directory);
        lock::write_timestamp(&lock.fd, monotonic_seconds() - 1.0).unwrap();
        rustix::fs::seek(&lock.fd, SeekFrom::Start(0)).unwrap();

        let discovered = Cell::new(0);
        let opened = Cell::new(0);
        let ioctls = Cell::new(0);
        let result = reset_locked_with(
            &lock,
            || {
                discovered.set(discovered.get() + 1);
                Ok(fake_device())
            },
            |_| {
                opened.set(opened.get() + 1);
                Ok(fake_verified())
            },
            |_| {
                ioctls.set(ioctls.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result, Err(HelperError::Cooldown));
        assert_eq!((discovered.get(), opened.get(), ioctls.get()), (0, 0, 0));
    }

    #[test]
    fn success_runs_one_verified_ioctl_and_records_timestamp() {
        let directory = tempfile::tempdir().unwrap();
        let lock = lock_file(&directory);
        let discovered = Cell::new(0);
        let opened = Cell::new(0);
        let ioctls = Cell::new(0);
        let ioctl_inode = Cell::new(0u64);

        let result = reset_locked_with(
            &lock,
            || {
                discovered.set(discovered.get() + 1);
                Ok(fake_device())
            },
            |_| {
                opened.set(opened.get() + 1);
                Ok(fake_verified())
            },
            |verified| {
                ioctls.set(ioctls.get() + 1);
                ioctl_inode.set(verified.inode);
                Ok(())
            },
        );

        assert_eq!(result, Ok(ResetOutcome::Completed));
        assert_eq!(
            (
                discovered.get(),
                opened.get(),
                ioctls.get(),
                ioctl_inode.get()
            ),
            (1, 1, 1, 42)
        );

        rustix::fs::seek(&lock.fd, SeekFrom::Start(0)).unwrap();
        let stored = lock::read_previous(&lock.fd).unwrap().expect("timestamp");
        assert!((stored - monotonic_seconds()).abs() < 5.0);
    }

    #[test]
    fn open_failure_never_reaches_ioctl() {
        let directory = tempfile::tempdir().unwrap();
        let lock = lock_file(&directory);
        let ioctls = Cell::new(0);

        let result = reset_locked_with(
            &lock,
            || Ok(fake_device()),
            |_| Err(HelperError::NotCharacterDevice),
            |_| {
                ioctls.set(ioctls.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result, Err(HelperError::NotCharacterDevice));
        assert_eq!(ioctls.get(), 0);
    }

    #[test]
    fn reenumeration_requires_changed_inode_or_address() {
        let before = Device {
            name: "8-1".to_string(),
            bus: 8,
            address: 15,
            path: std::path::PathBuf::from("/dev/bus/usb/008/015"),
        };
        let same = Device {
            name: "8-1".to_string(),
            bus: 8,
            address: 15,
            path: std::path::PathBuf::from("/dev/bus/usb/008/015"),
        };
        assert!(!reenumeration_detected(7, 7, &before, &same));
        assert!(reenumeration_detected(7, 8, &before, &same));
        assert!(reenumeration_detected(
            7,
            7,
            &before,
            &Device {
                address: 16,
                ..same
            }
        ));
    }
}
