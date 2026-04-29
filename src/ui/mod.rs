// ui/mod.rs — App struct and eframe::App implementation
mod types;
mod helpers;

pub use types::{FileEntry, DeviceCommand, DeviceMessage};
use types::{FileMetadata, DeviceStatus, FileLoadState, TransferItem, TransferStatus, SortColumn, FilePanelActions};
use helpers::{
    read_clipboard_files, file_icon, entry_kind, format_size,
    setup_font, apply_win11_theme,
    cmd_btn, flat_icon_btn, danger_button, primary_button, dialog_button,
    W11_SURFACE, W11_PANEL, W11_BORDER, W11_ACCENT,
    W11_SEL, W11_SEL_TEXT, W11_HOVER, W11_TEXT, W11_TOOLBAR,
};

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use egui_material_icons::icons::{
    ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_UPWARD,
    ICON_UPLOAD, ICON_CREATE_NEW_FOLDER, ICON_DOWNLOAD,
    ICON_DRIVE_FILE_RENAME_OUTLINE, ICON_DELETE, ICON_CANCEL,
    ICON_CONTENT_PASTE, ICON_FOLDER, ICON_INSERT_DRIVE_FILE, ICON_SETTINGS,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{mpsc, Arc, atomic::{AtomicBool, Ordering}},
    time::Instant,
};
use crate::device::AppInfo;
use crate::settings::Settings;

// ─── App struct ───────────────────────────────────────────────────────────────

pub struct App {
    device_status:      DeviceStatus,
    apps:               Vec<AppInfo>,
    selected_app_idx:   Option<usize>,
    file_load_state:    FileLoadState,
    files:              Vec<FileEntry>,
    file_info:          HashMap<String, FileMetadata>,
    selected_files:     HashSet<String>,
    anchor_file:        Option<String>,
    sort_column:        SortColumn,
    sort_ascending:     bool,
    filter_text:        String,
    // Navigation
    current_path:   String,
    back_stack:     Vec<String>,
    forward_stack:  Vec<String>,
    // Delete confirmation
    show_delete_confirm: bool,
    files_to_delete:     Vec<(String, bool)>,  // (name, is_dir)
    delete_base_path:    String,               // current_path snapped at trigger time
    // Rename dialog
    renaming_file:      Option<String>,
    rename_input:       String,
    rename_just_opened: bool,
    rename_base_path:   String,               // current_path snapped at trigger time
    is_renaming:        bool,
    // New folder dialog
    show_new_folder:         bool,
    new_folder_input:        String,
    new_folder_just_opened:  bool,
    new_folder_base_path:    String,          // current_path snapped at trigger time
    is_creating_folder:      bool,
    // Settings dialog
    show_settings:      bool,
    settings_snapshot:  Option<(crate::i18n::Lang, usize)>,  // snapshot for Cancel to restore
    // Transfer tracking
    transfers:         Vec<TransferItem>,
    transfer_cancel:   Option<Arc<AtomicBool>>,
    speed_checkpoints: VecDeque<(Instant, u64)>,
    is_deleting:          bool,
    is_preparing_export:  bool,
    status_msg:           Option<(String, Instant)>,
    settings:          Settings,
    ctrl_v_was_held:   bool,
    hovered_row:       std::cell::Cell<Option<usize>>,
    // App icons (bundle_id → texture, refreshed on each connection)
    app_icons: HashMap<String, egui::TextureHandle>,
    // Channels
    cmd_tx: mpsc::Sender<DeviceCommand>,
    msg_rx: mpsc::Receiver<DeviceMessage>,
}

