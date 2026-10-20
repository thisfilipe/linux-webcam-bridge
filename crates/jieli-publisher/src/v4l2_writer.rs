use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jieli_core::frame::{Frame, HEIGHT, WIDTH};
use jieli_core::placeholder::placeholder;
use v4l::buffer::Type;
use v4l::format::field::FieldOrder;
use v4l::format::{Colorspace, Format, FourCC};
use v4l::io::mmap::Stream as MmapStream;
use v4l::io::traits::OutputStream;
use v4l::prelude::Device;
use v4l::video::Output;

use crate::feed::PublisherFeed;
use crate::v4l2;

const FRAME_INTERVAL: Duration = Duration::from_nanos(33_333_333);
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_DELAY: Duration = Duration::from_secs(30);
const BUFFER_COUNT: u32 = 8;
const STATS_INTERVAL: u64 = 300;

/// Feed the optional v4l2loopback device on its own thread.
///
/// A missing or broken loopback must never stop the PipeWire stream, so all
/// errors are logged and retried here instead of being propagated.
pub fn spawn(feed: Arc<Mutex<PublisherFeed>>, quit: &'static AtomicBool) {
    std::thread::spawn(move || supervise(feed, quit));
}

fn supervise(feed: Arc<Mutex<PublisherFeed>>, quit: &'static AtomicBool) {
    let sysfs = Path::new(v4l2::SYSFS_VIDEO);
    let dev_root = Path::new(v4l2::DEV_ROOT);
    let virtual_root = Path::new(v4l2::VIRTUAL_VIDEO_ROOT);

    while !quit.load(Ordering::Relaxed) {
        let Some(device) = v4l2::discover_device(sysfs, dev_root, virtual_root) else {
            sleep_interruptible(DEVICE_CHECK_INTERVAL, quit);
            continue;
        };

        match Writer::open(&device) {
            Ok(mut writer) => {
                println!("V4L2 output: {}, 1280x720 I420 30 fps", device.display());
                match writer.run(&feed, quit) {
                    Ok(()) => sleep_interruptible(DEVICE_CHECK_INTERVAL, quit),
                    Err(error) => {
                        eprintln!("V4L2 output unavailable: {error}; PipeWire continues");
                        sleep_interruptible(RETRY_DELAY, quit);
                    }
                }
            }
            Err(error) => {
                eprintln!("V4L2 output unavailable: {error}; PipeWire continues");
                sleep_interruptible(RETRY_DELAY, quit);
            }
        }
    }
}

struct Writer {
    stream: MmapStream<'static>,
    device: PathBuf,
    pushed: u64,
}

impl Writer {
    fn open(device: &Path) -> Result<Self, String> {
        let handle = Device::with_path(device)
            .map_err(|error| format!("open {}: {error}", device.display()))?;

        let mut format = output_format();
        format.field_order = FieldOrder::Progressive;
        format.colorspace = Colorspace::SMPTE170M;
        let negotiated = Output::set_format(&handle, &format)
            .map_err(|error| format!("set format on {}: {error}", device.display()))?;
        if negotiated.width != WIDTH as u32
            || negotiated.height != HEIGHT as u32
            || negotiated.fourcc != format.fourcc
        {
            return Err(format!(
                "{} negotiated {}x{} {}, expected {}x{} YU12",
                device.display(),
                negotiated.width,
                negotiated.height,
                negotiated.fourcc,
                WIDTH,
                HEIGHT
            ));
        }

        let mut stream: MmapStream<'static> =
            MmapStream::with_buffers(&handle, Type::VideoOutput, BUFFER_COUNT)
                .map_err(|error| format!("allocate buffers on {}: {error}", device.display()))?;
        stream.set_timeout(Duration::from_secs(1));
        drop(handle);

        Ok(Self {
            stream,
            device: device.to_path_buf(),
            pushed: 0,
        })
    }

    fn run(&mut self, feed: &Arc<Mutex<PublisherFeed>>, quit: &AtomicBool) -> Result<(), String> {
        let mut next_frame_at = Instant::now();
        let mut last_device_check = Instant::now();

        while !quit.load(Ordering::Relaxed) {
            if last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
                last_device_check = Instant::now();
                let discovered = v4l2::discover_device(
                    Path::new(v4l2::SYSFS_VIDEO),
                    Path::new(v4l2::DEV_ROOT),
                    Path::new(v4l2::VIRTUAL_VIDEO_ROOT),
                );
                if discovered.as_deref() != Some(self.device.as_path()) {
                    return Ok(());
                }
            }

            let now = Instant::now();
            if next_frame_at > now {
                sleep_interruptible(next_frame_at - now, quit);
            }
            let now = Instant::now();
            next_frame_at = if next_frame_at < now {
                now + FRAME_INTERVAL
            } else {
                next_frame_at + FRAME_INTERVAL
            };

            let frame = next_frame(feed);
            let (buffer, meta) = OutputStream::next(&mut self.stream)
                .map_err(|error| format!("write frame to {}: {error}", self.device.display()))?;
            if buffer.len() < frame.len() {
                return Err(format!(
                    "{} buffer is {} bytes, frame is {} bytes",
                    self.device.display(),
                    buffer.len(),
                    frame.len()
                ));
            }
            buffer[..frame.len()].copy_from_slice(frame.as_bytes());
            meta.bytesused = frame.len() as u32;
            meta.field = FieldOrder::Progressive as u32;

            self.pushed += 1;
            if self.pushed.is_multiple_of(STATS_INTERVAL) {
                println!("V4L2 pushed={}", self.pushed);
            }
        }
        Ok(())
    }
}

/// The v4l2loopback output format must match the publisher's I420 contract.
fn output_format() -> Format {
    Format::new(WIDTH as u32, HEIGHT as u32, FourCC::new(b"YU12"))
}

fn next_frame(feed: &Arc<Mutex<PublisherFeed>>) -> Frame {
    let chosen = match feed.lock() {
        Ok(mut feed) => feed.take_new().or_else(|| feed.latest()).map(|item| item.0),
        Err(_) => None,
    };
    chosen.unwrap_or_else(|| placeholder().clone())
}

fn sleep_interruptible(duration: Duration, quit: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while !quit.load(Ordering::Relaxed) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_is_yu12_720p() {
        let format = output_format();
        assert_eq!(format.width, 1280);
        assert_eq!(format.height, 720);
        assert_eq!(format.fourcc, FourCC::new(b"YU12"));
    }

    #[test]
    fn missing_device_fails_without_panicking() {
        assert!(Writer::open(Path::new("/nonexistent/jieli-video")).is_err());
    }

    #[test]
    fn empty_feed_uses_the_neutral_placeholder() {
        let feed = Arc::new(Mutex::new(PublisherFeed::new(Duration::ZERO)));
        assert_eq!(next_frame(&feed), placeholder().clone());
    }
}
