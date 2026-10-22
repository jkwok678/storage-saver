pub mod ai;
pub mod scan;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan::disk_summary,
            scan::scan_directory,
            scan::stop_scan,
            scan::scan_largest_files,
            scan::stop_largest_scan,
            scan::move_to_trash,
            ai::ai_providers_status,
            ai::connect_ai_provider,
            ai::disconnect_ai_provider,
            ai::ai_models,
            ai::query_ai,
            ai::stop_ai_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
