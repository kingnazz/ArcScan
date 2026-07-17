mod commands;
mod db;
mod ipparse;
mod netinfo;
mod oui;
mod scanner;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Open the scan-history database under the app data directory.
            let dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("no app data dir: {e}"))?;
            let db_path = dir.join("arcscan.db");
            let database = db::Db::open(&db_path).map_err(|e| format!("db open failed: {e}"))?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan_network,
            commands::save_scan,
            commands::list_scans,
            commands::get_scan,
            commands::delete_scan,
            commands::last_scan_ips,
            commands::detect_networks,
            commands::save_text,
            commands::wake_on_lan,
            commands::open_smb,
            commands::open_web,
            commands::open_rdp,
            commands::open_ssh,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ArcScan");
}
