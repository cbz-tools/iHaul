// idevice wrapper layer
// All idevice imports are confined to this module to limit
// the blast radius of library upgrades.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use idevice::IdeviceService;
use idevice::services::afc::AfcClient;
use idevice::services::lockdown::LockdownClient;
use idevice::usbmuxd::{UsbmuxdAddr, UsbmuxdConnection, UsbmuxdDevice};

pub mod apps;
pub mod house_arrest;

pub struct DeviceInfo {
    #[allow(dead_code)]
    pub udid:          String,
    pub device_name:   String,          // user-assigned device name (e.g. "John's iPhone")
    pub model_name:    String,          // marketing name (e.g. "iPhone 14")
    pub storage_used:  Option<u64>,     // bytes used (None if unavailable)
    pub storage_total: Option<u64>,     // total capacity in bytes
}

pub struct AppInfo {
    pub bundle_id: String,
    pub display_name: String,
    pub icon_png: Option<Vec<u8>>,  // PNG bytes fetched from SpringBoard
}

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

async fn connect_first_device() -> Result<(Vec<UsbmuxdDevice>, usize), String> {
    let mut mux = UsbmuxdConnection::default()
        .await
        .map_err(|e| e.to_string())?;
    let devices: Vec<UsbmuxdDevice> = mux.get_devices().await.map_err(|e| e.to_string())?;
    if devices.is_empty() {
        return Err("no device found".to_string());
    }
    Ok((devices, 0))
}

/// Fetches DeviceName, ProductType, and disk capacity from lockdownd.
async fn fetch_device_info(
    provider: &impl idevice::provider::IdeviceProvider,
    udid: &str,
) -> (String, String, Option<u64>, Option<u64>) {
    let mut lock = match LockdownClient::connect(provider).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("lockdownd connect failed: {e}");
            return (udid.to_string(), String::new(), None, None);
        }
    };

    let str_val = |v: plist::Value| -> Option<String> {
        if let plist::Value::String(s) = v { Some(s) } else { None }
    };
    let device_name = lock.get_value(Some("DeviceName"), None).await.ok()
        .and_then(str_val)
        .unwrap_or_else(|| udid.to_string());

    let model_name = lock.get_value(Some("ProductType"), None).await.ok()
        .and_then(str_val)
        .map(|pt| product_type_to_name(&pt))
        .unwrap_or_default();

    // Fetch storage via AFC (com.apple.afc).
    // com.apple.disk_usage via lockdownd requires a session and returns "prohibited", so use AFC instead.
    let (storage_used, storage_total) = match AfcClient::connect(provider).await {
        Ok(mut afc) => match afc.get_device_info().await {
            Ok(info) => {
                let total = info.total_bytes as u64;
                let free  = info.free_bytes  as u64;
                let used  = total.saturating_sub(free);
                log::info!("storage via AFC: total={total} free={free} used={used}");
                (Some(used), Some(total))
            }
            Err(e) => { log::warn!("AFC get_device_info failed: {e}"); (None, None) }
        },
        Err(e) => { log::warn!("AFC connect failed: {e}"); (None, None) }
    };

    (device_name, model_name, storage_used, storage_total)
}

/// Converts a ProductType string (e.g. "iPhone14,7") to its marketing name.
/// Unknown models are returned as-is.
fn product_type_to_name(pt: &str) -> String {
    let name = match pt {
        // iPhone 16 series
        "iPhone17,3" => "iPhone 16",
        "iPhone17,4" => "iPhone 16 Plus",
        "iPhone17,1" => "iPhone 16 Pro",
        "iPhone17,2" => "iPhone 16 Pro Max",
        // iPhone 15 series
        "iPhone15,4" => "iPhone 15",
        "iPhone15,5" => "iPhone 15 Plus",
        "iPhone16,1" => "iPhone 15 Pro",
        "iPhone16,2" => "iPhone 15 Pro Max",
        // iPhone 14 series
        "iPhone14,7" => "iPhone 14",
        "iPhone14,8" => "iPhone 14 Plus",
        "iPhone15,2" => "iPhone 14 Pro",
        "iPhone15,3" => "iPhone 14 Pro Max",
        // iPhone 13 series
        "iPhone14,5" => "iPhone 13",
        "iPhone14,4" => "iPhone 13 mini",
        "iPhone14,2" => "iPhone 13 Pro",
        "iPhone14,3" => "iPhone 13 Pro Max",
        // iPhone SE
        "iPhone14,6" => "iPhone SE (3rd gen)",
        "iPhone12,8" => "iPhone SE (2nd gen)",
        // iPhone 12 series
        "iPhone13,1" => "iPhone 12 mini",
        "iPhone13,2" => "iPhone 12",
        "iPhone13,3" => "iPhone 12 Pro",
        "iPhone13,4" => "iPhone 12 Pro Max",
        // iPhone 11 series
        "iPhone12,1" => "iPhone 11",
        "iPhone12,3" => "iPhone 11 Pro",
        "iPhone12,5" => "iPhone 11 Pro Max",
        // iPhone XR / XS series
        "iPhone11,8" => "iPhone XR",
        "iPhone11,2" => "iPhone XS",
        "iPhone11,4" | "iPhone11,6" => "iPhone XS Max",
        // iPhone X / 8 series
        "iPhone10,3" | "iPhone10,6" => "iPhone X",
        "iPhone10,1" | "iPhone10,4" => "iPhone 8",
        "iPhone10,2" | "iPhone10,5" => "iPhone 8 Plus",
        // iPhone SE (1st gen) / 7 series
        "iPhone9,1"  | "iPhone9,3"  => "iPhone 7",
        "iPhone9,2"  | "iPhone9,4"  => "iPhone 7 Plus",
        "iPhone8,4"                  => "iPhone SE (1st gen)",
        _ => return pt.to_string(),
    };
    name.to_string()
}

