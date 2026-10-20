use thiserror::Error;

pub const VID: u16 = 0x1224;
pub const PID: u16 = 0x2A25;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HelperError {
    #[error("Expected exactly one Jieli 1224:2a25; found {0}")]
    DeviceCount(usize),
    #[error("Target is not a USB character device")]
    NotCharacterDevice,
    #[error("USB address changed before opening")]
    AddressChanged,
    #[error("Invalid USB device descriptor")]
    InvalidDescriptor,
    #[error("Opened device is not Jieli 1224:2a25")]
    WrongDevice,
    #[error("Unsafe reset lock file")]
    UnsafeLock,
    #[error("Reset cooldown: wait at least eight seconds between attempts")]
    Cooldown,
    #[error("{0}")]
    Os(String),
    #[error("{0}")]
    Message(String),
}

impl HelperError {
    pub fn from_errno(errno: rustix::io::Errno) -> Self {
        Self::Os(errno_message(errno.raw_os_error()))
    }
}

/// Python-style `[Errno N] text`, matching `str(OSError)` on Linux.
pub fn errno_message(code: i32) -> String {
    let text = unsafe {
        let pointer = libc::strerror(code);
        if pointer.is_null() {
            format!("Unknown error {code}")
        } else {
            std::ffi::CStr::from_ptr(pointer)
                .to_string_lossy()
                .into_owned()
        }
    };
    format!("[Errno {code}] {text}")
}

pub fn io_message(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => errno_message(code),
        None => error.to_string(),
    }
}
