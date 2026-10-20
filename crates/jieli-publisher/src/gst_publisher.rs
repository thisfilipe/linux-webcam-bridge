//! GStreamer publisher path.
//!
//! This intentionally mirrors the GStreamer publisher that was validated with
//! Snapshot before the native PipeWire publisher was introduced. Snapshot keeps
//! a PipeWire consumer alive across minimisation; a GStreamer `pipewiresink`
//! source survives that consumer lifecycle in the field, while the raw
//! `pipewire-rs` provide stream does not.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::glib::{self, ControlFlow};
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use jieli_core::frame::WIDTH;
use jieli_core::placeholder::placeholder;

use crate::native::NativeOptions;

const FRAME_INTERVAL: Duration = Duration::from_nanos(33_333_333);

/// Keep the producer contract identical to the validated reference publisher. In
/// particular, `use-bufferpool=false` avoids handing Snapshot a consumer-owned
/// buffer pool that it does not recycle after the window loses focus.
fn pipeline_description() -> &'static str {
    concat!(
        "appsrc name=frames is-live=true do-timestamp=true format=time ",
        "block=false max-buffers=2 max-bytes=0 max-time=0 leaky-type=downstream ",
        "caps=\"video/x-raw,format=I420,width=1280,height=720,framerate=30/1\" ",
        "! queue leaky=downstream max-size-buffers=2 max-size-bytes=0 max-size-time=0 ",
        "! pipewiresink name=pw mode=provide use-bufferpool=false sync=false async=false ",
        "enable-last-sample=false"
    )
}

/// Prefer the locally built PipeWire plugin when it is installed. The stock
/// plugin retains a busy buffer slot after an unclean consumer; the local
/// build is the version that was validated with Snapshot.
pub fn prefer_local_pipewire_plugin() {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let directory = PathBuf::from(home).join(".local/lib/jieli-gstreamer");
    if !directory.is_dir() {
        return;
    }
    let mut paths: Vec<PathBuf> = std::env::var_os("GST_PLUGIN_PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    if !paths.iter().any(|path| path == &directory) {
        paths.insert(0, directory);
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: the publisher binary calls this before spawning its reader/V4L2
        // threads and before GStreamer initialization.
        unsafe { std::env::set_var("GST_PLUGIN_PATH", joined) };
    }
}

fn stream_properties(node_name: &str, description: &str) -> gst::Structure {
    gst::Structure::builder("props")
        .field("media.class", "Video/Source")
        .field("media.role", "Camera")
        .field("node.name", node_name)
        .field("node.description", description)
        .field("node.virtual", true)
        .field("node.pause-on-idle", false)
        .field("priority.session", 1000i32)
        .build()
}

fn monotonic_millis() -> u64 {
    let mut timestamp: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timestamp) };
    timestamp.tv_sec as u64 * 1000 + timestamp.tv_nsec as u64 / 1_000_000
}