// ─── App impl: constructor, message handling, helpers ────────────────────────

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cmd_tx: mpsc::Sender<DeviceCommand>,
        msg_rx: mpsc::Receiver<DeviceMessage>,
        settings: Settings,
    ) -> Self {
        setup_font(&cc.egui_ctx);
        apply_win11_theme(&cc.egui_ctx);
        Self {
            device_status:   DeviceStatus::Unknown,
            apps:            Vec::new(),
            selected_app_idx: None,
            file_load_state: FileLoadState::Empty,
            files:           Vec::new(),
            file_info:       HashMap::new(),
            selected_files:  HashSet::new(),
            anchor_file:     None,
            sort_column:     SortColumn::Name,
            sort_ascending:  true,
            filter_text:     String::new(),
            current_path:    "/Documents".to_string(),
            back_stack:      Vec::new(),
            forward_stack:   Vec::new(),
            show_delete_confirm: false,
            files_to_delete:     Vec::new(),
            delete_base_path:    String::new(),
            renaming_file:      None,
            rename_input:       String::new(),
            rename_just_opened: false,
            rename_base_path:   String::new(),
            is_renaming:        false,
            show_new_folder:         false,
            new_folder_input:        String::new(),
            new_folder_just_opened:  false,
            new_folder_base_path:    String::new(),
            is_creating_folder:      false,
            show_settings:     false,
            settings_snapshot: None,
            transfers:         Vec::new(),
            transfer_cancel:   None,
            speed_checkpoints: VecDeque::new(),
            is_deleting:         false,
            is_preparing_export: false,
            status_msg:          None,
            settings,
            ctrl_v_was_held: false,
            hovered_row:     std::cell::Cell::new(None),
            app_icons: HashMap::new(),
            cmd_tx,
            msg_rx,
        }
    }

    fn poll_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                DeviceMessage::ScanResult(Ok(Some((info, apps)))) => {
                    self.device_status = DeviceStatus::Connected {
                        device_name:   info.device_name,
                        model_name:    info.model_name,
                        storage_used:  info.storage_used,
                        storage_total: info.storage_total,
                    };
                    self.app_icons.clear();  // discard stale textures on reconnect
                    self.apps = apps;
                    self.reset_file_state();
                }
                DeviceMessage::ScanResult(Ok(None)) => {
                    self.device_status = DeviceStatus::Disconnected;
                    self.apps.clear();
                    self.app_icons.clear();
                    self.selected_app_idx = None;
                    self.reset_file_state();
                }
                DeviceMessage::ScanResult(Err(e)) => {
                    self.device_status = DeviceStatus::Error(e);
                }
                DeviceMessage::DeviceDisconnected => {
                    self.device_status = DeviceStatus::Disconnected;
                    self.apps.clear();
                    self.app_icons.clear();
                    self.selected_app_idx = None;
                    self.reset_file_state();
                    let msg = crate::i18n::strings(self.settings.lang).disconnected_msg.to_string();
                    self.set_status(msg);
                }
                DeviceMessage::FileListLoading => {
                    self.file_load_state = FileLoadState::Loading;
                    self.is_deleting = false;
                    self.is_renaming = false;
                    self.is_creating_folder = false;
                }
                DeviceMessage::DeleteStarted => {
                    self.is_deleting = true;
                }
                DeviceMessage::FileList(Ok((entries, info))) => {
                    let names: HashSet<String> = entries.iter().map(|e| e.name.clone()).collect();
                    self.selected_files.retain(|s| names.contains(s));
                    self.files = entries;
                    self.file_info = info;
                    self.file_load_state = FileLoadState::Loaded;
                    if !self.has_active_transfers() {
                        self.clear_transfers();
                    }
                }
                DeviceMessage::FileList(Err(e)) => {
                    self.file_load_state = FileLoadState::Error(e);
                }
                DeviceMessage::UploadQueued { filename, bytes_total } => {
                    self.push_transfer_queued(filename, bytes_total, true);
                }
                DeviceMessage::UploadStarted(f) => {
                    self.handle_transfer_started(&f, true);
                }
                DeviceMessage::UploadProgress { filename, bytes_done, bytes_total } => {
                    self.handle_transfer_progress(&filename, bytes_done, bytes_total, true);
                }
                DeviceMessage::UploadDone(f) => { self.mark_transfer_done(&f, true); }
                DeviceMessage::UploadFailed { filename, error } => {
                    self.mark_transfer_failed(&filename, true, error);
                }
                DeviceMessage::DownloadQueued { filename, bytes_total } => {
                    self.is_preparing_export = false;
                    self.push_transfer_queued(filename, bytes_total, false);
                }
                DeviceMessage::DownloadStarted(f) => {
                    self.handle_transfer_started(&f, false);
                }
                DeviceMessage::DownloadProgress { filename, bytes_done, bytes_total } => {
                    self.handle_transfer_progress(&filename, bytes_done, bytes_total, false);
                }
                DeviceMessage::DownloadDone(f)  => { self.mark_transfer_done(&f, false); }
                DeviceMessage::DownloadFailed { filename, error } => {
                    self.mark_transfer_failed(&filename, false, error);
                }
                DeviceMessage::OperationError(e) => {
                    self.is_renaming = false;
                    let prefix = crate::i18n::strings(self.settings.lang).error_prefix;
                    self.set_status(format!("{prefix}{e}"));
                }
            }
        }
        // auto-clear when all transfers are done or failed (e.g. after export completes)
        if !self.transfers.is_empty() && !self.has_active_transfers() {
            self.clear_transfers();
        }
    }

    fn reset_file_state(&mut self) {
        self.files.clear();
        self.file_info.clear();
        self.selected_files.clear();
        self.anchor_file = None;
        self.filter_text.clear();
        self.file_load_state = FileLoadState::Empty;
        self.clear_transfers();
        self.transfer_cancel   = None;
        self.is_deleting       = false;
        self.is_preparing_export = false;
        self.current_path = "/Documents".to_string();
        self.back_stack.clear();
        self.forward_stack.clear();
    }

    fn push_transfer_queued(&mut self, filename: String, bytes_total: u64, is_upload: bool) {
        self.transfers.push(TransferItem {
            filename, is_upload,
            status: TransferStatus::Queued,
            bytes_done: 0, bytes_total, started_at: None,
        });
    }

    fn handle_transfer_started(&mut self, filename: &str, is_upload: bool) {
        if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
            t.filename == filename && t.is_upload == is_upload && matches!(t.status, TransferStatus::Queued)
        }) {
            t.status = TransferStatus::Active;
            if !is_upload { t.started_at = Some(Instant::now()); }
        }
    }

    fn handle_transfer_progress(&mut self, filename: &str, bytes_done: u64, bytes_total: u64, is_upload: bool) {
        if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
            t.filename == filename && t.is_upload == is_upload && matches!(t.status, TransferStatus::Active)
        }) {
            if is_upload && t.started_at.is_none() && bytes_done > 0 { t.started_at = Some(Instant::now()); }
            t.bytes_done = bytes_done;
            t.bytes_total = bytes_total;
        }
        let cum: u64 = self.transfers.iter()
            .filter(|t| t.is_upload == is_upload && !matches!(t.status, TransferStatus::Failed(_)))
            .map(|t| t.bytes_done).sum();
        let now = Instant::now();
        self.speed_checkpoints.push_back((now, cum));
        let cutoff = now - std::time::Duration::from_secs(3);
        while self.speed_checkpoints.len() > 2 {
            if self.speed_checkpoints.front().map(|(t, _)| *t < cutoff).unwrap_or(false) {
                self.speed_checkpoints.pop_front();
            } else { break; }
        }
    }

    fn mark_transfer_done(&mut self, filename: &str, is_upload: bool) {
        if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
            t.filename == filename && t.is_upload == is_upload
                && !matches!(t.status, TransferStatus::Done | TransferStatus::Failed(_))
        }) { t.status = TransferStatus::Done; }
    }

    fn mark_transfer_failed(&mut self, filename: &str, is_upload: bool, error: String) {
        if let Some(t) = self.transfers.iter_mut().rev().find(|t| {
            t.filename == filename && t.is_upload == is_upload
                && !matches!(t.status, TransferStatus::Done | TransferStatus::Failed(_))
        }) { t.status = TransferStatus::Failed(error); }
    }

    fn set_status(&mut self, msg: String) { self.status_msg = Some((msg, Instant::now())); }

    fn has_active_transfers(&self) -> bool {
        self.transfers.iter().any(|t| matches!(t.status, TransferStatus::Queued | TransferStatus::Active))
    }

    fn selected_bundle_id(&self) -> Option<String> {
        self.selected_app_idx.and_then(|i| self.apps.get(i).map(|a| a.bundle_id.clone()))
    }

    fn clear_transfers(&mut self) {
        self.transfers.clear();
        self.speed_checkpoints.clear();
    }

    fn clear_transfers_if_idle(&mut self) {
        if !self.has_active_transfers() { self.clear_transfers(); }
    }

    fn has_current_op(&self) -> bool {
        self.is_renaming
            || self.is_deleting
            || self.is_preparing_export
            || matches!(self.file_load_state, FileLoadState::Loading)
            || self.has_active_transfers()
    }

    fn is_busy(&self) -> bool {
        self.has_current_op() || self.any_dialog_open()
    }

    fn current_op_label<'a>(&self, s: &'a crate::i18n::S) -> Option<&'a str> {
        if self.is_renaming          { return Some(s.op_renaming); }
        if self.is_deleting          { return Some(s.op_deleting); }
        if self.is_creating_folder   { return Some(s.op_creating_folder); }
        if self.is_preparing_export  { return Some(s.op_preparing); }
        if matches!(self.file_load_state, FileLoadState::Loading) { return Some(s.op_loading); }
        None
    }

    fn toggle_sort(&mut self, col: SortColumn) {
        if self.sort_column == col { self.sort_ascending = !self.sort_ascending; }
        else { self.sort_column = col; self.sort_ascending = true; }
    }

    fn new_cancel_flag(&mut self) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.transfer_cancel = Some(flag.clone());
        flag
    }

    fn any_dialog_open(&self) -> bool {
        self.show_delete_confirm || self.show_new_folder || self.renaming_file.is_some()
    }

    fn trigger_delete_confirm(&mut self) {
        if self.any_dialog_open() { return; }
        if !self.selected_files.is_empty() {
            self.delete_base_path = self.current_path.clone();
            self.files_to_delete = self.selected_files.iter().map(|name| {
                let is_dir = self.files.iter().find(|e| &e.name == name).map(|e| e.is_dir).unwrap_or(false);
                (name.clone(), is_dir)
            }).collect();
            self.files_to_delete.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            self.show_delete_confirm = true;
        }
    }

    fn trigger_rename(&mut self) {
        if self.any_dialog_open() { return; }
        if self.selected_files.len() == 1 {
            self.rename_base_path = self.current_path.clone();
            let name = self.selected_files.iter().next().unwrap().clone();
            self.rename_input = name.clone();
            self.renaming_file = Some(name);
            self.rename_just_opened = true;
        }
    }

    fn trigger_new_folder(&mut self) {
        if self.any_dialog_open() { return; }
        self.new_folder_base_path = self.current_path.clone();
        self.new_folder_input.clear();
        self.show_new_folder = true;
        self.new_folder_just_opened = true;
    }

    fn trigger_export(&mut self, bundle_id: &str, dest: std::path::PathBuf) {
        let cur = self.current_path.clone();
        let ios_paths: Vec<String> = self.selected_files.iter()
            .map(|f| format!("{cur}/{f}"))
            .collect();
        self.clear_transfers_if_idle();
        self.is_preparing_export = true;
        let cancel = self.new_cancel_flag();
        let concurrency = self.settings.concurrency;
        self.cmd_tx.send(DeviceCommand::ExportFiles {
            bundle_id: bundle_id.to_owned(),
            ios_paths, dest_dir: dest, cancel, concurrency,
        }).ok();
    }

    fn trigger_paste(&mut self) {
        if let Some(bid) = self.selected_bundle_id() {
            let paths = read_clipboard_files();
            if !paths.is_empty() {
                let cancel = self.new_cancel_flag();
                let concurrency = self.settings.concurrency;
                let cur = self.current_path.clone();
                self.cmd_tx.send(DeviceCommand::UploadFiles {
                    bundle_id: bid, current_path: cur, paths, cancel, concurrency,
                }).ok();
            }
        }
    }

    /// Shared navigation core: clears transient state and sends NavigateTo.
    /// Callers are responsible for updating back_stack / forward_stack beforehand.
    fn do_navigate(&mut self, path: String) {
        self.clear_transfers_if_idle();
        self.current_path = path.clone();
        self.selected_files.clear();
        self.anchor_file = None;
        self.files.clear();
        self.file_info.clear();
        if let Some(bid) = self.selected_bundle_id() {
            self.cmd_tx.send(DeviceCommand::NavigateTo { bundle_id: bid, path }).ok();
        }
    }

    fn navigate_to(&mut self, path: String) {
        if path == self.current_path { return; }
        self.back_stack.push(self.current_path.clone());
        self.forward_stack.clear();
        self.do_navigate(path);
    }

    fn navigate_back(&mut self) {
        if let Some(prev) = self.back_stack.pop() {
            self.forward_stack.push(self.current_path.clone());
            self.do_navigate(prev);
        }
    }

    fn navigate_forward(&mut self) {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(self.current_path.clone());
            self.do_navigate(next);
        }
    }

    fn navigate_up(&mut self) {
        if self.current_path == "/Documents" { return; }
        if let Some(pos) = self.current_path.rfind('/') {
            let parent = self.current_path[..pos].to_string();
            let parent = if parent.is_empty() { "/Documents".to_string() } else { parent };
            self.navigate_to(parent);
        }
    }
}

