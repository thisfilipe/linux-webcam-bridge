use std::time::Duration;

use crate::frame::{FRAME_SIZE, Frame};
use crate::placeholder::build_placeholder;

pub const STARVATION: Duration = Duration::from_millis(250);
pub const FALLBACK_INTERVAL: Duration = Duration::from_nanos(33_333_333);

/// Accumulates partial pipe reads and emits real frames or neutral fallbacks.
///
/// The caller supplies time, so the state machine is deterministic and does
/// not sleep or perform I/O. EOF discards any private partial bytes; they are
/// never replayed as a real frame.
pub struct FrameReader {
    pending: Vec<u8>,
    last_real: Option<Duration>,
    next_fallback: Duration,
    fallback: Frame,
    eof: bool,
}

impl FrameReader {
    pub fn new(start: Duration) -> Self {
        Self {
            pending: Vec::with_capacity(FRAME_SIZE),
            last_real: None,
            next_fallback: start,
            fallback: build_placeholder(),
            eof: false,
        }
    }

    /// Append bytes read from the pipe. Returns every complete real frame.
    ///
    /// An empty chunk is EOF and clears the pending partial frame.
    pub fn push(&mut self, now: Duration, chunk: &[u8]) -> Vec<Frame> {
        if self.eof {
            return Vec::new();
        }
        if chunk.is_empty() {
            self.eof = true;
            self.pending.clear();
            return Vec::new();
        }

        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while self.pending.len() >= FRAME_SIZE {
            let frame = Frame::from_slice(&self.pending[..FRAME_SIZE])
                .expect("pending buffer always contains a full frame");
            self.pending.drain(..FRAME_SIZE);
            self.last_real = Some(now);
            self.next_fallback = now + STARVATION;
            frames.push(frame);
        }
        frames
    }

    /// Emit a neutral frame when the live frame path has starved.
    ///
    /// The initial fallback is allowed before the first real frame. After a
    /// real frame, fallbacks start 250 ms later and repeat at 30 fps.
    pub fn tick(&mut self, now: Duration) -> Option<Frame> {
        if self.eof {
            return None;
        }
        let starving = match self.last_real {
            None => true,
            Some(last_real) => now >= last_real + STARVATION,
        };
        if starving && now >= self.next_fallback {
            self.next_fallback = now + FALLBACK_INTERVAL;
            Some(self.fallback.clone())
        } else {
            None
        }
    }

    pub fn eof(&mut self) {
        self.eof = true;
        self.pending.clear();
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn has_real_frame(&self) -> bool {
        self.last_real.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FRAME_SIZE;

    fn private_frame(byte: u8) -> Vec<u8> {
        vec![byte; FRAME_SIZE]
    }

    #[test]
    fn partial_reads_preserve_bytes_then_neutral_never_replays() {
        let start = Duration::ZERO;
        let private = private_frame(b'X');
        let mut reader = FrameReader::new(start);

        assert!(reader.tick(start).is_some(), "startup placeholder");
        assert!(reader.push(start, &private[..100]).is_empty());
        assert_eq!(reader.pending_len(), 100);

        let real = reader.push(start, &private[100..]);
        assert_eq!(real.len(), 1);
        assert_eq!(real[0].as_bytes(), private.as_slice());
        assert!(reader.tick(start).is_none());

        let due = start + STARVATION;
        let neutral = reader.tick(due).expect("neutral after starvation");
        assert_ne!(neutral.as_bytes(), private.as_slice());

        let mut placeholders = 0;
        let mut now = due;
        for _ in 0..30 {
            now += FALLBACK_INTERVAL;
            assert!(reader.tick(now).is_some());
            placeholders += 1;
        }
        assert_eq!(placeholders, 30);
        assert!(now.abs_diff(due + Duration::from_secs(1)) <= Duration::from_nanos(10));
    }

    #[test]
    fn eof_discards_partial_private_bytes() {
        let start = Duration::ZERO;
        let mut reader = FrameReader::new(start);
        assert!(reader.push(start, b"private fragment").is_empty());
        assert_eq!(reader.pending_len(), 16);
        reader.eof();
        assert!(reader.is_eof());
        assert_eq!(reader.pending_len(), 0);
        assert!(reader.tick(start).is_none());
        assert!(reader.push(start, &private_frame(0)).is_empty());
    }

    #[test]
    fn empty_chunk_is_eof() {
        let mut reader = FrameReader::new(Duration::ZERO);
        assert!(reader.push(Duration::ZERO, &[]).is_empty());
        assert!(reader.is_eof());
    }

    #[test]
    fn multiple_frames_in_one_chunk_are_emitted() {
        let mut reader = FrameReader::new(Duration::ZERO);
        let mut chunk = private_frame(1);
        chunk.extend(private_frame(2));
        let frames = reader.push(Duration::from_secs(1), &chunk);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_bytes()[0], 1);
        assert_eq!(frames[1].as_bytes()[0], 2);
        assert_eq!(reader.pending_len(), 0);
    }
}
