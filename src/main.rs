#![windows_subsystem = "windows"]

mod device;
mod i18n;
mod settings;
mod ui;

use std::sync::{mpsc, Arc, atomic::Ordering};
use tokio::sync::Semaphore;
use ui::{DeviceCommand, DeviceMessage};

fn init_log() {
    let dir = settings::Settings::app_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let current = dir.join("ihaul.log");
    let old = dir.join("ihaul.log.old");
    if current.exists() {
        let _ = std::fs::rename(&current, &old);
    }
    if let Ok(file) = std::fs::File::create(&current) {
        let config = simplelog::ConfigBuilder::new()
            .add_filter_ignore_str("arboard")
            .add_filter_ignore_str("egui_winit")
            .add_filter_ignore_str("egui")
            .build();
        let _ = simplelog::WriteLogger::init(simplelog::LevelFilter::Info, config, file);
    }
}

/// Expands a list of local paths into a flat list of (local_path, ios_dest_dir) upload tasks.
/// Files are used as-is; directories are walked recursively.
/// Also returns the sorted list of iOS directories that must be pre-created (shallowest first).
fn collect_upload_tasks(
    paths: &[std::path::PathBuf],
    ios_base: &str,
) -> (Vec<(std::path::PathBuf, String)>, Vec<String>) {
    let mut file_tasks: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut ios_dirs: Vec<String> = Vec::new();

    for path in paths {
        if path.is_dir() {
            let folder_name = path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ios_top = format!("{ios_base}/{folder_name}");
            ios_dirs.push(ios_top.clone());
            walk_dir_for_upload(path, &ios_top, &mut file_tasks, &mut ios_dirs);
        } else if path.is_file() {
            file_tasks.push((path.clone(), ios_base.to_string()));
        }
    }

    // Shallowest dirs first so parents exist before children are created
    ios_dirs.sort_by_key(|d| d.bytes().filter(|&b| b == b'/').count());
    ios_dirs.dedup();
    (file_tasks, ios_dirs)
}

fn walk_dir_for_upload(
    dir: &std::path::Path,
    ios_dir: &str,
    file_tasks: &mut Vec<(std::path::PathBuf, String)>,
    ios_dirs: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            let child_ios_dir = format!("{ios_dir}/{name}");
            ios_dirs.push(child_ios_dir.clone());
            walk_dir_for_upload(&path, &child_ios_dir, file_tasks, ios_dirs);
        } else if path.is_file() {
            file_tasks.push((path, ios_dir.to_string()));
        }
    }
}

fn load_icon() -> Option<eframe::egui::IconData> {
    let bytes = include_bytes!("../assets/app_icon.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image::GenericImageView::dimensions(&img);
    Some(eframe::egui::IconData {
        rgba:   img.into_raw(),
        width:  w,
        height: h,
    })
}

fn main() -> eframe::Result {
    init_log();
    log::info!("iHaul {} started", env!("CARGO_PKG_VERSION"));
    let (cmd_tx, cmd_rx) = mpsc::channel::<DeviceCommand>();
    let (msg_tx, msg_rx) = mpsc::channel::<DeviceMessage>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(background_loop(cmd_rx, msg_tx));
    });

    let saved = settings::Settings::load();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("iHaul")
        .with_inner_size([
            saved.window_w.unwrap_or(960.0),
            saved.window_h.unwrap_or(580.0),
        ]);
    if let (Some(x), Some(y)) = (saved.window_x, saved.window_y) {
        viewport = viewport.with_position(eframe::egui::pos2(x, y));
    }
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "iHaul",
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc, cmd_tx, msg_rx, saved)))),
    )
}

/// Fetches a directory listing with metadata in a single connection and sends it as FileList.
async fn refresh_file_list(
    bundle_id: &str,
    current_path: &str,
    msg_tx: &mpsc::Sender<DeviceMessage>,
) {
    msg_tx.send(DeviceMessage::FileListLoading).ok();
    match device::list_dir_with_metadata(bundle_id, current_path).await {
        Err(e) => {
            msg_tx.send(DeviceMessage::FileList(Err(e))).ok();
        }
        Ok((entries, info)) => {
            msg_tx.send(DeviceMessage::FileList(Ok((
                entries.into_iter().map(|e| ui::FileEntry { name: e.name, is_dir: e.is_dir }).collect(),
                info,
            )))).ok();
        }
    }
}

