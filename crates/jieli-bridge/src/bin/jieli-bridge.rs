use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jieli_bridge::args::{ArgsError, USAGE, parse_args};
use jieli_bridge::capture::{Capture, CaptureCallbacks, CaptureSource};
use jieli_bridge::device;
use jieli_bridge::logind::{SleepEvent, SleepMonitor};
use jieli_bridge::publisher::PublisherManager;
use jieli_bridge::reset::ResetManager;
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

    if let Err(error) = gstreamer::init() {
        eprintln!("jieli-bridge: GStreamer initialization failed: {error}");
        std::process::exit(1);
    }

    let source = if arguments.test_source {
        CaptureSource::Test
    } else {
        CaptureSource::V4l2(PathBuf::from(device::DEVICE_PATH))
    };
    let description = if arguments.test_source {
        "Jieli validation"
    } else {
        "Jieli 720p30"
    };
    let mut sleep_monitor = if arguments.test_source {
        None
    } else {
        Some(SleepMonitor::start())
    };

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
    let mut reset = ResetManager::new();
    let mut capture: Option<Capture> = None;

    loop {
        let now = start.elapsed().as_secs_f64();

        if let Some(result) = reset.poll() {
            let actions = state
                .lock()
                .map(|mut state| state.reset_finished(now, result))
                .unwrap_or_default();
            apply_actions(
                actions,
                &state,
                &mut capture,
                &mut publisher,
                &mut reset,
                sleep_monitor.as_ref(),
                &frame_tx,
                &start,
                &source,
                description,
                &arguments.node_name,
                now,
            );
        }

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
                &mut reset,
                sleep_monitor.as_ref(),
                &frame_tx,
                &start,
                &source,
                description,
                &arguments.node_name,
                now,
            );
        }

        if publisher.poll_exit()
            && let Ok(mut state) = state.lock()
        {
            state.publisher_exited(now);
        }

        let identity = if arguments.test_source {
            Some("synthetic".to_string())
        } else {
            device::identity(Path::new(device::DEVICE_PATH))
        };
        let inhibited = sleep_monitor
            .as_ref()
            .map(SleepMonitor::inhibited)
            .unwrap_or(true);
        let actions = match state.lock() {
            Ok(mut state) => state.tick(now, identity.as_deref(), inhibited),
            Err(_) => break,
        };
        apply_actions(
            actions,
            &state,
            &mut capture,
            &mut publisher,
            &mut reset,
            sleep_monitor.as_ref(),
            &frame_tx,
            &start,
            &source,
            description,
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

        let event = match sleep_monitor.as_mut() {
            Some(monitor) => monitor.recv_timeout(Duration::from_secs(1)),
            None => {
                std::thread::sleep(Duration::from_secs(1));
                None
            }
        };
        if let Some(event) = event {
            let now = start.elapsed().as_secs_f64();
            match event {
                SleepEvent::Prepare => {
                    let actions = state
                        .lock()
                        .map(|mut state| state.prepare_sleep())
                        .unwrap_or_default();
                    apply_actions(
                        actions,
                        &state,
                        &mut capture,
                        &mut publisher,
                        &mut reset,
                        sleep_monitor.as_ref(),
                        &frame_tx,
                        &start,
                        &source,
                        description,
                        &arguments.node_name,
                        now,
                    );
                }
                SleepEvent::Resume => {
                    let actions = state
                        .lock()
                        .map(|mut state| state.resume(now))
                        .unwrap_or_default();
                    apply_actions(
                        actions,
                        &state,
                        &mut capture,
                        &mut publisher,
                        &mut reset,
                        sleep_monitor.as_ref(),
                        &frame_tx,
                        &start,
                        &source,
                        description,
                        &arguments.node_name,
                        now,
                    );
                }
                SleepEvent::Deadline => {
                    if let Ok(mut state) = state.lock() {
                        state.sleep_deadline();
                    }
                }
                SleepEvent::Inhibited => {
                    if let Ok(mut state) = state.lock() {
                        state.set_sleep_monitor_error(None);
                    }
                }
                SleepEvent::Error(error) => {
                    eprintln!("{error}");
                    if let Ok(mut state) = state.lock() {
                        state.set_sleep_monitor_error(Some(error));
                    }
                }
            }
        }
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
        &mut reset,
        sleep_monitor.as_ref(),
        &frame_tx,
        &start,
        &source,
        description,
        &arguments.node_name,
        start.elapsed().as_secs_f64(),
    );
    if let Some(active) = capture.take() {
        let _ = active.stop();
    }
    publisher.stop();
    if let Some(monitor) = sleep_monitor.as_ref() {
        monitor.release();
    }
    let _ = std::fs::remove_file(&arguments.state_file);
}

#[allow(clippy::too_many_arguments)]
fn apply_actions(
    actions: Vec<Action>,
    state: &Arc<Mutex<BridgeState>>,
    capture: &mut Option<Capture>,
    publisher: &mut PublisherManager,
    reset: &mut ResetManager,
    sleep: Option<&SleepMonitor>,
    frame_tx: &Arc<Mutex<Option<SyncSender<Frame>>>>,
    start: &Arc<Instant>,
    source: &CaptureSource,
    description: &str,
    node_name: &str,
    now: f64,
) {
    for action in actions {
        match action {
            Action::StartPublisher => match publisher.start(node_name, description) {
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
                match Capture::start(source, generation, callbacks) {
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
                if reset.is_running() {
                    continue;
                }
                reset.begin();
                println!("Resetting USB");
            }
            Action::ReleaseSleepInhibitor => {
                if let Some(monitor) = sleep {
                    monitor.release();
                }
            }
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
