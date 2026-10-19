use std::os::fd::{AsFd, OwnedFd};

use rustix::fs::{self as rfs, Mode, OFlags, Stat};
use rustix::io;

use crate::device::{
    Device, LockStat, TargetStat, file_kind_from_raw_mode, validate_descriptor, validate_target,
};
use crate::error::{HelperError, errno_message};

/// A device node opened and verified in one operation.
///
/// The fd is private and there is no public constructor: callers cannot
/// fabricate a verified device, and the reset ioctl runs on this same fd whose
/// descriptor bytes were checked.
#[derive(Debug)]
pub struct VerifiedDevice {
    pub(crate) fd: OwnedFd,
    pub(crate) inode: u64,
}

impl VerifiedDevice {
    pub fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

pub fn open_verified(device: &Device) -> Result<VerifiedDevice, HelperError> {
    let fd = rfs::open(
        device.path.as_path(),
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|errno| {
        HelperError::Os(format!(
            "{}: '{}'",
            errno_message(errno.raw_os_error()),
            device.path.display()
        ))
    })?;

    let stat = rfs::fstat(&fd).map_err(HelperError::from_errno)?;
    validate_target(device, target_stat(&stat))?;

    let mut descriptor = [0u8; 18];
    let count = io::read(&fd, &mut descriptor).map_err(HelperError::from_errno)?;
    validate_descriptor(&descriptor[..count])?;

    Ok(VerifiedDevice {
        fd,
        inode: stat.st_ino,
    })
}

pub fn target_stat(stat: &Stat) -> TargetStat {
    TargetStat {
        kind: file_kind_from_raw_mode(stat.st_mode),
        major: rfs::major(stat.st_rdev),
        minor: rfs::minor(stat.st_rdev),
    }
}

pub fn lock_stat(stat: &Stat) -> LockStat {
    LockStat {
        uid: stat.st_uid,
        kind: file_kind_from_raw_mode(stat.st_mode),
        nlink: stat.st_nlink,
        mode: stat.st_mode & 0o777,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::device::{Device, FileKind};

    #[test]
    fn regular_file_is_rejected_before_descriptor_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-a-device");
        std::fs::write(&path, b"x").unwrap();
        let device = Device {
            name: "fixture".to_string(),
            bus: 8,
            address: 15,
            path: PathBuf::from(&path),
        };
        assert_eq!(
            open_verified(&device).err(),
            Some(HelperError::NotCharacterDevice)
        );
    }

    #[test]
    fn missing_path_reports_os_error_with_path() {
        let device = Device {
            name: "missing".to_string(),
            bus: 8,
            address: 15,
            path: PathBuf::from("/nonexistent/jieli-usb-test-device"),
        };
        match open_verified(&device).expect_err("error") {
            HelperError::Os(message) => {
                assert!(
                    message.contains("/nonexistent/jieli-usb-test-device"),
                    "{message}"
                );
            }
            other => panic!("expected an OS error, got {other:?}"),
        }
    }

    #[test]
    fn target_stat_reads_character_device_numbers() {
        let fd = rfs::open("/dev/null", OFlags::RDWR, Mode::empty()).expect("/dev/null");
        let stat = rfs::fstat(&fd).expect("fstat");
        let mapped = target_stat(&stat);
        assert_eq!(mapped.kind, FileKind::Character);
        assert_eq!(mapped.major, 1);
        assert_eq!(mapped.minor, 3);
    }

    #[test]
    fn lock_stat_preserves_identity_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let fd = rfs::open(
            path.as_path(),
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .expect("lock file");
        let stat = rfs::fstat(&fd).expect("fstat");
        let mapped = lock_stat(&stat);
        assert_eq!(mapped.kind, FileKind::Regular);
        assert_eq!(mapped.uid, unsafe { libc::geteuid() });
        assert_eq!(mapped.nlink, 1);
        assert_eq!(mapped.mode, 0o600);
    }
}
