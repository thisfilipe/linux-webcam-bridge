use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
pub enum EntryError {
    Usage(String),
    Reset(String),
}

/// `argv` includes the program name, exactly like `sys.argv`.
pub fn check_usage(args: &[OsString]) -> Result<(), EntryError> {
    if args.len() != 1 {
        return Err(EntryError::Usage(
            "This helper accepts no arguments".to_string(),
        ));
    }
    Ok(())
}

pub fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn entry(args: &[OsString], euid: u32) -> Result<(), EntryError> {
    check_usage(args)?;
    if euid != 0 {
        return Err(EntryError::Usage(
            "This installed helper requires its narrowly scoped sudo rule".to_string(),
        ));
    }

    install_alarm();
    let result = crate::reset::reset();
    unsafe { libc::alarm(0) };
    result
        .map(|_| ())
        .map_err(|error| EntryError::Reset(error.to_string()))
}

const ALARM_MESSAGE: &[u8] = b"Jieli reset failed: Jieli reset exceeded eight seconds\n";

extern "C" fn alarm_handler(_signal: libc::c_int) {
    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            ALARM_MESSAGE.as_ptr() as *const libc::c_void,
            ALARM_MESSAGE.len(),
        );
        libc::_exit(1);
    }
}

fn install_alarm() {
    unsafe {
        libc::signal(
            libc::SIGALRM,
            alarm_handler as *const () as libc::sighandler_t,
        );
        libc::alarm(8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn rejects_arguments_before_any_reset() {
        assert_eq!(
            check_usage(&args(&["helper", "8/15"])),
            Err(EntryError::Usage(
                "This helper accepts no arguments".to_string()
            ))
        );
        assert_eq!(check_usage(&args(&["helper"])), Ok(()));
    }

    #[test]
    fn rejects_unprivileged_execution() {
        assert_eq!(
            entry(&args(&["helper"]), 1000),
            Err(EntryError::Usage(
                "This installed helper requires its narrowly scoped sudo rule".to_string()
            ))
        );
    }
}
