#![windows_subsystem = "windows"]
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
include!("win_app.rs");
