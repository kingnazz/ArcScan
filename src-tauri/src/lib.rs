mod commands;
mod db;
mod oui;
mod scanner;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let path = commands::db_path(&handle).expect("resolve ArcScan data directory");
            let conn = db::open(&path).expect("open ArcScan scan-history database");
            app.manage(AppState::new(conn));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_network,
            commands::cancel_scan,
            commands::list_scans,
            commands::get_scan_hosts,
            commands::delete_scan,
            commands::launch_action,
            commands::write_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ArcScan");
}
