//! Install and remove the user-owned files for the Rust camera bridge.
//!
//! The installer deliberately has no service-management mode. It refuses to
//! change files while the bridge unit is active and leaves service start/stop
//! to the administrator.

pub mod args;
pub mod environment;
pub mod fsutil;
pub mod install;
pub mod layout;
pub mod manifest;
