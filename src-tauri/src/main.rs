// Prevents an additional console window on Windows in release. This is a GUI
// app; child processes it spawns are separately suppressed with CREATE_NO_WINDOW.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    arcscan_lib::run()
}
