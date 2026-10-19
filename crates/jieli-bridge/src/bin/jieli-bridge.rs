use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jieli_bridge::args::{ArgsError, USAGE, parse_args};
use jieli_bridge::capture::{Capture, CaptureCallbacks};
use jieli_bridge::publisher::PublisherManager;
use jieli_core::frame::Frame;
use jieli_core::state::{Action, BridgeConfig, BridgeState};

static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn quit_handler(_signal: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

fn main() {
    let arguments = match parse_args(std::env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(ArgsError::Help) => {
            println!("{USAGE}");
            return;
        }
        Err(error) => {
            eprintln!("jieli-bridge: {error}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    if !arguments.test_source {
        eprintln!("jieli-bridge: real V4L2 capture is not ported yet; use --test-source");
        std::process::exit(2);
    }

    if let Err(error) = gstreamer::init() {
        eprintln!("jieli-bridge: GStreamer initialization failed: {error}");
        std::process::exit(1);
    }

    install_signals();
    let start = Arc::new(Instant::now());
    let state = Arc::new(Mutex::new(BridgeState::new(
        BridgeConfig {
            node_name: arguments.node_name.clone(),
            test_source: arguments.test_source,
        },
        0.0,
    )));
    let frame_tx: Arc<Mutex<Option<SyncSender<Frame>>>> = Arc::new(Mutex::new(None));
    let mut publisher = PublisherManager::new(frame_tx.clone());
    let mut capture: Option<Capture> = None;

    loop {
        let now = start.elapsed().as_secs_f64();

        if let Some(active) = capture.as_ref()
            && let Some(reason) = active.poll_error()
        {
            let actions = state
                .lock()
                .map(|mut state| state.capture_error(now, &reason))
                .unwrap_or_default();
            apply_actions(
                actions,
                &state,
                &mut capture,
                &mut publisher,
                &frame_tx,
                &start,
                &arguments.node_name,
                now,
            );
        }

        if publisher.poll_exit()
            && let Ok(mut state) = state.lock()
        {
            state.publisher_exited(now);
        }

        let actions = match state.lock() {
            Ok(mut state) => state.tick(now, Some("synthetic"), true),
            Err(_) => break,
        };
        apply_actions(
            actions,
            &state,
            &mut capture,
            &mut publisher,
            &frame_tx,
            &start,
            &arguments.node_name,
            now,
        );

        if let Ok(state) = state.lock() {
            let report = state.report(std::process::id(), now);
            if let Err(error) = report.write_atomic(&arguments.state_file) {
                eprintln!(
                    "jieli-bridge: cannot write {}: {error}",
                    arguments.state_file.display()
                );
            }
        }

        if QUIT.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1000));
    }

    let shutdown = state
        .lock()
        .map(|mut state| state.shutdown())
        .unwrap_or_default();
    apply_actions(
        shutdown,
        &state,
        &mut capture,
        &mut publisher,
        &frame_tx,
        &start,
        &arguments.node_name,
        start.elapsed().as_secs_f64(),
    );
    if let Some(active) = capture.take() {
        let _ = active.stop();
    }
    publisher.stop();
    let _ = std::fs::remove_file(&arguments.state_file);
}

#[allow(clippy::too_many_arguments)]
fn apply_actions(
    actions: Vec<Action>,
    state: &Arc<Mutex<BridgeState>>,
    capture: &mut Option<Capture>,
    publisher: &mut PublisherManager,
    frame_tx: &Arc<Mutex<Option<SyncSender<Frame>>>>,
    start: &Arc<Instant>,
    node_name: &str,
    now: f64,
) {
    for action in actions {
        match action {
            Action::StartPublisher => match publisher.start(node_name, "Jieli validation") {
                Ok(()) => {
                    if let Ok(mut state) = state.lock() {
                        state.publisher_started();
                    }
                    println!("Publishing {node_name}");
                }
                Err(error) => {
                    eprintln!("Publisher unavailable: {error}");
                    if let Ok(mut state) = state.lock() {
                        state.publisher_failed(now, &error);
                    }
                }
            },
            Action::StopPublisher => publisher.stop(),
            Action::StartCapture {
                generation,
                identity,
            } => {
                if capture.is_some() {
                    continue;
                }
                let callbacks = CaptureCallbacks {
                    state: state.clone(),
                    frame_tx: frame_tx.clone(),
                    start: start.clone(),
                };
                match Capture::start(true, generation, callbacks) {
                    Ok(active) => {
                        *capture = Some(active);
                        if let Ok(mut state) = state.lock() {
                            let _ = state.capture_started(generation, now, Ok(()));
                        }
                        println!("Capturing {identity}, generation {generation}");
                    }
                    Err(error) => {
                        eprintln!("Capture unavailable: {error}");
                        if let Ok(mut state) = state.lock() {
                            let _ = state.capture_start_failed(generation, now, &error);
                            let _ = state.capture_closed(Ok(()));
                        }
                    }
                }
            }
            Action::StopCapture => {
                if let Some(active) = capture.take() {
                    let result = active.stop();
                    if let Ok(mut state) = state.lock() {
                        let _ = state.capture_closed(result);
                    }
                }
            }
            Action::BeginReset => {
                eprintln!("jieli-bridge: reset requested in test-source mode; ignored");
            }
            Action::ReleaseSleepInhibitor => {}
        }
    }
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
    }
}
