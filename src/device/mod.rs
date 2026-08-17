// idevice wrapper layer
// All idevice imports are confined to this module to limit
// the blast radius of library upgrades.

use idevice::IdeviceService;
use idevice::services::afc::AfcClient;
use idevice::services::lockdown::LockdownClient;
use idevice::usbmuxd::{UsbmuxdAddr, UsbmuxdConnection, UsbmuxdDevice};
use std::collections::HashMap;

pub mod apps;
pub mod house_arrest;

use house_arrest::DocumentsSession;

pub struct DeviceInfo {
    #[allow(dead_code)]
    pub udid: String,
    pub device_name: String, // user-assigned device name (e.g. "John's iPhone")
    pub model_name: String,  // marketing name (e.g. "iPhone 14")
    pub storage_used: Option<u64>, // bytes used (None if unavailable)
    pub storage_total: Option<u64>, // total capacity in bytes
}

pub struct AppInfo {
    pub bundle_id: String,
    pub display_name: String,
    pub icon_png: Option<Vec<u8>>, // PNG bytes fetched from SpringBoard
}

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentsPoolKey {
    udid: String,
    bundle_id: String,
}

/// Idle Documents AFC sessions for one device/app container.
/// The background worker is the sole owner; every session is lent exclusively.
#[derive(Default)]
pub struct DocumentsPool {
    key: Option<DocumentsPoolKey>,
    idle: Vec<DocumentsSession>,
}

impl DocumentsPool {
    pub async fn prepare(&mut self, bundle_id: &str) -> Result<String, String> {
        let (devices, _) = connect_first_device().await?;
        let udid = devices[0].udid.clone();
        let key = DocumentsPoolKey {
            udid: udid.clone(),
            bundle_id: bundle_id.to_owned(),
        };
        if self.key.as_ref() != Some(&key) {
            self.clear();
            log::info!("AFC pool scope: device={udid}, app={bundle_id}");
            self.key = Some(key);
        }
        Ok(udid)
    }

    pub fn clear(&mut self) {
        if let Some(key) = self.key.take() {
            log::info!(
                "AFC pool discarded: device={}, app={}, idle={}",
                key.udid,
                key.bundle_id,
                self.idle.len()
            );
        }
        self.idle.clear();
    }

    fn active_udid(&self) -> Option<&str> {
        self.key.as_ref().map(|key| key.udid.as_str())
    }

    async fn open_session(&self, bundle_id: &str) -> Result<DocumentsSession, String> {
        let (devices, _) = connect_first_device().await?;
        let device = &devices[0];
        if self.active_udid() != Some(device.udid.as_str()) {
            return Err("device changed while opening AFC session".to_string());
        }
        let provider = device.to_provider(UsbmuxdAddr::default(), "ihaul");
        house_arrest::open_documents_session(&provider, bundle_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn take_session(&mut self, bundle_id: &str) -> Result<DocumentsSession, String> {
        self.prepare(bundle_id).await?;
        if let Some(session) = self.idle.pop() {
            log::debug!(
                "AFC pool lease: reused idle session (idle={})",
                self.idle.len()
            );
            return Ok(session);
        }
        let session = self.open_session(bundle_id).await?;
        log::info!("AFC pool lease: opened session");
        Ok(session)
    }

    fn return_session(&mut self, session: DocumentsSession) {
        self.idle.push(session);
        log::debug!("AFC pool return: idle={}", self.idle.len());
    }

    pub async fn list_dir_with_metadata(
        &mut self,
        bundle_id: &str,
        path: &str,
    ) -> Result<(Vec<FileEntry>, HashMap<String, (u64, String)>), String> {
        let mut session = self.take_session(bundle_id).await?;
        let result = session
            .list_dir_with_metadata(path)
            .await
            .map_err(|e| e.to_string())
            .map(|(entries, info)| {
                let entries = entries
                    .into_iter()
                    .map(|e| FileEntry {
                        name: e.name,
                        is_dir: e.is_dir,
                    })
                    .collect();
                (entries, info)
            });
        if result.is_ok() {
            self.return_session(session);
        }
        result
    }

    pub async fn make_dir(&mut self, bundle_id: &str, path: &str) -> Result<(), String> {
        let mut session = self.take_session(bundle_id).await?;
        let result = session.make_dir(path).await.map_err(|e| e.to_string());
        if result.is_ok() {
            self.return_session(session);
        }
        result
    }

    pub async fn delete_items(
        &mut self,
        bundle_id: &str,
        abs_paths: &[String],
    ) -> Result<(), String> {
        let mut session = self.take_session(bundle_id).await?;
        let result = session
            .delete_items(abs_paths)
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            self.return_session(session);
        }
        result
    }

    pub async fn rename_file(
        &mut self,
        bundle_id: &str,
        old_abs: &str,
        new_abs: &str,
    ) -> Result<(), String> {
        let mut session = self.take_session(bundle_id).await?;
        let result = session
            .rename_file(old_abs, new_abs)
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            self.return_session(session);
        }
        result
    }

    pub async fn scan_export(
        &mut self,
        bundle_id: &str,
        ios_paths: &[String],
    ) -> Result<(Vec<house_arrest::DownloadTask>, u64), String> {
        let mut session = self.take_session(bundle_id).await?;
        let result = session
            .scan_for_download(ios_paths)
            .await
            .map_err(|e| e.to_string());
        if result.is_ok() {
            self.return_session(session);
        }
        result
    }

