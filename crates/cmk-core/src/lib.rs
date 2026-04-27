pub mod config;
pub mod error;
pub mod manifest;
pub mod platform;
pub mod process;
pub mod store;
pub mod version;

pub use config::Config;
pub use error::{Error, Result};
pub use manifest::{Manifest, Package, Platform as PlatformEntry, Release};
pub use platform::current_platform;
pub use process::{completing_read, confirm, wait_with_cancel};
pub use store::{InstalledIndex, InstalledPackage, InstalledVersion, Store};
