use std::fmt;
use std::sync::Arc;

use thiserror::Error;

pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;
pub const Y_SIZE: usize = WIDTH * HEIGHT;
pub const UV_WIDTH: usize = WIDTH / 2;
pub const UV_HEIGHT: usize = HEIGHT / 2;
pub const UV_SIZE: usize = UV_WIDTH * UV_HEIGHT;
pub const FRAME_SIZE: usize = Y_SIZE + 2 * UV_SIZE;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("invalid I420 frame size: expected {expected}, got {actual}")]
    WrongSize { expected: usize, actual: usize },
}

/// An immutable, fixed-size I420 frame.
///
/// The backing allocation is shared between clones, so the latest-frame handoff
/// does not copy 1,382,400 bytes. All constructors validate the size.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame(Arc<Vec<u8>>);

impl Frame {
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, FrameError> {
        if bytes.len() != FRAME_SIZE {
            return Err(FrameError::WrongSize {
                expected: FRAME_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self(Arc::new(bytes)))
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() != FRAME_SIZE {
            return Err(FrameError::WrongSize {
                expected: FRAME_SIZE,
                actual: bytes.len(),
            });
        }
        Ok(Self(Arc::new(bytes.to_vec())))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn y(&self) -> &[u8] {
        &self.0[..Y_SIZE]
    }

    pub fn u(&self) -> &[u8] {
        &self.0[Y_SIZE..Y_SIZE + UV_SIZE]
    }

    pub fn v(&self) -> &[u8] {
        &self.0[Y_SIZE + UV_SIZE..]
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// A constructed frame always has `FRAME_SIZE` bytes.
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn into_bytes(self) -> Vec<u8> {
        match Arc::try_unwrap(self.0) {
            Ok(bytes) => bytes,
            Err(shared) => shared.as_slice().to_vec(),
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame").field("len", &self.0.len()).finish()
    }
}

/// A one-element slot that keeps the newest item and drops the previous one.
///
/// This is the ownership half of the publication depth-1 contract. Returning
/// the displaced item lets callers account for drops explicitly.
#[derive(Debug, Default)]
pub struct LatestSlot<T> {
    item: Option<T>,
}

impl<T> LatestSlot<T> {
    pub fn new() -> Self {
        Self { item: None }
    }

    pub fn put(&mut self, item: T) -> Option<T> {
        self.item.replace(item)
    }

    pub fn take(&mut self) -> Option<T> {
        self.item.take()
    }

    pub fn peek(&self) -> Option<&T> {
        self.item.as_ref()
    }

    pub fn clear(&mut self) -> Option<T> {
        self.item.take()
    }

    pub fn is_empty(&self) -> bool {
        self.item.is_none()
    }

    pub fn len(&self) -> usize {
        usize::from(self.item.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with(byte: u8) -> Frame {
        Frame::from_vec(vec![byte; FRAME_SIZE]).expect("valid frame")
    }

    #[test]
    fn frame_size_matches_i420_contract() {
        assert_eq!(Y_SIZE, 921_600);
        assert_eq!(UV_SIZE, 230_400);
        assert_eq!(FRAME_SIZE, 1_382_400);
    }

    #[test]
    fn frame_splits_planes() {
        let mut bytes = vec![32; FRAME_SIZE];
        bytes[Y_SIZE..Y_SIZE + UV_SIZE].fill(64);
        bytes[Y_SIZE + UV_SIZE..].fill(128);
        let frame = Frame::from_vec(bytes).expect("valid frame");

        assert_eq!(frame.y().len(), Y_SIZE);
        assert_eq!(frame.u(), &[64][..].repeat(UV_SIZE));
        assert_eq!(frame.v(), &[128][..].repeat(UV_SIZE));
    }

    #[test]
    fn rejects_wrong_size() {
        let error = Frame::from_vec(vec![0; FRAME_SIZE - 1]).unwrap_err();
        assert_eq!(
            error,
            FrameError::WrongSize {
                expected: FRAME_SIZE,
                actual: FRAME_SIZE - 1
            }
        );
    }

    #[test]
    fn latest_slot_drops_oldest() {
        let mut slot = LatestSlot::new();
        assert!(slot.is_empty());
        assert_eq!(slot.put(frame_with(1)), None);
        let displaced = slot.put(frame_with(2)).expect("old frame");
        assert_eq!(displaced.as_bytes()[0], 1);
        assert_eq!(slot.len(), 1);
        assert_eq!(slot.peek().expect("current").as_bytes()[0], 2);
        assert_eq!(slot.take().expect("taken").as_bytes()[0], 2);
        assert!(slot.is_empty());
    }
}