/// Fetches app icons from SpringBoard in bulk and stores them in AppInfo.icon_png.
/// Connection failures and per-icon failures are logged and silently skipped.
async fn fetch_app_icons(
    provider: &impl idevice::provider::IdeviceProvider,
    apps: &mut Vec<AppInfo>,
) {
    use idevice::services::springboardservices::SpringBoardServicesClient;
    let mut sb = match SpringBoardServicesClient::connect(provider).await {
        Ok(c)  => c,
        Err(e) => { log::warn!("SpringBoard connect failed: {e}"); return; }
    };
    for app in apps.iter_mut() {
        match sb.get_icon_pngdata(app.bundle_id.clone()).await {
            Ok(png) => { app.icon_png = Some(png); }
            Err(e)  => { log::warn!("get_icon_pngdata({}) failed: {e}", app.bundle_id); }
        }
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Returns true if at least one device is connected (lightweight; no lockdownd).
pub async fn is_any_device_connected() -> bool {
    let Ok(mut mux) = UsbmuxdConnection::default().await else { return false; };
    matches!(mux.get_devices().await, Ok(d) if !d.is_empty())
}

/// Scans for a connected device and returns its file-sharing app list.
/// Returns Ok(None) if no device is connected.
pub async fn scan_and_list() -> Result<Option<(DeviceInfo, Vec<AppInfo>)>, String> {
    let mut mux = UsbmuxdConnection::default()
        .await
        .map_err(|e| { let s = e.to_string(); log::error!("usbmuxd connect failed: {s}"); s })?;
    let devices: Vec<UsbmuxdDevice> = mux.get_devices().await
        .map_err(|e| { let s = e.to_string(); log::error!("get_devices failed: {s}"); s })?;

    if devices.is_empty() {
        return Ok(None);
    }

    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");

    let (device_name, model_name, storage_used, storage_total) =
        fetch_device_info(&provider, &device.udid).await;

    let info = DeviceInfo {
        udid: device.udid.clone(),
        device_name,
        model_name,
        storage_used,
        storage_total,
    };

    let mut app_list = apps::list_apps_info(&provider)
        .await
        .map_err(|e| { let s = e.to_string(); log::error!("list_apps failed: {s}"); s })?;

    // fetch icons from SpringBoard (failures do not prevent the app list from being returned)
    fetch_app_icons(&provider, &mut app_list).await;

    Ok(Some((info, app_list)))
}

/// Lists entries and fetches metadata in a single connection (path must be in "/Documents/..." form).
pub async fn list_dir_with_metadata(
    bundle_id: &str,
    path: &str,
) -> Result<(Vec<FileEntry>, HashMap<String, (u64, String)>), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");
    house_arrest::list_dir_with_metadata(&provider, bundle_id, path)
        .await
        .map(|(entries, info)| {
            let fe = entries.into_iter().map(|e| FileEntry { name: e.name, is_dir: e.is_dir }).collect();
            (fe, info)
        })
        .map_err(|e| e.to_string())
}

/// Creates a directory at the given path (path must be in "/Documents/..." form).
pub async fn make_dir(bundle_id: &str, path: &str) -> Result<(), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");
    house_arrest::make_dir(&provider, bundle_id, path)
        .await
        .map_err(|e| e.to_string())
}

/// Uploads a single file to the device (current_dir must be in "/Documents/..." form).
pub async fn upload_single_file(
    bundle_id: String,
    current_dir: String,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid filename")?;

    house_arrest::upload_file_with_progress(&provider, &bundle_id, &path, &current_dir, filename, &cancel, on_progress)
        .await
        .map_err(|e| { let s = e.to_string(); if s != "cancelled" { log::error!("upload_file failed: file={filename}, error={s}"); } s })
}

/// Deletes multiple files or folders (abs_paths must be absolute paths in "/Documents/..." form).
pub async fn delete_items(bundle_id: &str, abs_paths: &[String]) -> Result<(), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");

    for path in abs_paths {
        if let Err(e) = house_arrest::delete_item(&provider, bundle_id, path).await {
            log::warn!("delete skipped: {path}: {e}");
        }
    }
    Ok(())
}

/// Renames a file or folder (old_abs and new_abs must be absolute paths).
pub async fn rename_file(bundle_id: &str, old_abs: &str, new_abs: &str) -> Result<(), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");
    house_arrest::rename_file(&provider, bundle_id, old_abs, new_abs)
        .await
        .map_err(|e| e.to_string())
}

/// Recursively scans selected iOS paths and returns download tasks + total bytes.
pub async fn scan_export(
    bundle_id: &str,
    ios_paths: &[String],
) -> Result<(Vec<house_arrest::DownloadTask>, u64), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");
    house_arrest::scan_for_download(&provider, bundle_id, ios_paths)
        .await
        .map_err(|e| e.to_string())
}

/// Downloads a file in chunks and writes directly to dest (constant memory usage).
pub async fn download_file(
    bundle_id: &str,
    current_dir: &str,
    filename: &str,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
    on_progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let (devices, _) = connect_first_device().await?;
    let device = &devices[0];
    let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");

    house_arrest::download_file_with_progress(&provider, bundle_id, current_dir, filename, &dest, &cancel, on_progress)
        .await
        .map_err(|e| { let s = e.to_string(); if s != "cancelled" { log::error!("download_file failed: file={filename}, error={s}"); } s })
}
