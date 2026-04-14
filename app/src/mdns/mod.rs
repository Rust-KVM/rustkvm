//! Multicast DNS (mDNS) service discovery module
//!
//! Provides mDNS service discovery functionality, allowing devices to automatically
//! discover each other on the local network.

pub mod config;
pub mod server;
pub mod utils;

pub use config::{MdnsListenOptions, MdnsOptions};
pub use server::Mdns;
