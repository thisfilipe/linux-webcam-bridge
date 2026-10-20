//! Native publisher for the Linux webcam bridge.
//!
//! The binary reads raw I420 frames from stdin, emits a neutral placeholder
//! after 250 ms of starvation, publishes a `Video/Source` node through
//! GStreamer's `pipewiresink` and feeds the v4l2loopback device directly. The
//! pure modules here compile and test without the GStreamer/PipeWire
//! development files; the `native` feature enables the real publisher.

pub mod args;
pub mod feed;
pub mod v4l2;

#[cfg(feature = "native")]
pub mod gst_publisher;

#[cfg(feature = "native")]
pub mod native;

#[cfg(feature = "native")]
pub mod v4l2_writer;
