use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::{self as rfs, FlockOperation, Mode, OFlags};
use rustix::io;

use crate::device::validate_lock;
use crate::error::{HelperError, errno_message};
use crate::verify::lock_stat;

pub const LOCK_PATH: &str = "/run/lock/jieli-usb-reset.lock";
pub const MIN_INTERVAL: f64 = 8.0;

#[derive(Debug)]
pub struct ResetLock {
    pub(crate) fd: OwnedFd,
}

pub fn acquire(path: &Path) -> Result<ResetLock, HelperError> {
    let fd = rfs::open(
        path,
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|errno| {
        HelperError::Os(format!(
            "{}: '{}'",
            errno_message(errno.raw_os_error()),
            path.display()
        ))
    })?;

    let stat = rfs::fstat(&fd).map_err(HelperError::from_errno)?;
    validate_lock(lock_stat(&stat))?;
    rfs::flock(&fd, FlockOperation::NonBlockingLockExclusive).map_err(HelperError::from_errno)?;

    Ok(ResetLock { fd })
}

pub fn read_previous(fd: &OwnedFd) -> Result<Option<f64>, HelperError> {
    let mut buffer = [0u8; 128];
    let count = io::read(fd, &mut buffer).map_err(HelperError::from_errno)?;
    let text = String::from_utf8_lossy(&buffer[..count]);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<f64>().map(Some).map_err(|_| {
        // Python's `float(previous)` error text, including the bytes repr.
        HelperError::Message(format!("could not convert string to float: b'{text}'"))
    })
}

/// The live helper allows a reset when the previous attempt is absent or at
/// least eight seconds old (`now - previous < 8.0` blocks).
pub fn cooldown_allows(previous: Option<f64>, now: f64) -> Result<(), HelperError> {
    if let Some(previous) = previous
        && now - previous < MIN_INTERVAL
    {
        return Err(HelperError::Cooldown);
    }
    Ok(())
}

pub fn write_timestamp(fd: &OwnedFd, now: f64) -> Result<(), HelperError> {
    rfs::seek(fd, rfs::SeekFrom::Start(0)).map_err(HelperError::from_errno)?;
    rfs::ftruncate(fd, 0).map_err(HelperError::from_errno)?;

    let text = format!("{now}");
    let mut written = 0;
    while written < text.len() {
        let count = io::write(fd, &text.as_bytes()[written..]).map_err(HelperError::from_errno)?;
        if count == 0 {
            return Err(HelperError::Message(
                "short write to the reset lock".to_string(),
            ));
        }
        written += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_is_eight_seconds() {
        assert_eq!(MIN_INTERVAL, 8.0);
        assert_eq!(cooldown_allows(None, 20.0), Ok(()));
        assert_eq!(cooldown_allows(Some(11.0), 20.0), Ok(()));
        assert_eq!(cooldown_allows(Some(12.0), 20.0), Ok(()));
        assert_eq!(
            cooldown_allows(Some(12.1), 20.0),
            Err(HelperError::Cooldown)
        );
    }

    #[test]
    fn flock_is_exclusive_and_nonblocking() {
        use rustix::fs::{FlockOperation, Mode, OFlags};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let first = rustix::fs::open(
            path.as_path(),
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .unwrap();
        rustix::fs::flock(&first, FlockOperation::NonBlockingLockExclusive).unwrap();

        let second = rustix::fs::open(path.as_path(), OFlags::RDWR, Mode::empty()).unwrap();
        let error =
            rustix::fs::flock(&second, FlockOperation::NonBlockingLockExclusive).unwrap_err();
        assert_eq!(error.raw_os_error(), libc::EWOULDBLOCK);

        drop(first);
        rustix::fs::flock(&second, FlockOperation::NonBlockingLockExclusive).unwrap();
    }

    #[test]
    fn timestamp_round_trips_and_rejects_garbage() {
        use rustix::fs::{Mode, OFlags};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let fd = rustix::fs::open(
            path.as_path(),
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .unwrap();

        assert_eq!(read_previous(&fd).unwrap(), None);
        write_timestamp(&fd, 123.5).unwrap();
        rustix::fs::seek(&fd, rustix::fs::SeekFrom::Start(0)).unwrap();
        assert_eq!(read_previous(&fd).unwrap(), Some(123.5));
        write_timestamp(&fd, 456.25).unwrap();
        rustix::fs::seek(&fd, rustix::fs::SeekFrom::Start(0)).unwrap();
        assert_eq!(read_previous(&fd).unwrap(), Some(456.25));

        std::fs::write(&path, b"not-a-number").unwrap();
        rustix::fs::seek(&fd, rustix::fs::SeekFrom::Start(0)).unwrap();
        assert_eq!(
            read_previous(&fd),
            Err(HelperError::Message(
                "could not convert string to float: b'not-a-number'".to_string()
            ))
        );
    }

    #[test]
    fn lock_path_matches_the_live_helper() {
        assert_eq!(LOCK_PATH, "/run/lock/jieli-usb-reset.lock");
    }

    #[test]
    fn acquire_rejects_a_non_root_owned_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(acquire(&path).unwrap_err(), HelperError::UnsafeLock);
    }

    #[test]
    fn acquire_refuses_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, b"").unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match acquire(&link).unwrap_err() {
            HelperError::Os(message) => {
                assert!(
                    message.contains("Too many levels of symbolic links"),
                    "{message}"
                );
                assert!(message.contains("link"), "{message}");
            }
            other => panic!("expected an OS error, got {other:?}"),
        }
    }
}
