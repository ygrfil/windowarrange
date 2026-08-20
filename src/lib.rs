#[cfg(not(target_os = "windows"))]
compile_error!("Table Arranger Control supports Windows only");

pub mod app;
pub mod config;
pub mod controller;
pub mod hotkeys;
pub mod identity;
pub mod layout;
pub mod logging;
pub mod model;
pub mod rng_overlay;
pub mod tray;
pub mod win32;
