//! Native publisher for the Linux webcam bridge.
//!
//! The binary reads raw I420 frames from stdin, emits a neutral placeholder
//! after 250 ms of starvation, publishes a `Video/Source` node through PipeWire
//! and (in a later step) feeds the v4l2loopback device. The pure modules here
//! compile and test without the PipeWire development files; the `native`
//! feature enables the real stream.

pub mod args;
pub mod feed;
pub mod v4l2;

#[cfg(feature = "native")]
pub mod native;