// ─── App impl: update() sub-methods ──────────────────────────────────────────

impl App {
    /// Handles keyboard shortcuts and drag-and-drop.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, is_busy: bool) {
        let has_sel  = !self.selected_files.is_empty();
        let app_sel  = self.selected_app_idx.is_some();

        // Do not treat Ctrl+V as a file paste when a text input has focus
        let text_edit_focused = ctx.memory(|m| {
            // TODO: file_filter_input is currently commented out
            m.has_focus(egui::Id::new("rename_text_edit"))
            || m.has_focus(egui::Id::new("new_folder_input"))
        });

        // Ctrl+V: handles both CF_HDROP and text path clipboard formats
        let paste_event = !text_edit_focused
            && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        let ctrl_v_held = {
            unsafe extern "system" { fn GetAsyncKeyState(vKey: i32) -> i16; }
            unsafe {
                (GetAsyncKeyState(0x11) as u16 & 0x8000 != 0) &&
                (GetAsyncKeyState(0x56) as u16 & 0x8000 != 0)
            }
        };
        let ctrl_v_win32 = !text_edit_focused && ctrl_v_held && !self.ctrl_v_was_held && !paste_event;
        self.ctrl_v_was_held = ctrl_v_held;
        let ctrl_v       = paste_event || ctrl_v_win32;
        let ctrl_a       = ctx.input(|i| i.key_pressed(egui::Key::A)          && i.modifiers.ctrl);
        let del_key      = ctx.input(|i| i.key_pressed(egui::Key::Delete));
        let f2_key       = ctx.input(|i| i.key_pressed(egui::Key::F2));
        let enter        = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let alt_left     = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)  && i.modifiers.alt);
        let alt_right    = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight) && i.modifiers.alt);
        let ctrl_shift_n = ctx.input(|i| i.key_pressed(egui::Key::N)          && i.modifiers.ctrl && i.modifiers.shift);
        let mouse_back   = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1));
        let mouse_forward= ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2));

        // TODO: when search filter is re-enabled, Esc should clear the filter text
        // if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.filter_text.is_empty() {
        //     self.filter_text.clear();
        // }

        if !is_busy && app_sel {
            if alt_left  || mouse_back    { self.navigate_back(); }
            if alt_right || mouse_forward { self.navigate_forward(); }
        }

        if !is_busy {
            if ctrl_v && app_sel  { self.trigger_paste(); }
            if ctrl_a && app_sel  { self.selected_files = self.files.iter().map(|e| e.name.clone()).collect(); }
            if del_key && has_sel { self.trigger_delete_confirm(); }
            let sel_has_dir = self.selected_files.iter()
                .any(|n| self.files.iter().any(|e| &e.name == n && e.is_dir));
            if f2_key && has_sel && self.renaming_file.is_none() && !sel_has_dir {
                self.trigger_rename();
            }
            if ctrl_shift_n && app_sel { self.trigger_new_folder(); }
            if enter && has_sel && self.selected_files.len() == 1 {
                let sel = self.selected_files.iter().next().unwrap().clone();
                if self.files.iter().any(|e| e.name == sel && e.is_dir) {
                    let path = format!("{}/{sel}", self.current_path);
                    self.navigate_to(path);
                }
            }
        }

        // D&D
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() && !is_busy {
            if let Some(bid) = self.selected_bundle_id() {
                let paths: Vec<PathBuf> = dropped.iter().filter_map(|f| f.path.clone()).collect();
                if !paths.is_empty() {
                    let cancel = self.new_cancel_flag();
                    let concurrency = self.settings.concurrency;
                    let cur = self.current_path.clone();
                    self.cmd_tx.send(DeviceCommand::UploadFiles {
                        bundle_id: bid, current_path: cur, paths, cancel, concurrency,
                    }).ok();
                }
            }
        }
    }

    /// Connection status toolbar (Win11 style).
    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let s = crate::i18n::strings(self.settings.lang);
        let toolbar_rect = ui.max_rect();
        ui.painter().rect_filled(toolbar_rect, 0.0, W11_TOOLBAR);
        ui.set_min_height(40.0);

        // Settings button: drawn at a fixed position via painter+interact to avoid ID warnings
        // Y center: 2px from toolbar top + half button size (top-aligned layout)
        let settings_sz = 40.0_f32;
        let settings_cy = toolbar_rect.min.y + 2.0 + settings_sz / 2.0;
        let settings_rect = egui::Rect::from_center_size(
            egui::pos2(toolbar_rect.max.x - settings_sz / 2.0 - 4.0, settings_cy),
            egui::Vec2::splat(settings_sz),
        );
        let settings_resp = ui.interact(
            settings_rect, ui.id().with("toolbar_settings"), egui::Sense::click(),
        );
        {
            let bg = if self.show_settings || settings_resp.is_pointer_button_down_on() { W11_SEL }
                     else if settings_resp.hovered() { W11_HOVER }
                     else { egui::Color32::TRANSPARENT };
            if bg != egui::Color32::TRANSPARENT {
                ui.painter().rect_filled(settings_rect.shrink(2.0), 4.0, bg);
            }
            ui.painter().text(
                settings_rect.center(), egui::Align2::CENTER_CENTER, ICON_SETTINGS.codepoint,
                egui::FontId::new(22.0, egui::FontFamily::Name("material-icons".into())),
                W11_TEXT,
            );
        }
        let settings_resp = settings_resp.on_hover_text(s.tip_settings);
        if settings_resp.clicked() && !self.show_settings {
            self.show_settings = true;
            self.settings_snapshot = Some((self.settings.lang, self.settings.concurrency));
        }

        // Device info: small top margin, top-aligned (no bottom padding)
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            match &self.device_status {
                DeviceStatus::Unknown | DeviceStatus::Disconnected => {
                    ui.spinner();
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(s.waiting).color(W11_TEXT).size(16.0));
                }
                DeviceStatus::Connected { device_name, model_name, storage_used, storage_total } => {
                    ui.label(egui::RichText::new("●")
                        .color(egui::Color32::from_rgb(16, 124, 16))
                        .size(12.0));
                    ui.add_space(4.0);
                    let name_part = if model_name.is_empty() {
                        device_name.clone()
                    } else {
                        format!("{} ( {} )", model_name, device_name)
                    };
                    ui.label(egui::RichText::new(&name_part).color(W11_TEXT).strong().size(19.0));
                    let storage_str = match (storage_used, storage_total) {
                        (Some(u), Some(t)) => format!("{} / {}", format_size(*u), format_size(*t)),
                        _ => String::new(),
                    };
                    if !storage_str.is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(storage_str)
                            .color(egui::Color32::from_rgb(96, 94, 92))
                            .size(16.0));
                    }
                }
                DeviceStatus::Error(e) => {
                    ui.colored_label(egui::Color32::from_rgb(196, 43, 28),
                        format!("{}{}", s.error_prefix, e));
                }
            }
            if let Some((msg, _)) = &self.status_msg {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(16, 124, 16), msg);
            }
        });
    }

    /// Left panel: app list (Win11 navigation pane style).
    fn show_sidebar(&mut self, ui: &mut egui::Ui) {
        let s = crate::i18n::strings(self.settings.lang);

        if self.apps.is_empty() {
            ui.add_space(8.0);
            let hint = match &self.device_status {
                DeviceStatus::Error(_)       => s.connect_failed,
                DeviceStatus::Connected {..} => s.no_apps,
                _                            => s.waiting,
            };
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.colored_label(egui::Color32::from_gray(130), egui::RichText::new(hint).size(12.0));
            });
            return;
        }

        // sort alphabetically, then partition into favorites and the rest
        let mut all_sorted: Vec<usize> = (0..self.apps.len()).collect();
        {
            let apps = &self.apps;
            all_sorted.sort_by(|&a, &b| apps[a].display_name.cmp(&apps[b].display_name));
        }
        let (fav_sorted, non_fav_sorted): (Vec<usize>, Vec<usize>) = all_sorted.iter().copied()
            .partition(|&i| self.settings.favorites.contains(&self.apps[i].bundle_id));

        let mut clicked_app: Option<(usize, String)> = None;
        let mut toggle_fav:  Option<String>           = None;

        // pre-load textures (borrow separation)
        // Decode PNG → upload to egui texture → free the raw PNG bytes immediately.
        let ctx = ui.ctx().clone();
        for &real_idx in &all_sorted {
            let bundle_id = self.apps[real_idx].bundle_id.clone();
            if !self.app_icons.contains_key(&bundle_id) {
                // Decode while borrow of icon_png is active, producing an owned DynamicImage.
                let decoded = self.apps[real_idx].icon_png.as_deref()
                    .and_then(|png| image::load_from_memory(png).ok());
                if let Some(img) = decoded {
                    // Borrow of icon_png has ended; safe to mutate below.
                    let rgba = img.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let ci   = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    let handle = ctx.load_texture(
                        format!("icon_{bundle_id}"), ci, egui::TextureOptions::LINEAR,
                    );
                    self.app_icons.insert(bundle_id, handle);
                    // Free the raw PNG bytes — the texture is now on the egui side.
                    self.apps[real_idx].icon_png = None;
                }
            }
        }

        egui::ScrollArea::vertical().id_salt("app_scroll").show(ui, |ui| {
            // ── Favorites section ────────────────────────────────────────
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(s.sidebar_favorites)
                    .color(egui::Color32::from_gray(130)).size(11.0).strong());
            });
            ui.add_space(2.0);
            for &real_idx in &fav_sorted {
                let bundle_id    = self.apps[real_idx].bundle_id.clone();
                let display_name = self.apps[real_idx].display_name.clone();
                let selected     = self.selected_app_idx == Some(real_idx);
                let tex          = self.app_icons.get(&bundle_id);
                let (row_resp, star_clicked) =
                    sidebar_row(ui, &bundle_id, &display_name, selected, true, tex, "fav");
                if star_clicked {
                    toggle_fav = Some(bundle_id.clone());
                } else if row_resp.clicked() && !selected {
                    clicked_app = Some((real_idx, bundle_id.clone()));
                }
            }
            ui.add_space(8.0);

            // ── All apps section ─────────────────────────────────────────
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(s.sidebar_all_apps)
                    .color(egui::Color32::from_gray(130)).size(11.0).strong());
            });
            ui.add_space(2.0);
            for &real_idx in &non_fav_sorted {
                let bundle_id    = self.apps[real_idx].bundle_id.clone();
                let display_name = self.apps[real_idx].display_name.clone();
                let selected     = self.selected_app_idx == Some(real_idx);
                let tex          = self.app_icons.get(&bundle_id);
                let (row_resp, star_clicked) =
                    sidebar_row(ui, &bundle_id, &display_name, selected, false, tex, "all");
                if star_clicked {
                    toggle_fav = Some(bundle_id.clone());
                } else if row_resp.clicked() && !selected {
                    clicked_app = Some((real_idx, bundle_id.clone()));
                }
            }
        });

        if let Some(bid) = toggle_fav {
            if self.settings.favorites.contains(&bid) { self.settings.favorites.remove(&bid); }
            else { self.settings.favorites.insert(bid); }
            self.settings.save();
        }
        if let Some((i, bundle_id)) = clicked_app {
            self.selected_app_idx = Some(i);
            self.reset_file_state();
            self.file_load_state = FileLoadState::Loading;
            self.cmd_tx.send(DeviceCommand::SelectApp {
                bundle_id, path: "/Documents".to_string(),
            }).ok();
        }
    }

    /// Bottom panel for transfer progress.
    fn show_transfer_panel(&mut self, ui: &mut egui::Ui) {
        let s = crate::i18n::strings(self.settings.lang);

        let total        = self.transfers.len();
        let active       = self.transfers.iter().filter(|t| matches!(t.status, TransferStatus::Queued | TransferStatus::Active)).count();
        let done_count   = self.transfers.iter().filter(|t| matches!(t.status, TransferStatus::Done)).count();
        let failed_count = self.transfers.iter().filter(|t| matches!(t.status, TransferStatus::Failed(_))).count();

        let total_bytes: u64 = self.transfers.iter()
            .filter(|t| !matches!(t.status, TransferStatus::Failed(_)))
            .map(|t| t.bytes_total).sum();
        let done_bytes: u64 = self.transfers.iter()
            .filter(|t| !matches!(t.status, TransferStatus::Failed(_)))
            .map(|t| t.bytes_done).sum();

        let progress = if total_bytes > 0 {
            done_bytes as f32 / total_bytes as f32
        } else {
            done_count as f32 / total.max(1) as f32
        };

        let mut info = format!("{} / {}", format_size(done_bytes), format_size(total_bytes));
        if failed_count > 0 { info += &format!(" · {}{}", failed_count, s.failed_suffix); }

        let speed_bps: f64 = if self.speed_checkpoints.len() >= 2 {
            let (t_old, b_old) = self.speed_checkpoints.front().unwrap();
            let (t_new, b_new) = self.speed_checkpoints.back().unwrap();
            let dt = t_new.duration_since(*t_old).as_secs_f64();
            if dt > 0.0 { (b_new - b_old) as f64 / dt } else { 0.0 }
        } else {
            let batch_started = self.transfers.iter()
                .filter_map(|t| t.started_at).min();
            batch_started.map(|t0| {
                let elapsed = t0.elapsed().as_secs_f64();
                if elapsed > 0.5 && done_bytes > 0 { done_bytes as f64 / elapsed } else { 0.0 }
            }).unwrap_or(0.0)
        };
        if speed_bps > 0.0 && active > 0 {
            info += &format!(" · {}{}", format_size(speed_bps as u64), s.per_sec);
            if total_bytes > done_bytes {
                let remain = (total_bytes.saturating_sub(done_bytes) as f64 / speed_bps) as u64;
                info += &format!(" · {}", s.format_remaining(remain));
            }
        }

        let w = ui.available_width();
        ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0))
            .desired_width(w)
            .text(info));
    }

    /// Navigation bar (back/forward/up buttons + address bar).
    fn show_navigation_bar(
        &mut self, ui: &mut egui::Ui,
        can_act: bool, app_name: &str, s: &crate::i18n::S,
    ) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(can_act && !self.back_stack.is_empty(), |ui| {
                if flat_icon_btn(ui, ICON_ARROW_BACK, 28.0)
                    .on_hover_text(s.tip_back).clicked() { self.navigate_back(); }
            });
            ui.add_enabled_ui(can_act && !self.forward_stack.is_empty(), |ui| {
                if flat_icon_btn(ui, ICON_ARROW_FORWARD, 28.0)
                    .on_hover_text(s.tip_forward).clicked() { self.navigate_forward(); }
            });
            ui.add_enabled_ui(can_act && self.current_path != "/Documents", |ui| {
                if flat_icon_btn(ui, ICON_ARROW_UPWARD, 28.0)
                    .on_hover_text(s.tip_up).clicked() { self.navigate_up(); }
            });
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(W11_SURFACE)
                .stroke(egui::Stroke::new(1.0, W11_BORDER))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(app_name).strong().color(W11_TEXT));
                        let segments: Vec<&str> = self.current_path.trim_start_matches('/').split('/').collect();
                        let mut nav_to: Option<String> = None;
                        for (i, seg) in segments.iter().enumerate() {
                            ui.label(egui::RichText::new("›").color(egui::Color32::from_gray(160)));
                            if i == segments.len() - 1 {
                                ui.label(egui::RichText::new(*seg).strong().color(W11_TEXT));
                            } else {
                                let path = format!("/{}", segments[..=i].join("/"));
                                if ui.link(*seg).clicked() { nav_to = Some(path); }
                            }
                        }
                        if let Some(p) = nav_to { self.navigate_to(p); }
                    });
                });
        });
    }

    /// Command bar (Upload/Export/NewFolder/Paste | Rename/Delete | Cancel).
    fn show_command_bar(
        &mut self, ui: &mut egui::Ui,
        can_act: bool, can_export: bool, has_sel: bool, sel_has_dir: bool,
        bundle_id: &str, s: &crate::i18n::S,
    ) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(can_act, |ui| {
                if cmd_btn(ui, ICON_UPLOAD, s.lbl_upload)
                    .on_hover_text(s.tip_upload).clicked()
                {
                    if let Some(paths) = rfd::FileDialog::new()
                        .set_title(s.tip_upload)
                        .add_filter(s.no_files, &["*"])
                        .pick_files()
                    {
                        let cancel = self.new_cancel_flag();
                        let concurrency = self.settings.concurrency;
                        let cur = self.current_path.clone();
                        self.cmd_tx.send(DeviceCommand::UploadFiles {
                            bundle_id: bundle_id.to_owned(), current_path: cur, paths, cancel, concurrency,
                        }).ok();
                    }
                }
            });
            ui.add_enabled_ui(can_export, |ui| {
                if cmd_btn(ui, ICON_DOWNLOAD, s.lbl_export)
                    .on_hover_text(s.tip_export).clicked()
                {
                    if let Some(dest) = rfd::FileDialog::new().set_title(s.tip_export).pick_folder() {
                        self.trigger_export(bundle_id, dest);
                    }
                }
            });
            ui.add_enabled_ui(can_act, |ui| {
                if cmd_btn(ui, ICON_CREATE_NEW_FOLDER, s.lbl_new_folder)
                    .on_hover_text(s.tip_new_folder).clicked() { self.trigger_new_folder(); }
            });
            ui.add_enabled_ui(can_act, |ui| {
                if cmd_btn(ui, ICON_CONTENT_PASTE, s.lbl_paste)
                    .on_hover_text(s.tip_paste).clicked() { self.trigger_paste(); }
            });
            ui.separator();
            ui.add_enabled_ui(can_act && has_sel && self.selected_files.len() == 1 && !sel_has_dir, |ui| {
                if cmd_btn(ui, ICON_DRIVE_FILE_RENAME_OUTLINE, s.lbl_rename)
                    .on_hover_text(s.tip_rename).clicked() { self.trigger_rename(); }
            });
            ui.add_enabled_ui(can_act && has_sel, |ui| {
                if cmd_btn(ui, ICON_DELETE, s.lbl_delete)
                    .on_hover_text(s.tip_delete).clicked() { self.trigger_delete_confirm(); }
            });
            if self.has_active_transfers() {
                ui.separator();
                if let Some(cancel) = self.transfer_cancel.clone() {
                    if cmd_btn(ui, ICON_CANCEL, s.lbl_cancel)
                        .on_hover_text(s.tip_cancel).clicked()
                    { cancel.store(true, Ordering::Relaxed); }
                }
            }
        });
        ui.separator();
    }

    /// Status bar row: current operation (left) and selection count (right).
    fn show_status_bar(&mut self, ui: &mut egui::Ui, s: &crate::i18n::S) {
        // TODO: add a TextEdit here when re-enabling the search filter.
        // Disabled: DEL key incorrectly triggers file deletion when the filter is focused.
        ui.horizontal(|ui| {
            let status_label: Option<String> = if self.has_active_transfers() {
                let active_count = self.transfers.iter()
                    .filter(|t| matches!(t.status, TransferStatus::Active)).count();
                let parallel_str = if active_count > 1 { active_count.to_string() } else { "-".to_string() };
                let done_count   = self.transfers.iter().filter(|t| matches!(t.status, TransferStatus::Done)).count();
                let total        = self.transfers.len();
                Some(format!("{}{}{} {}/{}{}", s.xfer_active_pre, parallel_str, s.xfer_active_suf, done_count, total, s.items_unit))
            } else {
                self.current_op_label(s).map(|l| l.to_string())
            };
            if let Some(label) = status_label {
                ui.spinner();
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_gray(120), label);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let total_count = self.files.len();
                ui.colored_label(egui::Color32::from_gray(150),
                    format!("{}{}", total_count, s.items_unit));
                if !self.selected_files.is_empty() {
                    ui.colored_label(W11_ACCENT,
                        format!("{}{}", self.selected_files.len(), s.selected_suffix));
                }
            });
        });
        ui.separator();
    }

    /// Renders the file table and returns deferred actions (avoids &mut self conflicts during egui callbacks).
    fn build_file_table(
        &self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        display_files: &[&FileEntry],
        file_list_h: f32,
        can_act: bool,
        has_sel: bool,
        sel_has_dir: bool,
        s: &crate::i18n::S,
    ) -> FilePanelActions {
        if let FileLoadState::Error(e) = &self.file_load_state {
            ui.colored_label(egui::Color32::RED, format!("{}{}", s.error_prefix, e.clone()));
            return FilePanelActions::default();
        }

        let sc = self.sort_column;
        let sa = self.sort_ascending;
        let selected_files = &self.selected_files;
        let file_info = &self.file_info;
        let prev_hovered = self.hovered_row.get();
        let new_hovered  = std::cell::Cell::new(None::<usize>);

        let mut do_delete     = false;
        let mut do_export     = false;
        let mut do_rename     = false;
        let mut do_new_folder = false;
        let mut enter_folder: Option<String>             = None;
        let mut sort_click:   Option<SortColumn>         = None;
        let mut sel_action:   Option<(usize, String, bool, bool)> = None;
        let mut right_click_name: Option<String>         = None;

        let table_x_range = ui.available_rect_before_wrap().x_range();

        TableBuilder::new(ui)
            .id_salt("file_table")
            .striped(false)
            .resizable(true)
            .max_scroll_height(file_list_h)
            .column(Column::remainder().at_least(120.0))
            .column(Column::initial(62.0).resizable(true).at_least(50.0))
            .column(Column::initial(85.0).resizable(true).at_least(60.0))
            .column(Column::initial(138.0).resizable(true).at_least(100.0))
            .header(22.0, |mut header| {
                let arrow = |col: SortColumn| if sc == col { if sa { " ▲" } else { " ▼" } } else { "" };
                macro_rules! hdr_col {
                    ($label:expr, $col:expr) => {
                        header.col(|ui| {
                            let r = ui.add(egui::Label::new(
                                egui::RichText::new(format!("{}{}", $label, arrow($col))).strong()
                            ).sense(egui::Sense::click()));
                            if r.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                            if r.clicked() { sort_click = Some($col); }
                        });
                    }
                }
                hdr_col!(s.col_name, SortColumn::Name);
                hdr_col!(s.col_kind, SortColumn::Kind);
                header.col(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.add(egui::Label::new(
                            egui::RichText::new(format!("{}{}", s.col_size, arrow(SortColumn::Size))).strong()
                        ).sense(egui::Sense::click()));
                        if r.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                        if r.clicked() { sort_click = Some(SortColumn::Size); }
                    });
                });
                hdr_col!(s.col_modified, SortColumn::Modified);
            })
            .body(|mut body| {
                for (i, entry) in display_files.iter().enumerate() {
                    let is_sel = selected_files.contains(&entry.name);
                    let mut row_sel: Option<(bool, bool)> = None;
                    let mut row_dbl = false;
                    let row_right = std::cell::Cell::new(false);
                    let row_del   = std::cell::Cell::new(false);
                    let row_exp   = std::cell::Cell::new(false);
                    let row_ren   = std::cell::Cell::new(false);
                    let row_nfld  = std::cell::Cell::new(false);

                    let mk = || {
                        let nd = &row_nfld; let dd = &row_del;
                        let ed = &row_exp;  let rd = &row_ren;
                        move |ui: &mut egui::Ui| {
                            ui.set_min_width(160.0);
                            if can_act {
                                if ui.button(s.ctx_new_folder).clicked() { nd.set(true); ui.close(); }
                                ui.separator();
                                if ui.add_enabled(has_sel, egui::Button::new(s.ctx_export)).clicked() { ed.set(true); ui.close(); }
                                if ui.add_enabled(selected_files.len() == 1 && !sel_has_dir, egui::Button::new(s.ctx_rename)).clicked() { rd.set(true); ui.close(); }
                                ui.separator();
                                if ui.add_enabled(has_sel, egui::Button::new(s.ctx_delete)).clicked() { dd.set(true); ui.close(); }
                            }
                        }
                    };

                    let show_hover = !is_sel && prev_hovered == Some(i);
                    body.row(26.0, |mut row| {
                        row.set_selected(is_sel);
                        row.col(|ui| {
                            let full_rect = ui.max_rect();
                            // Reserve bg slot before label — full row width via with_clip_rect
                            let bg_slot = ui.painter().add(egui::Shape::Noop);
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            let icon = file_icon(entry);
                            let text_color = if is_sel { W11_SEL_TEXT } else { ui.visuals().text_color() };
                            let mut job = egui::text::LayoutJob::default();
                            job.wrap.max_rows = 1;
                            job.wrap.overflow_character = Some('…');
                            job.wrap.break_anywhere = false;
                            job.append(icon.codepoint, 0.0, egui::TextFormat {
                                font_id: egui::FontId::new(18.0, icon.font_family()),
                                color: text_color,
                                valign: egui::Align::Center, ..Default::default()
                            });
                            job.append(&format!("  {}", entry.name), 0.0, egui::TextFormat {
                                font_id: egui::FontId::proportional(14.0),
                                color: text_color,
                                valign: egui::Align::Center, ..Default::default()
                            });
                            ui.add(egui::Label::new(job));
                            let r = ui.interact(full_rect, egui::Id::new(("row_col1", i)), egui::Sense::click());
                            if r.hovered() { new_hovered.set(Some(i)); }
                            if show_hover {
                                let row_rect = egui::Rect::from_x_y_ranges(table_x_range, full_rect.y_range());
                                ui.painter().with_clip_rect(row_rect).set(bg_slot, egui::Shape::rect_filled(row_rect, 0.0, W11_HOVER));
                            }
                            if r.secondary_clicked() && !is_sel { row_right.set(true); }
                            if r.double_clicked() && entry.is_dir { row_dbl = true; }
                            else if r.clicked() {
                                row_sel = Some((
                                    ui.input(|inp| inp.modifiers.ctrl),
                                    ui.input(|inp| inp.modifiers.shift),
                                ));
                            }
                            r.context_menu(mk());
                        });
                        row.col(|ui| {
                            let full_rect = ui.max_rect();
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.colored_label(egui::Color32::GRAY, entry_kind(entry, s.folder_kind));
                            let r = ui.interact(full_rect, egui::Id::new(("row_col2", i)), egui::Sense::click());
                            if r.hovered() { new_hovered.set(Some(i)); }
                            if r.secondary_clicked() && !is_sel { row_right.set(true); }
                            else if r.clicked() {
                                row_sel = Some((
                                    ui.input(|inp| inp.modifiers.ctrl),
                                    ui.input(|inp| inp.modifiers.shift),
                                ));
                            }
                            r.context_menu(mk());
                        });
                        row.col(|ui| {
                            let full_rect = ui.max_rect();
                            if let Some((size, _)) = file_info.get(&entry.name) {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if entry.is_dir { ui.label("—"); } else { ui.label(format_size(*size)); }
                                });
                            }
                            let r = ui.interact(full_rect, egui::Id::new(("row_col3", i)), egui::Sense::click());
                            if r.hovered() { new_hovered.set(Some(i)); }
                            if r.secondary_clicked() && !is_sel { row_right.set(true); }
                            else if r.clicked() {
                                row_sel = Some((
                                    ui.input(|inp| inp.modifiers.ctrl),
                                    ui.input(|inp| inp.modifiers.shift),
                                ));
                            }
                            r.context_menu(mk());
                        });
                        row.col(|ui| {
                            let full_rect = ui.max_rect();
                            if let Some((_, m)) = file_info.get(&entry.name) { ui.label(m.as_str()); }
                            let r = ui.interact(full_rect, egui::Id::new(("row_col4", i)), egui::Sense::click());
                            if r.hovered() { new_hovered.set(Some(i)); }
                            if r.secondary_clicked() && !is_sel { row_right.set(true); }
                            else if r.clicked() {
                                row_sel = Some((
                                    ui.input(|inp| inp.modifiers.ctrl),
                                    ui.input(|inp| inp.modifiers.shift),
                                ));
                            }
                            r.context_menu(mk());
                        });
                    });

                    if row_dbl { enter_folder = Some(entry.name.clone()); }
                    if let Some((ctrl, shift)) = row_sel {
                        sel_action = Some((i, entry.name.clone(), ctrl, shift));
                    }
                    if row_right.get() { right_click_name = Some(entry.name.clone()); }
                    if row_del.get()   { do_delete    = true; }
                    if row_exp.get()   { do_export    = true; }
                    if row_ren.get()   { do_rename    = true; }
                    if row_nfld.get()  { do_new_folder = true; }
                }
            });
        self.hovered_row.set(new_hovered.get());

        // empty folder / background area below the table — context menu
        if display_files.is_empty() {
            // TODO: when the filter is re-enabled, switch between no_match / no_files based on filter_lower
            let msg = s.no_files;
            let avail = ui.available_rect_before_wrap();
            let resp = ui.allocate_rect(avail, egui::Sense::click());
            ui.painter().text(avail.center(), egui::Align2::CENTER_CENTER, msg,
                egui::FontId::proportional(13.0), egui::Color32::GRAY);
            resp.context_menu(|ui| {
                ui.set_min_width(160.0);
                if can_act {
                    if ui.button(s.ctx_new_folder).clicked() { do_new_folder = true; ui.close(); }
                }
            });
        } else {
            let bg_rect = ui.available_rect_before_wrap();
            if bg_rect.height() > 0.0 {
                let bg_resp = ui.allocate_rect(bg_rect, egui::Sense::click());
                bg_resp.context_menu(|ui| {
                    ui.set_min_width(160.0);
                    if can_act {
                        if ui.button(s.ctx_new_folder).clicked() { do_new_folder = true; ui.close(); }
                        ui.separator();
                        if ui.add_enabled(has_sel, egui::Button::new(s.ctx_export)).clicked() { do_export = true; ui.close(); }
                        if ui.add_enabled(has_sel && selected_files.len() == 1 && !sel_has_dir, egui::Button::new(s.ctx_rename)).clicked() { do_rename = true; ui.close(); }
                        ui.separator();
                        if ui.add_enabled(has_sel, egui::Button::new(s.ctx_delete)).clicked() { do_delete = true; ui.close(); }
                    }
                });
            }
        }

        FilePanelActions {
            enter_folder,
            delete: do_delete,
            rename: do_rename,
            new_folder: do_new_folder,
            export: do_export,
            sort_click,
            right_click_name,
            sel_action,
        }
    }

    /// Applies selection changes for Shift, Ctrl, or plain click.
    fn apply_selection(
        &mut self,
        sel: (usize, String, bool, bool),
        display_names: &[String],
    ) {
        let (idx, name, ctrl, shift) = sel;
        if shift {
            let anchor_i = self.anchor_file.as_deref()
                .and_then(|af| display_names.iter().position(|n| n == af))
                .unwrap_or(idx);
            let lo = anchor_i.min(idx);
            let hi = anchor_i.max(idx).min(display_names.len().saturating_sub(1));
            if !ctrl { self.selected_files.clear(); }
            for j in lo..=hi { self.selected_files.insert(display_names[j].clone()); }
        } else if ctrl {
            if self.selected_files.contains(&name) { self.selected_files.remove(&name); }
            else { self.selected_files.insert(name.clone()); self.anchor_file = Some(name); }
        } else {
            self.selected_files.clear();
            self.selected_files.insert(name.clone());
            self.anchor_file = Some(name);
        }
    }

    /// Central panel: file list with toolbar, address bar, and action buttons.
    fn show_file_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, is_busy: bool) {
        let s = crate::i18n::strings(self.settings.lang);

        let Some(idx) = self.selected_app_idx else {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::GRAY, s.select_app);
            });
            return;
        };

        let app_name  = self.apps[idx].display_name.clone();
        let bundle_id = self.apps[idx].bundle_id.clone();
        let can_act   = matches!(self.device_status, DeviceStatus::Connected { .. }) && !is_busy;
        let has_sel   = !self.selected_files.is_empty();
        let sel_has_dir = self.selected_files.iter()
            .any(|n| self.files.iter().any(|e| &e.name == n && e.is_dir));
        let can_export = can_act && has_sel;

        // ── Row 1: navigation + address bar ──────────────────────────────
        self.show_navigation_bar(ui, can_act, &app_name, s);
        ui.add_space(4.0);

        // ── Row 2: command bar (actions) ──────────────────────────────────
        self.show_command_bar(ui, can_act, can_export, has_sel, sel_has_dir, &bundle_id, s);

        // ── Status row: current operation (left) / count (right) ──────────
        self.show_status_bar(ui, s);

        // ── Sorted file list ──────────────────────────────────────────────
        // TODO: filter with filter_lower when search filter is re-enabled.
        let mut display_files: Vec<&FileEntry> = self.files.iter().collect();
        {
            let sort_col = self.sort_column;
            let sort_asc = self.sort_ascending;
            let fi = &self.file_info;
            let fk = s.folder_kind;
            display_files.sort_by(|a, b| {
                let dir_ord = b.is_dir.cmp(&a.is_dir);
                if dir_ord != std::cmp::Ordering::Equal { return dir_ord; }
                let ord = match sort_col {
                    SortColumn::Name     => a.name.cmp(&b.name),
                    SortColumn::Kind     => entry_kind(a, fk).cmp(&entry_kind(b, fk)),
                    SortColumn::Size     => fi.get(&a.name).map(|(s,_)| *s).unwrap_or(0)
                                              .cmp(&fi.get(&b.name).map(|(s,_)| *s).unwrap_or(0)),
                    SortColumn::Modified => fi.get(&a.name).map(|(_, m)| m.as_str()).unwrap_or("")
                                              .cmp(fi.get(&b.name).map(|(_, m)| m.as_str()).unwrap_or("")),
                };
                if sort_asc { ord } else { ord.reverse() }
            });
        }
        let display_names: Vec<String> = display_files.iter().map(|e| e.name.clone()).collect();

        // ── Arrow key navigation ──────────────────────────────────────────
        if !is_busy && !display_names.is_empty() {
            let (arrow_up, arrow_down, shift) = ctx.input(|i| (
                i.key_pressed(egui::Key::ArrowUp)   && !i.modifiers.alt,
                i.key_pressed(egui::Key::ArrowDown) && !i.modifiers.alt,
                i.modifiers.shift,
            ));
            if arrow_up || arrow_down {
                let anchor_idx = self.anchor_file.as_deref()
                    .and_then(|af| display_names.iter().position(|n| n == af));
                let cursor_idx = if shift && anchor_idx.is_some() {
                    // Shift: cursor is the selection end farthest from the anchor
                    let ai = anchor_idx.unwrap();
                    let sel_min = self.selected_files.iter()
                        .filter_map(|f| display_names.iter().position(|n| n == f))
                        .min().unwrap_or(ai);
                    let sel_max = self.selected_files.iter()
                        .filter_map(|f| display_names.iter().position(|n| n == f))
                        .max().unwrap_or(ai);
                    if ai <= sel_min { sel_max } else { sel_min }
                } else {
                    anchor_idx.or_else(|| self.selected_files.iter().next()
                        .and_then(|f| display_names.iter().position(|n| n == f)))
                        .unwrap_or(0)
                };
                let new_cursor = if arrow_up {
                    cursor_idx.saturating_sub(1)
                } else {
                    (cursor_idx + 1).min(display_names.len().saturating_sub(1))
                };
                if shift {
                    // Shift+arrow: extend selection while keeping anchor fixed
                    let ai = anchor_idx.unwrap_or(new_cursor);
                    let lo = ai.min(new_cursor);
                    let hi = ai.max(new_cursor);
                    self.selected_files.clear();
                    for name in &display_names[lo..=hi] {
                        self.selected_files.insert(name.clone());
                    }
                    // anchor_file stays unchanged
                } else {
                    self.selected_files.clear();
                    self.selected_files.insert(display_names[new_cursor].clone());
                    self.anchor_file = Some(display_names[new_cursor].clone());
                }
            }
        }

        let file_list_h = ui.available_height().max(40.0);

        // ── Header + file list ───────────────────────────────────────────
        let actions = self.build_file_table(
            ui, ctx, &display_files, file_list_h,
            can_act, has_sel, sel_has_dir, s,
        );

        // ── Apply deferred actions ────────────────────────────────────────
        if let Some(col)  = actions.sort_click       { self.toggle_sort(col); }
        if let Some(name) = actions.right_click_name {
            self.selected_files.clear();
            self.selected_files.insert(name.clone());
            self.anchor_file = Some(name);
        }
        if let Some(sel) = actions.sel_action {
            self.apply_selection(sel, &display_names);
        }
        if let Some(name) = actions.enter_folder {
            self.navigate_to(format!("{}/{}", self.current_path, name));
        }
        if actions.delete    { self.trigger_delete_confirm(); }
        if actions.rename    { self.trigger_rename(); }
        if actions.new_folder { self.trigger_new_folder(); }
        if actions.export && can_export {
            if let Some(dest) = rfd::FileDialog::new().set_title(s.tip_export).pick_folder() {
                self.trigger_export(&bundle_id, dest);
            }
        }
    }

    /// Settings dialog.
    fn show_settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_settings { return; }
        let s = crate::i18n::strings(self.settings.lang);

        let mut do_ok     = false;
        let mut do_cancel = false;
        let mut do_reset  = false;
        let mut window_open = true;  // detect window X-button close

        egui::Window::new(s.settings_title)
            .open(&mut window_open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([20.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(s.settings_language);
                        egui::ComboBox::new("lang_combo", "")
                            .selected_text(self.settings.lang.label())
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                for lang in [
                                    crate::i18n::Lang::En,
                                    crate::i18n::Lang::ZhCn,
                                    crate::i18n::Lang::Ja,
                                ] {
                                    if ui.selectable_label(self.settings.lang == lang, lang.label()).clicked() {
                                        self.settings.lang = lang;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label(s.settings_threads);
                        ui.add(egui::Slider::new(&mut self.settings.concurrency, 1..=8));
                        ui.end_row();
                    });

                if ui.input(|i| i.key_pressed(egui::Key::Escape)) { do_cancel = true; }

                ui.separator();
                ui.add_space(4.0);

                // [Reset defaults] left-aligned ... [Cancel][OK] right-aligned
                ui.horizontal(|ui| {
                    if ui.button(s.settings_reset_defaults).clicked() { do_reset = true; }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, s.btn_ok).clicked()      { do_ok     = true; }
                        if dialog_button(ui, s.btn_cancel).clicked()   { do_cancel = true; }
                    });
                });
                ui.add_space(2.0);
            });

        // closing with X is treated as Cancel
        if !window_open { do_cancel = true; }

        if do_ok {
            self.settings_snapshot = None;
            self.settings.save();
            self.show_settings = false;
        } else if do_cancel {
            // restore from snapshot
            if let Some((lang, concurrency)) = self.settings_snapshot.take() {
                self.settings.lang = lang;
                self.settings.concurrency = concurrency;
            }
            self.show_settings = false;
        } else if do_reset {
            self.settings.lang        = crate::i18n::Lang::default();
            self.settings.concurrency = 2;
            // font is set once at startup — do not change on language switch
            // changes take effect when OK is clicked
        }
    }

    /// Delete / rename / new folder dialogs.
    fn show_dialogs(&mut self, ctx: &egui::Context) {
        let s = crate::i18n::strings(self.settings.lang);
        self.show_delete_dialog(ctx, s);
        self.show_rename_dialog(ctx, s);
        self.show_new_folder_dialog(ctx, s);
        self.show_settings_dialog(ctx);
    }

    /// Delete confirmation dialog.
    fn show_delete_dialog(&mut self, ctx: &egui::Context, s: &crate::i18n::S) {
        if !self.show_delete_confirm { return; }
        let bid = self.selected_bundle_id();
        let cur = self.delete_base_path.clone();
        let mut do_delete  = false;
        let mut cancel_del = false;

        egui::Window::new(s.del_confirm_title)
            .collapsible(false).resizable(false)
            .default_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let n       = self.files_to_delete.len();
                let has_dir = self.files_to_delete.iter().any(|(_, d)| *d);
                if n == 1 {
                    ui.label(egui::RichText::new(s.del_confirm_single).size(14.0));
                    if has_dir {
                        ui.add_space(2.0);
                        ui.colored_label(egui::Color32::from_rgb(196, 120, 0), s.del_warn_folder);
                    }
                    ui.add_space(4.0);
                    if let Some((name, is_dir)) = self.files_to_delete.first() {
                        let icon = if *is_dir { ICON_FOLDER } else { ICON_INSERT_DRIVE_FILE };
                        ui.horizontal(|ui| {
                            ui.label(icon.rich_text().size(15.0));
                            ui.label(name.as_str());
                        });
                    }
                } else {
                    let msg = format!("{}{}{}", s.del_confirm_pre_multi, n, s.del_confirm_suf_multi);
                    ui.label(egui::RichText::new(msg).size(14.0));
                    if has_dir {
                        ui.add_space(2.0);
                        ui.colored_label(egui::Color32::from_rgb(196, 120, 0), s.del_warn_folder);
                    }
                }
                ui.add_space(4.0);

                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) { cancel_del = true; }

                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if danger_button(ui, s.btn_delete).clicked() || enter { do_delete  = true; }
                        if dialog_button(ui, s.btn_cancel).clicked()          { cancel_del = true; }
                    });
                });
                ui.add_space(2.0);
            });

        if do_delete {
            if let Some(bundle_id) = bid {
                let abs_paths: Vec<String> = self.files_to_delete.iter()
                    .map(|(n, _)| format!("{cur}/{n}"))
                    .collect();
                self.clear_transfers_if_idle();
                self.is_deleting = true;
                self.cmd_tx.send(DeviceCommand::DeleteFiles {
                    bundle_id, current_path: cur, abs_paths,
                }).ok();
                self.selected_files.clear();
            }
            self.files_to_delete.clear();
            self.show_delete_confirm = false;
        } else if cancel_del {
            self.files_to_delete.clear();
            self.show_delete_confirm = false;
        }
    }

    /// Rename dialog.
    fn show_rename_dialog(&mut self, ctx: &egui::Context, s: &crate::i18n::S) {
        if self.renaming_file.is_none() { return; }
        let bid = self.selected_bundle_id();
        let cur = self.rename_base_path.clone();
        let mut submit        = false;
        let mut cancel_rename = false;
        let rename_edit_id    = egui::Id::new("rename_text_edit");

        egui::Window::new(s.rename_title)
            .collapsible(false).resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("rename_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(s.rename_current_prefix)
                            .color(egui::Color32::from_gray(130)).size(12.0));
                        if let Some(old) = &self.renaming_file {
                            ui.label(egui::RichText::new(old.as_str())
                                .color(egui::Color32::from_gray(100)).size(12.0));
                        }
                        ui.end_row();
                    });
                ui.add_space(4.0);
                if self.rename_just_opened {
                    let stem_byte  = self.rename_input.rfind('.').unwrap_or(self.rename_input.len());
                    let stem_chars = self.rename_input[..stem_byte].chars().count();
                    let mut state  = egui::text_edit::TextEditState::load(ctx, rename_edit_id).unwrap_or_default();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(stem_chars),
                    )));
                    state.store(ctx, rename_edit_id);
                }
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_input)
                        .id(rename_edit_id)
                        .desired_width(f32::INFINITY),
                );
                if self.rename_just_opened { resp.request_focus(); self.rename_just_opened = false; }
                // restore focus if lost due to IME commit (Enter)
                if resp.lost_focus()
                    && !ctx.input(|i| i.pointer.any_pressed())
                    && !ctx.input(|i| i.key_pressed(egui::Key::Escape))
                { resp.request_focus(); }

                let valid = !self.rename_input.trim().is_empty()
                    && Some(self.rename_input.trim().to_string()) != self.renaming_file;
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) { cancel_rename = true; }

                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let apply = if valid {
                            primary_button(ui, s.btn_change)
                        } else {
                            ui.add_enabled(false, egui::Button::new(s.btn_change)
                                .min_size(egui::Vec2::new(80.0, 28.0)))
                        };
                        if apply.clicked() || (valid && enter) { submit = true; }
                        if dialog_button(ui, s.btn_cancel).clicked() { cancel_rename = true; }
                    });
                });
                ui.add_space(2.0);
            });

        if submit {
            if let (Some(old), Some(bundle_id)) = (self.renaming_file.take(), bid) {
                let new_name = self.rename_input.trim().to_string();
                let old_abs  = format!("{cur}/{old}");
                let new_abs  = format!("{cur}/{new_name}");
                self.clear_transfers_if_idle();
                self.is_renaming = true;
                self.cmd_tx.send(DeviceCommand::RenameFile {
                    bundle_id, current_path: cur, old_abs, new_abs,
                }).ok();
            }
            self.rename_input.clear();
        } else if cancel_rename {
            self.renaming_file      = None;
            self.rename_input.clear();
            self.rename_just_opened = false;
        }
    }

    /// New folder dialog.
    fn show_new_folder_dialog(&mut self, ctx: &egui::Context, s: &crate::i18n::S) {
        if !self.show_new_folder { return; }
        let bid = self.selected_bundle_id();
        let cur = self.new_folder_base_path.clone();
        let mut submit     = false;
        let mut cancel_new = false;
        let new_folder_id  = egui::Id::new("new_folder_input");

        egui::Window::new(s.new_folder_title)
            .collapsible(false).resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(s.new_folder_hint).color(egui::Color32::from_gray(100)).size(12.0));
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.new_folder_input)
                        .id(new_folder_id).desired_width(f32::INFINITY),
                );
                if self.new_folder_just_opened { resp.request_focus(); self.new_folder_just_opened = false; }
                if resp.lost_focus()
                    && !ctx.input(|i| i.pointer.any_pressed())
                    && !ctx.input(|i| i.key_pressed(egui::Key::Escape))
                { resp.request_focus(); }

                let valid = !self.new_folder_input.trim().is_empty();
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) { cancel_new = true; }

                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let create = if valid {
                            primary_button(ui, s.btn_create)
                        } else {
                            ui.add_enabled(false, egui::Button::new(s.btn_create)
                                .min_size(egui::Vec2::new(80.0, 28.0)))
                        };
                        if create.clicked() || (valid && enter) { submit = true; }
                        if dialog_button(ui, s.btn_cancel).clicked() { cancel_new = true; }
                    });
                });
                ui.add_space(2.0);
            });

        if submit {
            if let Some(bundle_id) = bid {
                let name     = self.new_folder_input.trim().to_string();
                let new_path = format!("{cur}/{name}");
                self.cmd_tx.send(DeviceCommand::MkDir {
                    bundle_id, current_path: cur, new_path,
                }).ok();
                self.is_creating_folder = true;
            }
            self.new_folder_input.clear();
            self.show_new_folder = false;
        } else if cancel_new {
            self.new_folder_input.clear();
            self.show_new_folder = false;
        }
    }
}

