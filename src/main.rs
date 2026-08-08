#![windows_subsystem = "windows"]
#![allow(linker_messages)]

mod device;
mod i18n;
mod settings;
mod ui;

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, atomic::Ordering, mpsc};
use tokio::sync::{Mutex, Semaphore};
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
        let mut config = simplelog::ConfigBuilder::new();
        let _ = config.set_time_offset_to_local();
        let config = config
            .set_time_format_rfc3339()
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
            let folder_name = path
                .file_name()
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
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
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
        rgba: img.into_raw(),
        width: w,
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

enum GuardError {
    DeviceLost,
    Operation(String),
}

/// Runs one AFC operation while independently confirming that its device remains present.
/// A usbmuxd transport error is only logged; an empty matching-UDID result is DeviceLost.
async fn guard_device<F, T>(udid: &str, operation: F) -> Result<T, GuardError>
where
    F: Future<Output = Result<T, String>>,
{
    let mut operation = std::pin::pin!(operation);
    let mut poll = tokio::time::interval(std::time::Duration::from_secs(1));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = &mut operation => match result {
                Ok(value) => return Ok(value),
                Err(error) => match device::is_device_connected(udid).await {
                    Ok(false) => return Err(GuardError::DeviceLost),
                    Ok(true) => return Err(GuardError::Operation(error)),
                    Err(e) => {
                        log::warn!("device monitor unavailable for {udid}: {e}");
                        return Err(GuardError::Operation(error));
                    }
                },
            },
            _ = poll.tick() => match device::is_device_connected(udid).await {
                Ok(true) => {}
                Ok(false) => return Err(GuardError::DeviceLost),
                Err(e) => log::warn!("device monitor unavailable for {udid}: {e}"),
            }
        }
    }
}

fn notify_device_lost(pool: &mut device::DocumentsPool, msg_tx: &mpsc::Sender<DeviceMessage>) {
    log::info!("device disconnected during AFC operation");
    pool.clear();
    msg_tx.send(DeviceMessage::DeviceDisconnected).ok();
}