    pub async fn take_transfer_sessions(
        &mut self,
        bundle_id: &str,
        count: usize,
    ) -> Result<(String, Vec<DocumentsSession>), String> {
        let udid = self.prepare(bundle_id).await?;
        let count = count.clamp(1, 8);
        let mut sessions = Vec::with_capacity(count);
        while sessions.len() < count {
            if let Some(session) = self.idle.pop() {
                sessions.push(session);
            } else {
                match self.open_session(bundle_id).await {
                    Ok(session) => sessions.push(session),
                    Err(e) if sessions.is_empty() => return Err(e),
                    Err(e) => {
                        log::warn!("AFC pool transfer lease reduced: {e}");
                        break;
                    }
                }
            }
        }
        log::info!("AFC pool transfer lease: sessions={}", sessions.len());
        Ok((udid, sessions))
    }

    pub fn return_transfer_sessions(
        &mut self,
        udid: &str,
        bundle_id: &str,
        sessions: Vec<DocumentsSession>,
    ) {
        let expected = DocumentsPoolKey {
            udid: udid.to_owned(),
            bundle_id: bundle_id.to_owned(),
        };
        if self.key.as_ref() != Some(&expected) {
            return;
        }
        self.idle.extend(sessions);
        log::info!("AFC pool transfer return: idle={}", self.idle.len());
    }
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
        if let plist::Value::String(s) = v {
            Some(s)
        } else {
            None
        }
    };
    let device_name = lock
        .get_value(Some("DeviceName"), None)
        .await
        .ok()
        .and_then(str_val)
        .unwrap_or_else(|| udid.to_string());

    let model_name = lock
        .get_value(Some("ProductType"), None)
        .await
        .ok()
        .and_then(str_val)
        .map(|pt| product_type_to_name(&pt))
        .unwrap_or_default();

    // Fetch storage via AFC (com.apple.afc).
    // com.apple.disk_usage via lockdownd requires a session and returns "prohibited", so use AFC instead.
    let (storage_used, storage_total) = match AfcClient::connect(provider).await {
        Ok(mut afc) => match afc.get_device_info().await {
            Ok(info) => {
                let total = info.total_bytes as u64;
                let free = info.free_bytes as u64;
                let used = total.saturating_sub(free);
                log::info!("storage via AFC: total={total} free={free} used={used}");
                (Some(used), Some(total))
            }
            Err(e) => {
                log::warn!("AFC get_device_info failed: {e}");
                (None, None)
            }
        },
        Err(e) => {
            log::warn!("AFC connect failed: {e}");
            (None, None)
        }
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
        "iPhone9,1" | "iPhone9,3" => "iPhone 7",
        "iPhone9,2" | "iPhone9,4" => "iPhone 7 Plus",
        "iPhone8,4" => "iPhone SE (1st gen)",
        _ => return pt.to_string(),
    };
    name.to_string()
}

/// Fetches app icons from SpringBoard in bulk and stores them in AppInfo.icon_png.
/// Connection failures and per-icon failures are logged and silently skipped.
async fn fetch_app_icons(provider: &impl idevice::provider::IdeviceProvider, apps: &mut [AppInfo]) {
    use idevice::services::springboardservices::SpringBoardServicesClient;
    let mut sb = match SpringBoardServicesClient::connect(provider).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("SpringBoard connect failed: {e}");
            return;
        }
    };
    for app in apps.iter_mut() {
        match sb.get_icon_pngdata(app.bundle_id.clone()).await {
            Ok(png) => {
                app.icon_png = Some(png);
            }
            Err(e) => {
                log::warn!("get_icon_pngdata({}) failed: {e}", app.bundle_id);
            }
        }
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Checks whether any device is connected (lightweight; no lockdownd).
/// The caller must distinguish a confirmed empty list from a usbmuxd error.
pub async fn is_any_device_connected() -> Result<bool, String> {
    let mut mux = UsbmuxdConnection::default()
        .await
        .map_err(|e| e.to_string())?;
    let devices: Vec<UsbmuxdDevice> = mux.get_devices().await.map_err(|e| e.to_string())?;
    Ok(!devices.is_empty())
}

/// Checks for this exact device. Errors are intentionally distinct from a
/// confirmed absence so a usbmuxd hiccup is not treated as DeviceLost.
pub async fn is_device_connected(udid: &str) -> Result<bool, String> {
    let mut mux = UsbmuxdConnection::default()
        .await
        .map_err(|e| e.to_string())?;
    let devices: Vec<UsbmuxdDevice> = mux.get_devices().await.map_err(|e| e.to_string())?;
    Ok(devices.iter().any(|device| device.udid == udid))
}

/// Scans for a connected device and returns its file-sharing app list.
/// Returns Ok(None) if no device is connected.
pub async fn scan_and_list() -> Result<Option<(DeviceInfo, Vec<AppInfo>)>, String> {
    let mut mux = UsbmuxdConnection::default().await.map_err(|e| {
        let s = e.to_string();
        log::error!("usbmuxd connect failed: {s}");
        s
    })?;
    let devices: Vec<UsbmuxdDevice> = mux.get_devices().await.map_err(|e| {
        let s = e.to_string();
        log::error!("get_devices failed: {s}");
        s
    })?;

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

    let mut app_list = apps::list_apps_info(&provider).await.map_err(|e| {
        let s = e.to_string();
        log::error!("list_apps failed: {s}");
        s
    })?;

    // fetch icons from SpringBoard (failures do not prevent the app list from being returned)
    fetch_app_icons(&provider, &mut app_list).await;

    Ok(Some((info, app_list)))
}