// ─── Sidebar row helper (free function for borrow separation) ────────────────
/// Renders one pill-shaped sidebar row. Uses painter only (no widget allocation),
/// so the entire row is clickable. Returns (row_response, star_was_clicked).
fn sidebar_row(
    ui:           &mut egui::Ui,
    bundle_id:    &str,
    display_name: &str,
    selected:     bool,
    is_fav:       bool,
    texture:      Option<&egui::TextureHandle>,
    section:      &str,
) -> (egui::Response, bool) {
    let row_h  = 36.0;
    let full_w = ui.available_width();
    let (row_rect, row_resp) = ui.allocate_exact_size(
        egui::Vec2::new(full_w, row_h),
        egui::Sense::click(),
    );

    // pill background
    let pill = row_rect.shrink2(egui::Vec2::new(4.0, 2.0));
    let bg   = if selected               { W11_SEL }
               else if row_resp.hovered() { W11_HOVER }
               else                       { egui::Color32::TRANSPARENT };
    if bg != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(pill, 4.0, bg);
    }
    // selected: 3 px accent line on the left edge
    if selected {
        let accent = egui::Rect::from_min_size(
            pill.min + egui::Vec2::new(0.0, 4.0),
            egui::Vec2::new(3.0, pill.height() - 8.0),
        );
        ui.painter().rect_filled(accent, 2.0, W11_ACCENT);
    }

    let text_color   = if selected { W11_SEL_TEXT } else { W11_TEXT };
    let content_rect = pill.shrink2(egui::Vec2::new(8.0, 0.0));
    let cy           = row_rect.center().y;

    // ── star / outline-star (24 px on left) — painter only, detected via Sense::click()
    let star_w    = 24.0;
    let star_rect = egui::Rect::from_min_size(
        content_rect.min,
        egui::Vec2::new(star_w, row_h),
    );
    let star_color = if is_fav {
        egui::Color32::from_rgb(245, 180, 0)
    } else {
        egui::Color32::from_gray(200)
    };
    ui.painter().text(
        egui::pos2(star_rect.center().x, cy + 4.0),
        egui::Align2::CENTER_CENTER,
        if is_fav { "★" } else { "☆" },
        egui::FontId::proportional(22.0),
        star_color,
    );
    let star_id   = egui::Id::new((bundle_id, section, "star"));
    let star_resp = ui.interact(star_rect, star_id, egui::Sense::click());

    // ── app icon (offset 24+4=28 px, 24×24)
    let icon_x = content_rect.min.x + star_w + 4.0;
    if let Some(tex) = texture {
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(icon_x + 12.0, cy),
            egui::Vec2::splat(24.0),
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(tex.id(), icon_rect, uv, egui::Color32::WHITE);
    }

    // ── app name (4 px after icon, clipped to prevent overflow)
    let name_x    = icon_x + 28.0;
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(name_x, content_rect.min.y),
        content_rect.max,
    );
    ui.painter().with_clip_rect(name_rect).text(
        egui::pos2(name_x, cy),
        egui::Align2::LEFT_CENTER,
        display_name,
        egui::FontId::proportional(13.0),
        text_color,
    );

    (row_resp, star_resp.clicked())
}