async fn background_loop(
    cmd_rx: mpsc::Receiver<DeviceCommand>,
    msg_tx: mpsc::Sender<DeviceMessage>,
) {
    // polling state for auto-connect and disconnect detection
    let mut device_connected = false;
    let mut last_check: Option<std::time::Instant> = None; // None = not yet checked → scan immediately

    loop {
        match cmd_rx.try_recv() {
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {
                // connected: lightweight disconnect check (1 s interval)
                // disconnected: full scan for auto-connect (2 s interval)
                let interval = if device_connected {
                    std::time::Duration::from_secs(1)
                } else {
                    std::time::Duration::from_secs(2)
                };
                let should_poll = last_check.map_or(true, |t| t.elapsed() >= interval);

                if should_poll {
                    last_check = Some(std::time::Instant::now());
                    if device_connected {
                        // lightweight check: usbmuxd only (no lockdownd/AFC)
                        if !device::is_any_device_connected().await {
                            device_connected = false;
                            log::info!("device disconnected (polling)");
                            msg_tx.send(DeviceMessage::DeviceDisconnected).ok();
                        }
                    } else {
                        // full scan: lockdownd + app list
                        match device::scan_and_list().await {
                            Ok(Some((info, apps))) => {
                                log::info!("auto-scan: device={}, apps={}", info.device_name, apps.len());
                                device_connected = true;
                                msg_tx.send(DeviceMessage::ScanResult(Ok(Some((info, apps))))).ok();
                            }
                            Ok(None) => {} // no device found, wait for next poll
                            Err(e) => log::warn!("auto-scan error: {e}"),
                        }
                    }
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }

            Ok(DeviceCommand::SelectApp { bundle_id, path }) => {
                log::info!("app selected: {bundle_id}, path={path}");
                refresh_file_list(&bundle_id, &path, &msg_tx).await;
            }

            Ok(DeviceCommand::NavigateTo { bundle_id, path }) => {
                log::info!("navigate: {path}");
                refresh_file_list(&bundle_id, &path, &msg_tx).await;
            }

            Ok(DeviceCommand::UploadFiles { bundle_id, current_path, paths, cancel, concurrency }) => {
                let (file_tasks, ios_dirs) = collect_upload_tasks(&paths, &current_path);
                log::info!(
                    "upload started: files={}, dirs={}, concurrency={concurrency}",
                    file_tasks.len(), ios_dirs.len()
                );

                // Queue all files immediately so the UI shows them before mk_dir runs.
                for (path, _) in &file_tasks {
                    let filename = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let bytes_total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    msg_tx.send(DeviceMessage::UploadQueued { filename, bytes_total }).ok();
                }

                // Pre-create iOS directory structure (sequential, shallowest first).
                // Errors are logged but do not abort the upload (dir may already exist).
                for ios_dir in &ios_dirs {
                    if let Err(e) = device::make_dir(&bundle_id, ios_dir).await {
                        log::warn!("mkdir {ios_dir}: {e}");
                    }
                }

                let semaphore = Arc::new(Semaphore::new(concurrency));

                let upload_futures: Vec<_> = file_tasks
                    .into_iter()
                    .map(|(path, ios_dest_dir)| {
                        let sem = semaphore.clone();
                        let bid = bundle_id.clone();
                        let tx = msg_tx.clone();
                        let cancel = cancel.clone();
                        let filename = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();

                        async move {
                            let _permit = sem.acquire_owned().await.unwrap();

                            if cancel.load(Ordering::Relaxed) {
                                log::warn!("upload cancelled before start: {filename}");
                                tx.send(DeviceMessage::UploadFailed {
                                    filename,
                                    error: "cancelled".to_string(),
                                }).ok();
                                return;
                            }

                            tx.send(DeviceMessage::UploadStarted(filename.clone())).ok();

                            let tx_prog = tx.clone();
                            let fname_prog = filename.clone();
                            let dest_filename = filename.clone();
                            match device::upload_single_file(bid, ios_dest_dir, path, cancel, move |done, total| {
                                tx_prog.send(DeviceMessage::UploadProgress {
                                    filename: fname_prog.clone(),
                                    bytes_done: done,
                                    bytes_total: total,
                                }).ok();
                            }).await {
                                Ok(()) => {
                                    log::info!("upload done: {dest_filename}");
                                    tx.send(DeviceMessage::UploadDone(dest_filename)).ok();
                                }
                                Err(e) if e == "cancelled" => {
                                    tx.send(DeviceMessage::UploadFailed { filename: dest_filename, error: e }).ok();
                                }
                                Err(e) => {
                                    log::error!("upload failed: file={dest_filename}, error={e}");
                                    tx.send(DeviceMessage::UploadFailed { filename: dest_filename, error: e }).ok();
                                }
                            }
                        }
                    })
                    .collect();

                futures::future::join_all(upload_futures).await;
                refresh_file_list(&bundle_id, &current_path, &msg_tx).await;
            }

            Ok(DeviceCommand::DeleteFiles { bundle_id, current_path, abs_paths }) => {
                log::info!("delete started: files={}", abs_paths.len());
                for p in &abs_paths {
                    log::info!("delete: {p}");
                }
                msg_tx.send(DeviceMessage::DeleteStarted).ok();
                match device::delete_items(&bundle_id, &abs_paths).await {
                    Ok(()) => log::info!("delete completed"),
                    Err(e) => {
                        log::error!("delete failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                }
                refresh_file_list(&bundle_id, &current_path, &msg_tx).await;
            }

            Ok(DeviceCommand::MkDir { bundle_id, current_path, new_path }) => {
                log::info!("mkdir: {new_path}");
                match device::make_dir(&bundle_id, &new_path).await {
                    Ok(()) => {}
                    Err(e) => {
                        log::error!("mkdir failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                }
                refresh_file_list(&bundle_id, &current_path, &msg_tx).await;
            }

            Ok(DeviceCommand::RenameFile { bundle_id, current_path, old_abs, new_abs }) => {
                log::info!("rename: {old_abs} -> {new_abs}");
                match device::rename_file(&bundle_id, &old_abs, &new_abs).await {
                    Ok(()) => {}
                    Err(e) => {
                        log::error!("rename failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                }
                refresh_file_list(&bundle_id, &current_path, &msg_tx).await;
            }

            Ok(DeviceCommand::ExportFiles { bundle_id, ios_paths, dest_dir, cancel, concurrency }) => {
                log::info!("export: scanning {} selected items", ios_paths.len());

                // Phase 1: recursive scan — collect tasks + total bytes
                let (tasks, _total) = match device::scan_export(&bundle_id, &ios_paths).await {
                    Ok(r) => r,
                    Err(e) => {
                        log::error!("export scan failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                        continue;
                    }
                };

                log::info!("export: {} files to download", tasks.len());

                // Queue all tasks before starting any download
                for task in &tasks {
                    let display = task.local_rel.to_string_lossy().replace('\\', "/");
                    msg_tx.send(DeviceMessage::DownloadQueued {
                        filename:    display,
                        bytes_total: task.size,
                    }).ok();
                }

                let semaphore = Arc::new(Semaphore::new(concurrency));

                // Phase 2: download with folder structure
                let download_futures: Vec<_> = tasks
                    .into_iter()
                    .map(|task| {
                        let sem     = semaphore.clone();
                        let bid     = bundle_id.clone();
                        let tx      = msg_tx.clone();
                        let cancel  = cancel.clone();
                        let display = task.local_rel.to_string_lossy().replace('\\', "/");
                        let local_dest = dest_dir.join(&task.local_rel);

                        async move {
                            let _permit = sem.acquire_owned().await.unwrap();

                            if cancel.load(Ordering::Relaxed) {
                                log::warn!("export cancelled: {display}");
                                tx.send(DeviceMessage::DownloadFailed {
                                    filename: display,
                                    error: "cancelled".to_string(),
                                }).ok();
                                return;
                            }

                            // Create parent directories for nested files
                            if let Some(parent) = local_dest.parent() {
                                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                                    tx.send(DeviceMessage::DownloadFailed {
                                        filename: display,
                                        error: e.to_string(),
                                    }).ok();
                                    return;
                                }
                            }

                            tx.send(DeviceMessage::DownloadStarted(display.clone())).ok();

                            let (ios_dir, ios_filename) = task.ios_abs
                                .rsplit_once('/')
                                .unwrap_or(("", task.ios_abs.as_str()));

                            let tx_prog  = tx.clone();
                            let disp_prog = display.clone();
                            match device::download_file(&bid, ios_dir, ios_filename, local_dest, cancel, move |done, total| {
                                tx_prog.send(DeviceMessage::DownloadProgress {
                                    filename:    disp_prog.clone(),
                                    bytes_done:  done,
                                    bytes_total: total,
                                }).ok();
                            }).await {
                                Ok(()) => {
                                    log::info!("export done: {display}");
                                    tx.send(DeviceMessage::DownloadDone(display)).ok();
                                }
                                Err(e) => {
                                    tx.send(DeviceMessage::DownloadFailed { filename: display, error: e }).ok();
                                }
                            }
                        }
                    })
                    .collect();

                futures::future::join_all(download_futures).await;
                log::info!("export finished");
            }
        }
    }
}
