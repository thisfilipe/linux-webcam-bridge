use std::ffi::OsString;

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    match jieli_usb::entry(&args, jieli_usb::effective_uid()) {
        Ok(()) => {}
        Err(jieli_usb::EntryError::Usage(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
        Err(jieli_usb::EntryError::Reset(message)) => {
            eprintln!("Jieli reset failed: {message}");
            std::process::exit(1);
        }
    }
}
