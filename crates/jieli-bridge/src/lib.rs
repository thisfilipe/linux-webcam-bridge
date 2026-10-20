//! Supervisor for the Linux webcam bridge.
//!
//! The pure argument parser builds without GStreamer. The `native` feature adds
//! the GStreamer capture and the publisher process manager; the test-source mode
//! exercises the full capture -> stdin -> PipeWire path without the camera.

pub mod args;
pub mod device;
pub mod reset;

#[cfg(feature = "native")]
pub mod capture;

#[cfg(feature = "native")]
pub mod logind;

#[cfg(feature = "native")]
pub mod publisher;
