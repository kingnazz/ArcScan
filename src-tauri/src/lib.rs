mod commands;
mod db;
mod discovery;
mod inventory;
mod ipparse;
mod netinfo;
mod oui;
mod portable;
mod ports;
mod runtime;
mod scanner;
mod signature;
mod startup;

use tauri::Manager;

use runtime::RuntimePaths;

// The portable edition exists partly to *not* contain the installer updater.
// Enabling both features would produce a binary that reports "portable" while
// carrying an install-and-relaunch path, which is exactly the thing the release
// promises cannot happen -- so it is a build error rather than a runtime check.
#[cfg(all(feature = "portable", feature = "installed-updater"))]
compile_error!(
    "the `portable` and `installed-updater` features are mutually exclusive: build the \
     portable edition with --no-default-features --features portable"
);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The Windows cleanup helper is the same Portable executable, but it must
    // never create a Tauri app, database, WebView or second session. Intercept
    // its private, path-free invocation before ordinary Portable startup.
    #[cfg(feature = "portable")]
    if portable::run_cleanup_helper_if_requested() {
        return;
    }

    // Portable startup happens before Tauri does.
    //
    // The Portable temp session needs no app handle, and creating it first
    // means failure never reaches a window, WebView, database or AppData path.
    let portable = match startup::portable_startup() {
        Ok(portable) => portable,
        Err(error) => {
            startup::report_fatal(&error);
            std::process::exit(1);
        }
    };
    #[cfg(feature = "portable")]
    let portable_cleanup = portable
        .as_ref()
        .expect("a Portable build always creates a Portable session")
        .session
        .cleanup_handle();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    // Desktop auto-updater (+ process plugin for relaunch after install). Not
    // compiled into a portable build at all: see the feature documentation in
    // Cargo.toml and Phase 15 of docs/PORTABLE-ARCHITECTURE.md.
    #[cfg(all(desktop, feature = "installed-updater"))]
    {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }

    let app = builder
        .setup(move |app| {
            // Where this edition keeps its data. The installed edition resolves
            // the same application-data directory it always has; the portable
            // edition uses the unique temp layout startup already claimed.
            let paths = match &portable {
                Some(portable) => RuntimePaths::portable(&portable.session.layout),
                None => RuntimePaths::installed(
                    app.path()
                        .app_data_dir()
                        .map_err(|e| format!("no app data dir: {e}"))?,
                ),
            };

            let database = db::Db::open(&paths.database_path).map_err(|e| {
                // In portable mode a database that will not open is a portable
                // failure with portable wording, not a generic panic -- and
                // still not a reason to try somewhere else.
                if let Some(portable) = &portable {
                    let error = runtime::PortableError::DatabaseUnavailable {
                        path: portable.session.layout.database_path.display().to_string(),
                        detail: e.clone(),
                    };
                    startup::report_fatal(&error);
                    std::process::exit(1);
                }
                format!("db open failed: {e}")
            })?;
            app.manage(database);
            app.manage(paths.clone());

            // The window is built here rather than by Tauri's own config pass
            // (`app.windows[0].create` is false) for one reason: the portable
            // edition has to name its WebView profile directory, and that has
            // to be set before the WebView is created. `from_config` is the
            // exact builder Tauri would have used, given the exact same config,
            // so the installed window is byte-for-byte the window it was.
            startup::build_main_window(app.handle(), &paths)?;

            // The portable lock is held for as long as ArcScan runs, so the
            // guard lives in application state and is dropped when the process
            // ends -- and released by the kernel if it ends abruptly.
            if let Some(portable) = portable {
                app.manage(portable);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::runtime_info,
            commands::open_data_folder,
            commands::open_portable_downloads,
            commands::scan_network,
            commands::cancel_scan,
            commands::preview_scan,
            commands::parse_port_spec,
            commands::service_catalog,
            commands::device_type_catalog,
            commands::save_scan,
            commands::list_scans,
            commands::get_scan,
            commands::compare_scan,
            commands::delete_scan,
            commands::prune_history,
            commands::list_devices,
            commands::inventory_summary,
            commands::list_change_events,
            commands::set_change_state,
            commands::set_device_statuses,
            commands::list_network_scopes,
            commands::rename_network_scope,
            commands::device_detail,
            commands::set_device_name,
            commands::set_device_status,
            commands::set_device_notes,
            commands::set_device_type_override,
            commands::device_discovery_report,
            commands::device_notes,
            commands::import_device_labels,
            commands::last_scan_ips,
            commands::detect_networks,
            commands::open_releases,
            commands::open_privacy,
            commands::save_text,
            commands::wake_on_lan,
            commands::open_smb,
            commands::open_web,
            commands::open_rdp,
            commands::open_ssh,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ArcScan");

    #[cfg(feature = "portable")]
    {
        // Unlike App::run, run_return gives us a boundary after the native
        // event loop. On Windows, start the no-window cleanup monitor while
        // this process and its exact active-session lock are definitely alive.
        // It cannot delete while that lock is held; after process exit it uses
        // the same strict marker and payload validation with bounded deletion
        // retries. Starting here avoids late-shutdown and PID-reuse races. No
        // extra helper is packaged.
        #[cfg(windows)]
        if let Err(error) = portable_cleanup.spawn_cleanup_monitor() {
            eprintln!(
                "ArcScan Portable could not start its cleanup monitor; the session will be retried on the next launch: {error}"
            );
        }

        // SQLite is closed on Exit before the monitor can observe the active
        // lock being released by process termination.
        let exit_code = app.run_return(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                scanner::request_cancel();
                if let Some(database) = app.try_state::<db::Db>() {
                    if let Err(error) = database.shutdown() {
                        eprintln!("ArcScan Portable database shutdown: {error}");
                    }
                }
            }
        });

        // Portable is Windows-only in release builds. Keeping direct cleanup
        // here makes feature builds and tests on other hosts deterministic.
        #[cfg(not(windows))]
        match portable_cleanup.cleanup() {
            Ok(true) => {}
            Ok(false) => eprintln!(
                "ArcScan Portable left its active temporary session for stale cleanup."
            ),
            Err(error) => eprintln!(
                "ArcScan Portable could not remove its temporary session; it will be retried on the next launch: {error}"
            ),
        }
        std::process::exit(exit_code);
    }

    #[cfg(not(feature = "portable"))]
    app.run(|_, _| {});
}
