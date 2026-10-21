mod scan;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan::disk_summary,
            scan::scan_directory,
            scan::stop_scan,
            scan::scan_largest_files,
            scan::stop_largest_scan
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
