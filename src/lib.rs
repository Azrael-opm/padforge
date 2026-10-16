//! PadForge core: platform-neutral mapping engine, config and controller model.
pub mod config;
pub mod engine;
pub mod keys;
pub mod mouse_stick;
pub mod pad;
pub mod shared;

#[cfg(windows)]
pub mod win;