/// Publish stdin frames through GStreamer's `pipewiresink`.
pub fn run(options: NativeOptions<'_>) -> Result<(), String> {
    // `prefer_local_pipewire_plugin()` runs in the publisher entry point before
    // the reader/V4L2 threads start; never mutate the environment here.
    gst::init().map_err(|error| format!("GStreamer initialization failed: {error}"))?;

    let pipeline = gst::parse::launch(pipeline_description())
        .map_err(|error| format!("publisher pipeline construction failed: {error}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "publisher construction did not return a pipeline".to_string())?;

    let source = pipeline
        .by_name("frames")
        .ok_or_else(|| "publisher is missing its appsrc".to_string())?
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| "frames is not an appsrc".to_string())?;
    let sink = pipeline
        .by_name("pw")
        .ok_or_else(|| "publisher is missing its pipewiresink".to_string())?;
    sink.set_property(
        "stream-properties",
        stream_properties(options.node_name, options.description),
    );

    let bus = pipeline
        .bus()
        .ok_or_else(|| "publisher has no bus".to_string())?;
    if pipeline.set_state(gst::State::Playing).is_err() {
        return Err("publisher failed to enter PLAYING".to_string());
    }

    println!(
        "publisher pid={}, plugin=gstreamer-pipewiresink, caps={}x{}, format=I420, fps=30",
        std::process::id(),
        WIDTH,
        jieli_core::frame::HEIGHT,
    );

    let main_loop = glib::MainLoop::new(None, false);
    let main_loop_for_timer = main_loop.clone();
    let source_for_timer = source.clone();
    let bus_for_timer = bus.clone();
    let feed = options.feed;
    let quit = options.quit;
    let failed = options.failed;
    let debug_requested = options.debug_requested;
    let debug_until_ms = options.debug_until_ms;
    let published = AtomicU64::new(0);
    let stream_start = Instant::now();
    let mut ticks = 0u64;

    let timer = glib::timeout_add_local(FRAME_INTERVAL, move || {
        if quit.load(Ordering::Relaxed) {
            main_loop_for_timer.quit();
            return ControlFlow::Break;
        }

        while let Some(message) = bus_for_timer.pop() {
            match message.view() {
                gst::MessageView::Error(error) => {
                    eprintln!(
                        "publisher: {}; {}",
                        error.error(),
                        error.debug().unwrap_or_default()
                    );
                    failed.store(true, Ordering::SeqCst);
                    quit.store(true, Ordering::SeqCst);
                    main_loop_for_timer.quit();
                    return ControlFlow::Break;
                }
                gst::MessageView::Eos(_) => {
                    main_loop_for_timer.quit();
                    return ControlFlow::Break;
                }
                _ => {}
            }
        }

        let (item, eof) = match feed.lock() {
            Ok(mut feed) => {
                let eof = feed.is_eof();
                (feed.take_new().or_else(|| feed.latest()), eof)
            }
            Err(_) => (None, false),
        };

        if eof {
            let _ = source_for_timer.end_of_stream();
            main_loop_for_timer.quit();
            return ControlFlow::Break;
        }

        let frame = match item {
            Some((frame, _placeholder)) => frame,
            None => placeholder().clone(),
        };
        let mut buffer = gst::Buffer::from_slice(frame.as_bytes().to_vec());
        if let Some(buffer) = buffer.get_mut() {
            let timestamp = gst::ClockTime::from_nseconds(stream_start.elapsed().as_nanos() as u64);
            buffer.set_pts(timestamp);
            buffer.set_dts(timestamp);
            buffer.set_duration(gst::ClockTime::from_nseconds(
                FRAME_INTERVAL.as_nanos() as u64
            ));
        }
        if let Err(error) = source_for_timer.push_buffer(buffer) {
            eprintln!("publisher: appsrc push failed: {error:?}");
            failed.store(true, Ordering::SeqCst);
            quit.store(true, Ordering::SeqCst);
            main_loop_for_timer.quit();
            return ControlFlow::Break;
        }
        published.fetch_add(1, Ordering::Relaxed);

        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(300) {
            let (received, placeholders) = feed
                .lock()
                .map(|feed| (feed.received(), feed.placeholders()))
                .unwrap_or((0, 0));
            println!(
                "publisher received={received}, placeholder={placeholders}, frames={}",
                published.load(Ordering::Relaxed)
            );
        }
        if debug_requested.swap(false, Ordering::Relaxed) {
            debug_until_ms.store(monotonic_millis() + 20_000, Ordering::Relaxed);
        }
        if debug_until_ms.load(Ordering::Relaxed) > monotonic_millis() && ticks.is_multiple_of(30) {
            eprintln!(
                "publisher: debug frames={}",
                published.load(Ordering::Relaxed)
            );
        }

        ControlFlow::Continue
    });

    main_loop.run();
    let _ = timer;
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_matches_the_validated_gstreamer_contract() {
        let description = pipeline_description();
        assert!(description.contains("appsrc"));
        assert!(description.contains("do-timestamp=true"));
        assert!(description.contains("mode=provide"));
        assert!(description.contains("use-bufferpool=false"));
        assert!(description.contains("pipewiresink"));
    }

    #[test]
    fn stream_properties_keep_the_stable_node_identity() {
        let _ = gst::init();
        let properties = stream_properties("jieli_720p30", "Jieli 720p30");
        assert_eq!(properties.get::<&str>("node.name").unwrap(), "jieli_720p30");
        assert_eq!(
            properties.get::<&str>("node.description").unwrap(),
            "Jieli 720p30"
        );
        assert!(!properties.get::<bool>("node.pause-on-idle").unwrap());
    }
}
