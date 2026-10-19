use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jieli_core::frame::Frame;
use jieli_core::framing::FrameReader;

/// Latest-frame handoff between the stdin reader and the PipeWire process
/// callback. The slot keeps one item and drops the oldest, matching the live
/// `max-buffers=2 leaky=downstream` depth-1 publication contract.
#[derive(Debug)]
pub struct PublisherFeed {
    reader: FrameReader,
    latest: Option<FeedItem>,
    last_taken: u64,
    eof: bool,
    received: u64,
    placeholders: u64,
}

#[derive(Debug)]
struct FeedItem {
    frame: Frame,
    placeholder: bool,
    seq: u64,
}

impl PublisherFeed {
    pub fn new(start: Duration) -> Self {
        Self {
            reader: FrameReader::new(start),
            latest: None,
            last_taken: 0,
            eof: false,
            received: 0,
            placeholders: 0,
        }
    }

    /// Append bytes read from stdin; returns the number of complete frames.
    pub fn push(&mut self, now: Duration, chunk: &[u8]) -> usize {
        let frames = self.reader.push(now, chunk);
        for frame in &frames {
            self.received += 1;
            self.store(frame.clone(), false);
        }
        frames.len()
    }

    /// Emit a neutral fallback when the real frame path has starved.
    pub fn tick(&mut self, now: Duration) -> bool {
        match self.reader.tick(now) {
            Some(frame) => {
                self.placeholders += 1;
                self.store(frame, true);
                true
            }
            None => false,
        }
    }

    fn store(&mut self, frame: Frame, placeholder: bool) {
        let seq = self.latest.as_ref().map(|item| item.seq + 1).unwrap_or(1);
        self.latest = Some(FeedItem {
            frame,
            placeholder,
            seq,
        });
    }

    /// Newest item not yet handed to PipeWire.
    pub fn take_new(&mut self) -> Option<(Frame, bool)> {
        let item = self.latest.as_ref()?;
        if item.seq == self.last_taken {
            return None;
        }
        self.last_taken = item.seq;
        Some((item.frame.clone(), item.placeholder))
    }

    /// Latest item, even if already handed out; used to keep buffers flowing.
    pub fn latest(&self) -> Option<(Frame, bool)> {
        self.latest
            .as_ref()
            .map(|item| (item.frame.clone(), item.placeholder))
    }

    pub fn eof(&mut self) {
        self.reader.eof();
        self.eof = true;
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    pub fn next_deadline(&self) -> Duration {
        self.reader.next_deadline()
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    pub fn placeholders(&self) -> u64 {
        self.placeholders
    }
}

/// Read stdin until EOF or `quit`, feeding the shared publisher slot.
///
/// This runs on its own thread because the PipeWire loop must stay on the
/// thread that created the stream.
pub fn feed_stdin(feed: Arc<Mutex<PublisherFeed>>, quit: &'static AtomicBool) {
    feed_fd(0, feed, quit);
}

/// Same as `feed_stdin`, with an explicit fd for tests and diagnostics.
pub fn feed_fd(fd: libc::c_int, feed: Arc<Mutex<PublisherFeed>>, quit: &AtomicBool) {
    let start = Instant::now();
    let mut buffer = vec![0u8; 64 * 1024];

    while !quit.load(Ordering::Relaxed) {
        let now = start.elapsed();
        let deadline = match feed.lock() {
            Ok(feed) => feed.next_deadline(),
            Err(_) => now,
        };
        let timeout = deadline.saturating_sub(now);

        if poll_fd(fd, timeout) {
            let count =
                unsafe { libc::read(fd, buffer.as_mut_ptr() as *mut libc::c_void, buffer.len()) };
            if count < 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if let Ok(mut feed) = feed.lock() {
                    feed.eof();
                }
                return;
            }
            if count == 0 {
                if let Ok(mut feed) = feed.lock() {
                    feed.eof();
                }
                return;
            }
            let now = start.elapsed();
            if let Ok(mut feed) = feed.lock() {
                feed.push(now, &buffer[..count as usize]);
            }
        }

        let now = start.elapsed();
        if let Ok(mut feed) = feed.lock() {
            feed.tick(now);
        }
    }
}

fn poll_fd(fd: libc::c_int, timeout: Duration) -> bool {
    let milliseconds = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut descriptor, 1, milliseconds.max(0)) };
    // A closed pipe reports POLLHUP, typically without POLLIN. Treat HUP/ERR
    // as readable so the next read observes EOF and ends publication.
    result > 0 && (descriptor.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::*;
    use jieli_core::frame::FRAME_SIZE;

    fn private_frame(byte: u8) -> Vec<u8> {
        vec![byte; FRAME_SIZE]
    }

    fn pipe() -> (OwnedFd, std::fs::File) {
        let mut descriptors = [0; 2];
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        (reader, writer)
    }

    #[test]
    fn keeps_only_the_newest_frame() {
        let start = Duration::ZERO;
        let mut feed = PublisherFeed::new(start);
        assert!(feed.take_new().is_none());

        let mut chunk = private_frame(1);
        chunk.extend(private_frame(2));
        assert_eq!(feed.push(start, &chunk), 2);
        assert_eq!(feed.received(), 2);

        let (frame, placeholder) = feed.take_new().expect("newest frame");
        assert_eq!(frame.as_bytes()[0], 2);
        assert!(!placeholder);
        assert!(feed.take_new().is_none());
        assert_eq!(feed.latest().expect("latest").0.as_bytes()[0], 2);
    }

    #[test]
    fn placeholder_marks_the_item_and_keeps_cadence() {
        let start = Duration::ZERO;
        let mut feed = PublisherFeed::new(start);
        assert!(feed.tick(start));
        let (_, placeholder) = feed.take_new().expect("placeholder");
        assert!(placeholder);
        assert_eq!(feed.placeholders(), 1);

        let private = private_frame(7);
        assert_eq!(feed.push(start, &private), 1);
        let (frame, placeholder) = feed.take_new().expect("real frame");
        assert_eq!(frame.as_bytes()[0], 7);
        assert!(!placeholder);
    }

    #[test]
    fn eof_discards_partial_bytes_and_stops_frames() {
        let mut feed = PublisherFeed::new(Duration::ZERO);
        assert_eq!(feed.push(Duration::ZERO, b"private fragment"), 0);
        feed.eof();
        assert!(feed.is_eof());
        assert_eq!(feed.push(Duration::ZERO, &private_frame(0)), 0);
        assert!(!feed.tick(Duration::from_secs(1)));
    }

    #[test]
    fn eof_on_a_closed_pipe_ends_the_reader_and_keeps_the_frame() {
        let (reader, mut writer) = pipe();
        let feed = Arc::new(Mutex::new(PublisherFeed::new(Duration::ZERO)));
        let quit = AtomicBool::new(false);

        std::thread::scope(|scope| {
            let thread_feed = Arc::clone(&feed);
            scope.spawn(move || {
                feed_fd(reader.as_raw_fd(), thread_feed, &quit);
            });
            writer.write_all(&private_frame(9)).unwrap();
            drop(writer);
        });

        let feed = feed.lock().unwrap();
        assert!(feed.is_eof());
        assert_eq!(feed.received(), 1);
        assert!(feed.latest().is_some());
    }
}