// ─── eframe::App ─────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages();

        // persist window position
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.settings.window_x = Some(rect.min.x);
            self.settings.window_y = Some(rect.min.y);
            self.settings.window_w = Some(rect.width());
            self.settings.window_h = Some(rect.height());
        }

        // expire status message after 4 seconds
        if let Some((_, t)) = &self.status_msg {
            if t.elapsed().as_secs() >= 4 { self.status_msg = None; }
        }

        let is_busy = self.is_busy();
        if is_busy { ctx.request_repaint(); }

        self.handle_shortcuts(ctx, is_busy);

        #[allow(deprecated)]
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Panel::top("status_bar").show_inside(ui, |ui| self.show_toolbar(ui));
            egui::Panel::left("app_panel")
                .resizable(true).default_size(230.0).min_size(160.0)
                .frame(egui::Frame::new()
                    .fill(W11_PANEL)
                    .inner_margin(egui::Margin::symmetric(0, 4)))
                .show_inside(ui, |ui| self.show_sidebar(ui));
            egui::Panel::bottom("transfer_panel")
                .show_inside(ui, |ui| {
                    if !self.transfers.is_empty() {
                        self.show_transfer_panel(ui);
                    } else {
                        // Always reserve the same height as the ProgressBar to prevent layout shift
                        let h = ui.spacing().interact_size.y;
                        ui.allocate_space(egui::Vec2::new(ui.available_width(), h));
                    }
                });
            self.show_file_panel(ui, ctx, is_busy);
        });

        self.show_dialogs(ctx);
    }
}