/// Fetches a directory listing with metadata through the Documents AFC pool.
async fn refresh_file_list(
    pool: &mut device::DocumentsPool,
    bundle_id: &str,
    current_path: &str,
    msg_tx: &mpsc::Sender<DeviceMessage>,
) -> bool {
    msg_tx.send(DeviceMessage::FileListLoading).ok();
    let udid = match pool.prepare(bundle_id).await {
        Ok(udid) => udid,
        Err(e) => {
            msg_tx.send(DeviceMessage::FileList(Err(e))).ok();
            return false;
        }
    };
    match guard_device(&udid, pool.list_dir_with_metadata(bundle_id, current_path)).await {
        Err(GuardError::DeviceLost) => true,
        Err(GuardError::Operation(e)) => {
            msg_tx.send(DeviceMessage::FileList(Err(e))).ok();
            false
        }
        Ok((entries, info)) => {
            msg_tx
                .send(DeviceMessage::FileList(Ok((
                    entries
                        .into_iter()
                        .map(|e| ui::FileEntry {
                            name: e.name,
                            is_dir: e.is_dir,
                        })
                        .collect(),
                    info,
                ))))
                .ok();
            false
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
    let mut documents_pool = device::DocumentsPool::default();

    'worker: loop {
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
                        match device::is_any_device_connected().await {
                            Ok(true) => {}
                            Ok(false) => {
                                device_connected = false;
                                log::info!("device disconnected (polling)");
                                documents_pool.clear();
                                msg_tx.send(DeviceMessage::DeviceDisconnected).ok();
                            }
                            Err(e) => log::warn!("device monitor unavailable: {e}"),
                        }
                    } else {
                        // full scan: lockdownd + app list
                        match device::scan_and_list().await {
                            Ok(Some((info, apps))) => {
                                log::info!(
                                    "auto-scan: device={}, apps={}",
                                    info.device_name,
                                    apps.len()
                                );
                                device_connected = true;
                                msg_tx
                                    .send(DeviceMessage::ScanResult(Ok(Some((info, apps)))))
                                    .ok();
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
                if refresh_file_list(&mut documents_pool, &bundle_id, &path, &msg_tx).await {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::NavigateTo { bundle_id, path }) => {
                log::info!("navigate: {path}");
                if refresh_file_list(&mut documents_pool, &bundle_id, &path, &msg_tx).await {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::UploadFiles {
                bundle_id,
                current_path,
                paths,
                cancel,
                concurrency,
            }) => {
                let (file_tasks, ios_dirs) = collect_upload_tasks(&paths, &current_path);
                log::info!(
                    "upload started: files={}, dirs={}, concurrency={concurrency}",
                    file_tasks.len(),
                    ios_dirs.len()
                );

                // Queue all files immediately so the UI shows them before mk_dir runs.
                for (path, _) in &file_tasks {
                    let filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let bytes_total = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    msg_tx
                        .send(DeviceMessage::UploadQueued {
                            filename,
                            bytes_total,
                        })
                        .ok();
                }

                // Directory creation is a regular pooled operation. It uses one lease
                // sequentially, before the transfer takes its parallel leases.
                for ios_dir in &ios_dirs {
                    let udid = match documents_pool.prepare(&bundle_id).await {
                        Ok(udid) => udid,
                        Err(e) => {
                            log::warn!("mkdir {ios_dir}: {e}");
                            continue;
                        }
                    };
                    match guard_device(&udid, documents_pool.make_dir(&bundle_id, ios_dir)).await {
                        Ok(()) => {}
                        Err(GuardError::Operation(e)) => log::warn!("mkdir {ios_dir}: {e}"),
                        Err(GuardError::DeviceLost) => {
                            device_connected = false;
                            notify_device_lost(&mut documents_pool, &msg_tx);
                            continue 'worker;
                        }
                    }
                }

                let (udid, sessions) = match documents_pool
                    .take_transfer_sessions(&bundle_id, concurrency)
                    .await
                {
                    Ok(leases) => leases,
                    Err(e) => {
                        log::error!("upload AFC lease failed: {e}");
                        for (path, _) in file_tasks {
                            let filename = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            msg_tx
                                .send(DeviceMessage::UploadFailed {
                                    filename,
                                    error: e.clone(),
                                })
                                .ok();
                        }
                        continue 'worker;
                    }
                };

                let tasks = Arc::new(Mutex::new(VecDeque::from(file_tasks)));
                let semaphore = Arc::new(Semaphore::new(sessions.len()));
                let upload_futures: Vec<_> = sessions
                    .into_iter()
                    .map(|mut session| {
                        let sem = semaphore.clone();
                        let tasks = tasks.clone();
                        let tx = msg_tx.clone();
                        let cancel = cancel.clone();

                        async move {
                            let _permit = sem.acquire_owned().await.unwrap();
                            loop {
                                let Some((path, ios_dest_dir)) = tasks.lock().await.pop_front()
                                else {
                                    return Some(session);
                                };
                                let filename = path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                if cancel.load(Ordering::Relaxed) {
                                    tx.send(DeviceMessage::UploadFailed {
                                        filename,
                                        error: "cancelled".to_string(),
                                    })
                                    .ok();
                                    continue;
                                }

                                tx.send(DeviceMessage::UploadStarted(filename.clone())).ok();
                                let tx_prog = tx.clone();
                                let fname_prog = filename.clone();
                                let result = session
                                    .upload_file_with_progress(
                                        &path,
                                        &ios_dest_dir,
                                        &filename,
                                        &cancel,
                                        move |done, total| {
                                            tx_prog
                                                .send(DeviceMessage::UploadProgress {
                                                    filename: fname_prog.clone(),
                                                    bytes_done: done,
                                                    bytes_total: total,
                                                })
                                                .ok();
                                        },
                                    )
                                    .await
                                    .map_err(|e| e.to_string());
                                match result {
                                    Ok(()) => {
                                        log::info!("upload done: {filename}");
                                        tx.send(DeviceMessage::UploadDone(filename)).ok();
                                    }
                                    Err(e) if e == "cancelled" => {
                                        tx.send(DeviceMessage::UploadFailed { filename, error: e })
                                            .ok();
                                    }
                                    Err(e) => {
                                        log::error!("upload failed: file={filename}, error={e}");
                                        tx.send(DeviceMessage::UploadFailed { filename, error: e })
                                            .ok();
                                        return None;
                                    }
                                }
                            }
                        }
                    })
                    .collect();

                let joined =
                    async { Ok::<_, String>(futures::future::join_all(upload_futures).await) };
                match guard_device(&udid, joined).await {
                    Err(GuardError::DeviceLost) => {
                        device_connected = false;
                        notify_device_lost(&mut documents_pool, &msg_tx);
                        continue 'worker;
                    }
                    Err(GuardError::Operation(e)) => log::error!("upload worker failed: {e}"),
                    Ok(results) => {
                        if results.iter().any(Option::is_none)
                            && matches!(device::is_device_connected(&udid).await, Ok(false))
                        {
                            device_connected = false;
                            notify_device_lost(&mut documents_pool, &msg_tx);
                            continue 'worker;
                        }
                        let healthy_sessions: Vec<_> = results.into_iter().flatten().collect();
                        documents_pool.return_transfer_sessions(
                            &udid,
                            &bundle_id,
                            healthy_sessions,
                        );
                        let remaining = std::mem::take(&mut *tasks.lock().await);
                        for (path, _) in remaining {
                            let filename = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            msg_tx
                                .send(DeviceMessage::UploadFailed {
                                    filename,
                                    error: "AFC session unavailable".to_string(),
                                })
                                .ok();
                        }
                    }
                }
                if refresh_file_list(&mut documents_pool, &bundle_id, &current_path, &msg_tx).await
                {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::DeleteFiles {
                bundle_id,
                current_path,
                abs_paths,
            }) => {
                log::info!("delete started: files={}", abs_paths.len());
                for p in &abs_paths {
                    log::info!("delete: {p}");
                }
                msg_tx.send(DeviceMessage::DeleteStarted).ok();
                let udid = match documents_pool.prepare(&bundle_id).await {
                    Ok(udid) => udid,
                    Err(e) => {
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                        continue 'worker;
                    }
                };
                match guard_device(&udid, documents_pool.delete_items(&bundle_id, &abs_paths)).await
                {
                    Ok(()) => log::info!("delete completed"),
                    Err(GuardError::Operation(e)) => {
                        log::error!("delete failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                    Err(GuardError::DeviceLost) => {
                        device_connected = false;
                        notify_device_lost(&mut documents_pool, &msg_tx);
                        continue 'worker;
                    }
                }
                if refresh_file_list(&mut documents_pool, &bundle_id, &current_path, &msg_tx).await
                {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::MkDir {
                bundle_id,
                current_path,
                new_path,
            }) => {
                log::info!("mkdir: {new_path}");
                let udid = match documents_pool.prepare(&bundle_id).await {
                    Ok(udid) => udid,
                    Err(e) => {
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                        continue 'worker;
                    }
                };
                match guard_device(&udid, documents_pool.make_dir(&bundle_id, &new_path)).await {
                    Ok(()) => {}
                    Err(GuardError::Operation(e)) => {
                        log::error!("mkdir failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                    Err(GuardError::DeviceLost) => {
                        device_connected = false;
                        notify_device_lost(&mut documents_pool, &msg_tx);
                        continue 'worker;
                    }
                }
                if refresh_file_list(&mut documents_pool, &bundle_id, &current_path, &msg_tx).await
                {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::RenameFile {
                bundle_id,
                current_path,
                old_abs,
                new_abs,
            }) => {
                log::info!("rename: {old_abs} -> {new_abs}");
                let udid = match documents_pool.prepare(&bundle_id).await {
                    Ok(udid) => udid,
                    Err(e) => {
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                        continue 'worker;
                    }
                };
                match guard_device(
                    &udid,
                    documents_pool.rename_file(&bundle_id, &old_abs, &new_abs),
                )
                .await
                {
                    Ok(()) => {}
                    Err(GuardError::Operation(e)) => {
                        log::error!("rename failed: {e}");
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                    }
                    Err(GuardError::DeviceLost) => {
                        device_connected = false;
                        notify_device_lost(&mut documents_pool, &msg_tx);
                        continue 'worker;
                    }
                }
                if refresh_file_list(&mut documents_pool, &bundle_id, &current_path, &msg_tx).await
                {
                    device_connected = false;
                    notify_device_lost(&mut documents_pool, &msg_tx);
                }
            }

            Ok(DeviceCommand::ExportFiles {
                bundle_id,
                ios_paths,
                dest_dir,
                cancel,
                concurrency,
            }) => {
                log::info!("export: scanning {} selected items", ios_paths.len());

                // Phase 1: recursive scan uses one ordinary Documents pool lease.
                let udid = match documents_pool.prepare(&bundle_id).await {
                    Ok(udid) => udid,
                    Err(e) => {
                        msg_tx.send(DeviceMessage::OperationError(e)).ok();
                        continue 'worker;
                    }
                };
                let (tasks, _total) =
                    match guard_device(&udid, documents_pool.scan_export(&bundle_id, &ios_paths))
                        .await
                    {
                        Ok(result) => result,
                        Err(GuardError::Operation(e)) => {
                            log::error!("export scan failed: {e}");
                            msg_tx.send(DeviceMessage::OperationError(e)).ok();
                            continue 'worker;
                        }
                        Err(GuardError::DeviceLost) => {
                            device_connected = false;
                            notify_device_lost(&mut documents_pool, &msg_tx);
                            continue 'worker;
                        }
                    };

                log::info!("export: {} files to download", tasks.len());

                // Queue all tasks before starting any download
                for task in &tasks {
                    let display = task.local_rel.to_string_lossy().replace('\\', "/");
                    msg_tx
                        .send(DeviceMessage::DownloadQueued {
                            filename: display,
                            bytes_total: task.size,
                        })
                        .ok();
                }

                let (transfer_udid, sessions) = match documents_pool
                    .take_transfer_sessions(&bundle_id, concurrency)
                    .await
                {
                    Ok(leases) => leases,
                    Err(e) => {
                        log::error!("export AFC lease failed: {e}");
                        for task in tasks {
                            let filename = task.local_rel.to_string_lossy().replace('\\', "/");
                            msg_tx
                                .send(DeviceMessage::DownloadFailed {
                                    filename,
                                    error: e.clone(),
                                })
                                .ok();
                        }
                        continue 'worker;
                    }
                };

                // Phase 2: each lane owns one exclusive Documents AFC session.
                let tasks = Arc::new(Mutex::new(VecDeque::from(tasks)));
                let semaphore = Arc::new(Semaphore::new(sessions.len()));
                let download_futures: Vec<_> = sessions
                    .into_iter()
                    .map(|mut session| {
                        let sem = semaphore.clone();
                        let tasks = tasks.clone();
                        let tx = msg_tx.clone();
                        let cancel = cancel.clone();
                        let dest_dir = dest_dir.clone();

                        async move {
                            let _permit = sem.acquire_owned().await.unwrap();
                            loop {
                                let Some(task) = tasks.lock().await.pop_front() else {
                                    return Some(session);
                                };
                                let display = task.local_rel.to_string_lossy().replace('\\', "/");
                                if cancel.load(Ordering::Relaxed) {
                                    tx.send(DeviceMessage::DownloadFailed {
                                        filename: display,
                                        error: "cancelled".to_string(),
                                    })
                                    .ok();
                                    continue;
                                }

                                let local_dest = dest_dir.join(&task.local_rel);
                                if let Some(parent) = local_dest.parent()
                                    && let Err(e) = tokio::fs::create_dir_all(parent).await
                                {
                                    tx.send(DeviceMessage::DownloadFailed {
                                        filename: display,
                                        error: e.to_string(),
                                    })
                                    .ok();
                                    continue;
                                }

                                tx.send(DeviceMessage::DownloadStarted(display.clone()))
                                    .ok();
                                let (ios_dir, ios_filename) = task
                                    .ios_abs
                                    .rsplit_once('/')
                                    .unwrap_or(("", task.ios_abs.as_str()));
                                let tx_prog = tx.clone();
                                let disp_prog = display.clone();
                                let result = session
                                    .download_file_with_progress(
                                        ios_dir,
                                        ios_filename,
                                        &local_dest,
                                        &cancel,
                                        move |done, total| {
                                            tx_prog
                                                .send(DeviceMessage::DownloadProgress {
                                                    filename: disp_prog.clone(),
                                                    bytes_done: done,
                                                    bytes_total: total,
                                                })
                                                .ok();
                                        },
                                    )
                                    .await
                                    .map_err(|e| e.to_string());
                                match result {
                                    Ok(()) => {
                                        log::info!("export done: {display}");
                                        tx.send(DeviceMessage::DownloadDone(display)).ok();
                                    }
                                    Err(e) if e == "cancelled" => {
                                        tx.send(DeviceMessage::DownloadFailed {
                                            filename: display,
                                            error: e,
                                        })
                                        .ok();
                                    }
                                    Err(e) => {
                                        log::error!("export failed: file={display}, error={e}");
                                        tx.send(DeviceMessage::DownloadFailed {
                                            filename: display,
                                            error: e,
                                        })
                                        .ok();
                                        return None;
                                    }
                                }
                            }
                        }
                    })
                    .collect();

                let joined =
                    async { Ok::<_, String>(futures::future::join_all(download_futures).await) };
                match guard_device(&transfer_udid, joined).await {
                    Err(GuardError::DeviceLost) => {
                        device_connected = false;
                        notify_device_lost(&mut documents_pool, &msg_tx);
                        continue 'worker;
                    }
                    Err(GuardError::Operation(e)) => log::error!("export worker failed: {e}"),
                    Ok(results) => {
                        if results.iter().any(Option::is_none)
                            && matches!(
                                device::is_device_connected(&transfer_udid).await,
                                Ok(false)
                            )
                        {
                            device_connected = false;
                            notify_device_lost(&mut documents_pool, &msg_tx);
                            continue 'worker;
                        }
                        let healthy_sessions: Vec<_> = results.into_iter().flatten().collect();
                        documents_pool.return_transfer_sessions(
                            &transfer_udid,
                            &bundle_id,
                            healthy_sessions,
                        );
                        let remaining = std::mem::take(&mut *tasks.lock().await);
                        for task in remaining {
                            let filename = task.local_rel.to_string_lossy().replace('\\', "/");
                            msg_tx
                                .send(DeviceMessage::DownloadFailed {
                                    filename,
                                    error: "AFC session unavailable".to_string(),
                                })
                                .ok();
                        }
                    }
                }
                log::info!("export finished");
            }
        }
    }
}
