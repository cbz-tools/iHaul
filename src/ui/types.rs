// ui/types.rs — UI layer type definitions
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    collections::HashMap,
    time::Instant,
};
use crate::device::{AppInfo, DeviceInfo};

// ─── Public types (referenced from main.rs) ──────────────────────────────────

/// File metadata: (size in bytes, last-modified string e.g. "2024/01/15 12:34").
pub type FileMetadata = (u64, String);

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

// ─── Commands (GUI → worker) ─────────────────────────────────────────────────

pub enum DeviceCommand {
    SelectApp { bundle_id: String, path: String },
    NavigateTo { bundle_id: String, path: String },
    UploadFiles {
        bundle_id: String,
        current_path: String,
        paths: Vec<PathBuf>,
        cancel: Arc<AtomicBool>,
        concurrency: usize,
    },
    DeleteFiles { bundle_id: String, current_path: String, abs_paths: Vec<String> },
    MkDir { bundle_id: String, current_path: String, new_path: String },
    RenameFile {
        bundle_id: String,
        current_path: String,
        old_abs: String,
        new_abs: String,
    },
    ExportFiles {
        bundle_id:  String,
        ios_paths:  Vec<String>,   // absolute iOS paths of selected items
        dest_dir:   PathBuf,
        cancel:     Arc<AtomicBool>,
        concurrency: usize,
    },
}

// ─── Messages (worker → GUI) ─────────────────────────────────────────────────

pub enum DeviceMessage {
    ScanResult(Result<Option<(DeviceInfo, Vec<AppInfo>)>, String>),
    DeviceDisconnected,
    FileListLoading,
    FileList(Result<(Vec<FileEntry>, HashMap<String, FileMetadata>), String>),
    DeleteStarted,
    UploadQueued { filename: String, bytes_total: u64 },
    UploadStarted(String),
    UploadProgress { filename: String, bytes_done: u64, bytes_total: u64 },
    UploadDone(String),
    UploadFailed { filename: String, error: String },
    DownloadQueued { filename: String, bytes_total: u64 },
    DownloadStarted(String),
    DownloadProgress { filename: String, bytes_done: u64, bytes_total: u64 },
    DownloadDone(String),
    DownloadFailed { filename: String, error: String },
    OperationError(String),
}

// ─── Internal state types ────────────────────────────────────────────────────

pub(super) enum DeviceStatus {
    Unknown,
    Connected {
        device_name:   String,
        model_name:    String,
        storage_used:  Option<u64>,
        storage_total: Option<u64>,
    },
    Disconnected,
    Error(String),
}

pub(super) enum FileLoadState {
    Empty,
    Loading,
    Loaded,
    Error(String),
}

pub(super) struct TransferItem {
    pub(super) filename:   String,
    pub(super) is_upload:  bool,
    pub(super) status:     TransferStatus,
    pub(super) bytes_done: u64,
    pub(super) bytes_total: u64,
    pub(super) started_at: Option<Instant>,
}

pub(super) enum TransferStatus {
    Queued,
    Active,
    Done,
    Failed(#[allow(dead_code)] String),
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum SortColumn {
    Name,
    Kind,
    Size,
    Modified,
}

/// Actions accumulated during file table rendering; applied after the frame
/// to avoid &mut self conflicts inside egui closures.
#[derive(Default)]
pub(super) struct FilePanelActions {
    pub(super) enter_folder:     Option<String>,
    pub(super) delete:           bool,
    pub(super) rename:           bool,
    pub(super) new_folder:       bool,
    pub(super) export:           bool,
    pub(super) sort_click:       Option<SortColumn>,
    pub(super) right_click_name: Option<String>,
    pub(super) sel_action:       Option<(usize, String, bool, bool)>,
}
