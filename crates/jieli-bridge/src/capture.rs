use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use jieli_core::frame::Frame;
use jieli_core::state::{BridgeState, FrameOutcome};

pub const CAPS: &str = "video/x-raw,format=I420,width=1280,height=720,framerate=30/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSource {
    Test,
    V4l2(PathBuf),
}

impl CaptureSource {
    pub fn description(&self) -> String {
        match self {
            Self::Test => format!("videotestsrc is-live=true pattern=ball ! {CAPS}"),
            Self::V4l2(device) => format!(
                "v4l2src device=\"{}\" io-mode=mmap do-timestamp=true ! \
                 image/jpeg,width=1280,height=720,framerate=30/1 ! \
                 jpegparse ! jpegdec ! videoconvert ! {CAPS}",
                device.display()
            ),
        }
    }
}

#[derive(Clone)]
pub struct CaptureCallbacks {
    pub state: Arc<Mutex<BridgeState>>,
    pub frame_tx: Arc<Mutex<Option<SyncSender<Frame>>>>,
    pub start: Arc<Instant>,
}

pub struct Capture {
    pipeline: gst::Pipeline,
    bus: gst::Bus,
}

impl Capture {
    pub fn start(
        source: &CaptureSource,
        generation: u64,
        callbacks: CaptureCallbacks,
    ) -> Result<Self, String> {
        let source_description = source.description();
        let description = format!(
            "{source_description} ! appsink name=frames emit-signals=true sync=false \
             max-buffers=2 drop=true enable-last-sample=false"
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|error| format!("capture construction failed: {error}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "capture construction did not return a pipeline".to_string())?;
        let sink = pipeline
            .by_name("frames")
            .ok_or_else(|| "capture is missing its appsink".to_string())?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "frames is not an appsink".to_string())?;

        let state = callbacks.state;
        let frame_tx = callbacks.frame_tx;
        let start = callbacks.start;
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let Ok(sample) = sink.pull_sample() else {
                        return Err(gst::FlowError::Eos);
                    };
                    let Some(buffer) = sample.buffer() else {
                        return Err(gst::FlowError::Error);
                    };
                    let Ok(mapped) = buffer.map_readable() else {
                        return Err(gst::FlowError::Error);
                    };
                    let Ok(frame) = Frame::from_slice(mapped.as_slice()) else {
                        return Err(gst::FlowError::Error);
                    };

                    let now = start.elapsed().as_secs_f64();
                    let accepted = match state.lock() {
                        Ok(mut state) => matches!(
                            state.on_frame(generation, now, frame.clone()),
                            FrameOutcome::Accepted { .. }
                        ),
                        Err(_) => false,
                    };
                    if accepted
                        && let Ok(slot) = frame_tx.lock()
                        && let Some(sender) = slot.as_ref()
                    {
                        let _ = sender.try_send(frame);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let bus = pipeline
            .bus()
            .ok_or_else(|| "capture has no bus".to_string())?;
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| format!("capture failed to enter PLAYING: {error}"))?;

        Ok(Self { pipeline, bus })
    }

    /// Drain the bus without blocking; return the first error/EOS reason.
    pub fn poll_error(&self) -> Option<String> {
        while let Some(message) = self.bus.pop() {
            match message.view() {
                gst::MessageView::Error(error) => {
                    return Some(format!("capture: {}", error.error()));
                }
                gst::MessageView::Eos(_) => {
                    return Some("capture: end of stream".to_string());
                }
                _ => {}
            }
        }
        None
    }

    pub fn stop(self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|error| format!("capture teardown failed: {error}"))?;
        let current = self.pipeline.current_state();
        if current != gst::State::Null {
            return Err("capture teardown did not reach NULL".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_uses_videotestsrc_and_i420() {
        let description = CaptureSource::Test.description();
        assert!(description.contains("videotestsrc is-live=true"));
        assert!(description.contains(CAPS));
    }

    #[test]
    fn real_source_matches_the_live_mjpeg_decode_chain() {
        let description =
            CaptureSource::V4l2(PathBuf::from("/dev/v4l/by-id/jieli-video0")).description();
        assert!(description.contains("v4l2src device=\"/dev/v4l/by-id/jieli-video0\""));
        assert!(description.contains("io-mode=mmap"));
        assert!(description.contains("image/jpeg,width=1280,height=720,framerate=30/1"));
        assert!(description.contains("jpegparse ! jpegdec ! videoconvert"));
        assert!(description.contains(CAPS));
    }
}
