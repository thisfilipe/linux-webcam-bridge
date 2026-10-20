use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jieli_publisher::args::{ArgsError, USAGE, parse_args};
use jieli_publisher::feed::{PublisherFeed, feed_stdin};
use jieli_publisher::v4l2;

static QUIT: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static DEBUG_REQUESTED: AtomicBool = AtomicBool::new(false);
static DEBUG_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

extern "C" fn quit_handler(_signal: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

extern "C" fn debug_handler(_signal: libc::c_int) {
    DEBUG_REQUESTED.store(true, Ordering::SeqCst);
}

fn main() {
    let arguments = match parse_args(std::env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(ArgsError::Help) => {
            println!("{USAGE}");
            return;
        }
        Err(error) => {
            eprintln!("jieli-publisher: {error}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    set_pipe_size();
    let pid_file = pid_file_path(&arguments.node_name);
    if let Some(parent) = pid_file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&pid_file, format!("{}\n", std::process::id()));

    install_signals();
    if debug_flag_exists(&arguments.node_name) {
        DEBUG_REQUESTED.store(true, Ordering::SeqCst);
    }

    let home = home_dir();
    let v4l2_enabled = v4l2::enabled(&arguments.node_name, &home);

    let feed = Arc::new(Mutex::new(PublisherFeed::new(Duration::ZERO)));
    if v4l2_enabled {
        jieli_publisher::v4l2_writer::spawn(Arc::clone(&feed), &QUIT);
    }
    {
        let feed = Arc::clone(&feed);
        std::thread::spawn(move || feed_stdin(feed, &QUIT));
    }

    let result = jieli_publisher::native::run(jieli_publisher::native::NativeOptions {
        node_name: &arguments.node_name,
        description: &arguments.description,
        feed,
        quit: &QUIT,
        failed: &FAILED,
        debug_requested: &DEBUG_REQUESTED,
        debug_until_ms: &DEBUG_UNTIL_MS,
    });

    if let Err(error) = result {
        eprintln!("publisher: {error}");
        FAILED.store(true, Ordering::SeqCst);
    }
    let _ = fs::remove_file(&pid_file);
    std::process::exit(if FAILED.load(Ordering::SeqCst) { 1 } else { 0 });
}

fn set_pipe_size() {
    unsafe {
        libc::fcntl(0, libc::F_SETPIPE_SZ, 1024 * 1024);
    }
}

fn pid_file_path(node_name: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(format!("jieli-publisher-{node_name}.pid"))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn debug_flag_exists(node_name: &str) -> bool {
    let home = home_dir();
    !home.as_os_str().is_empty()
        && home
            .join(".cache/jieli-camera")
            .join(format!("debug-{node_name}"))
            .exists()
}

fn install_signals() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            quit_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            quit_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGUSR1,
            debug_handler as *const () as libc::sighandler_t,
        );
    }
}
