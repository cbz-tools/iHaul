// ui/helpers.rs — utility functions
use eframe::egui;
use egui_material_icons::icons::{
    ICON_FOLDER, ICON_INSERT_DRIVE_FILE, ICON_FOLDER_ZIP,
    ICON_VIDEOCAM, ICON_IMAGE, ICON_AUDIO_FILE, ICON_PICTURE_AS_PDF,
};
use std::path::PathBuf;
use super::types::FileEntry;

// ─── Win11 color palette ─────────────────────────────────────────────────────

pub(super) const W11_ACCENT:   egui::Color32 = egui::Color32::from_rgb(0, 120, 212);   // #0078D4
pub(super) const W11_SEL:      egui::Color32 = egui::Color32::from_rgb(204, 232, 255);  // #CCE8FF
pub(super) const W11_SEL_TEXT: egui::Color32 = egui::Color32::from_rgb(0, 55, 128);    // #003780
pub(super) const W11_HOVER:    egui::Color32 = egui::Color32::from_rgb(229, 229, 229);  // #E5E5E5
pub(super) const W11_SURFACE:  egui::Color32 = egui::Color32::from_rgb(255, 255, 255);  // #FFFFFF
pub(super) const W11_PANEL:    egui::Color32 = egui::Color32::from_rgb(243, 243, 243);  // #F3F3F3
pub(super) const W11_TOOLBAR:  egui::Color32 = egui::Color32::from_rgb(249, 249, 249);  // #F9F9F9
pub(super) const W11_BORDER:   egui::Color32 = egui::Color32::from_rgb(229, 229, 229);  // #E5E5E5
pub(super) const W11_DANGER:   egui::Color32 = egui::Color32::from_rgb(196, 43, 28);    // #C42B1C
pub(super) const W11_TEXT:     egui::Color32 = egui::Color32::from_rgb(28, 28, 28);     // #1C1C1C
pub(super) const W11_TEXT_SEC: egui::Color32 = egui::Color32::from_rgb(96, 94, 92);    // #605E5C

// ─── Clipboard ───────────────────────────────────────────────────────────────

pub(super) fn read_clipboard_files() -> Vec<PathBuf> {
    use clipboard_win::{formats, Clipboard, Getter};
    if let Ok(_clip) = Clipboard::new_attempts(3) {
        let mut files: Vec<PathBuf> = Vec::new();
        if formats::FileList.read_clipboard(&mut files).is_ok() && !files.is_empty() {
            return files;
        }
        let mut text = String::new();
        if formats::Unicode.read_clipboard(&mut text).is_ok() {
            let paths: Vec<PathBuf> = text.lines()
                .map(|l| PathBuf::from(l.trim().trim_matches('"')))
                .filter(|p| p.exists())
                .collect();
            if !paths.is_empty() { return paths; }
        }
    }
    vec![]
}

// ─── File icons ──────────────────────────────────────────────────────────────

pub(super) fn file_icon(entry: &FileEntry) -> egui_material_icons::MaterialIcon {
    if entry.is_dir { return ICON_FOLDER; }
    let ext = entry.name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "zip" | "cbz" | "rar" | "cbr" | "7z" => ICON_FOLDER_ZIP,
        "mp4" | "mkv" | "avi" | "mov" | "wmv" => ICON_VIDEOCAM,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => ICON_IMAGE,
        "mp3" | "aac" | "flac" | "ogg" | "wav" | "m4a" => ICON_AUDIO_FILE,
        "pdf" => ICON_PICTURE_AS_PDF,
        _ => ICON_INSERT_DRIVE_FILE,
    }
}

/// `folder_label` is provided by the caller from i18n (e.g. `s.folder_kind`).
pub(super) fn entry_kind(entry: &FileEntry, folder_label: &str) -> String {
    if entry.is_dir { return folder_label.to_string(); }
    entry.name.rfind('.').map(|i| entry.name[i+1..].to_uppercase())
        .unwrap_or_else(|| "—".to_string())
}

// ─── Formatting ──────────────────────────────────────────────────────────────

pub(super) fn format_size(bytes: u64) -> String {
    if bytes < 1_024 { format!("{} B", bytes) }
    else if bytes < 1_024 * 1_024 { format!("{:.1} KB", bytes as f64 / 1_024.0) }
    else if bytes < 1_024 * 1_024 * 1_024 { format!("{:.1} MB", bytes as f64 / (1_024.0 * 1_024.0)) }
    else { format!("{:.2} GB", bytes as f64 / (1_024.0 * 1_024.0 * 1_024.0)) }
}

// ─── Win11 theme ─────────────────────────────────────────────────────────────

/// Applies a Windows 11 Explorer-style visual theme. Call once at startup.
pub(super) fn apply_win11_theme(ctx: &egui::Context) {
    let mut vis = egui::Visuals::light();
    let r4 = egui::CornerRadius::same(4);
    let r8 = egui::CornerRadius::same(8);

    // window / dialog
    vis.window_corner_radius = r8;
    vis.window_fill   = W11_SURFACE;
    vis.window_shadow = egui::Shadow { offset: [0, 4].into(), blur: 16, spread: 0,
                                       color: egui::Color32::from_black_alpha(32) };
    // panel background (content area = white)
    vis.panel_fill = W11_SURFACE;

    // widget corner radius (all states 4px)
    vis.widgets.noninteractive.corner_radius = r4;
    vis.widgets.inactive.corner_radius       = r4;
    vis.widgets.hovered.corner_radius        = r4;
    vis.widgets.active.corner_radius         = r4;
    vis.widgets.open.corner_radius           = r4;

    // button / widget colors
    vis.widgets.inactive.weak_bg_fill  = W11_TOOLBAR;
    vis.widgets.inactive.bg_fill       = W11_TOOLBAR;
    vis.widgets.inactive.bg_stroke     = egui::Stroke::new(1.0, W11_BORDER);
    vis.widgets.inactive.fg_stroke     = egui::Stroke::new(1.0, W11_TEXT);
    vis.widgets.hovered.weak_bg_fill   = W11_HOVER;
    vis.widgets.hovered.bg_fill        = W11_HOVER;
    vis.widgets.hovered.bg_stroke      = egui::Stroke::new(1.0, W11_ACCENT);
    vis.widgets.hovered.fg_stroke      = egui::Stroke::new(1.5, W11_TEXT);
    vis.widgets.active.weak_bg_fill    = egui::Color32::from_rgb(204, 228, 247);
    vis.widgets.active.bg_fill         = egui::Color32::from_rgb(204, 228, 247);
    vis.widgets.noninteractive.weak_bg_fill = W11_PANEL;
    vis.widgets.noninteractive.bg_fill      = W11_PANEL;
    vis.widgets.noninteractive.bg_stroke    = egui::Stroke::new(1.0, W11_BORDER);
    vis.widgets.noninteractive.fg_stroke    = egui::Stroke::new(1.0, W11_TEXT_SEC);

    // selection colors (table rows and selectable_label)
    vis.selection.bg_fill = W11_SEL;
    vis.selection.stroke  = egui::Stroke::new(1.0, W11_ACCENT);

    // link / accent color
    vis.hyperlink_color  = W11_ACCENT;

    // scrollbar
    vis.extreme_bg_color = W11_PANEL;

    // menu
    vis.menu_corner_radius = r8;
    vis.popup_shadow = egui::Shadow { offset: [0, 2].into(), blur: 8, spread: 0,
                                      color: egui::Color32::from_black_alpha(24) };

    ctx.set_visuals(vis);

    // spacing
    #[allow(deprecated)]
    let mut style = (*ctx.style()).clone();
    style.spacing.button_padding = egui::Vec2::new(10.0, 5.0);
    style.spacing.item_spacing   = egui::Vec2::new(6.0, 4.0);
    style.spacing.window_margin  = egui::Margin::same(16);
    style.spacing.menu_margin    = egui::Margin::same(4);
    #[allow(deprecated)]
    ctx.set_style(style);
}

// ─── Button helpers ──────────────────────────────────────────────────────────

/// Flat icon button with transparent background; shows a rounded hover highlight.
/// `size` is the side length of the square button in pixels.
pub(super) fn flat_icon_btn(
    ui:   &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    size: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if resp.is_pointer_button_down_on() {
            egui::Color32::from_rgb(204, 228, 247)   // pressed
        } else if resp.hovered() {
            W11_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        if bg != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink(2.0), 4.0, bg);
        }
        let color = if ui.is_enabled() { W11_TEXT } else { egui::Color32::from_gray(180) };
        let icon_sz = (size * 0.62).clamp(16.0, 26.0);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            egui::FontId::new(icon_sz, egui::FontFamily::Name("material-icons".into())),
            color,
        );
    }
    resp
}

/// Icon-only command bar button with transparent background.
/// The `_label` argument is unused; attach `.on_hover_text()` at the call site for a tooltip.
pub(super) fn cmd_btn(
    ui:     &mut egui::Ui,
    icon:   egui_material_icons::MaterialIcon,
    _label: &str,
) -> egui::Response {
    flat_icon_btn(ui, icon, 40.0)
}

/// Red button for destructive actions (delete).
pub(super) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
            .fill(W11_DANGER)
            .stroke(egui::Stroke::new(1.0, W11_DANGER))
            .min_size(egui::Vec2::new(80.0, 28.0)),
    )
}

/// Blue button for primary actions (OK, Create, Apply).
pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
            .fill(W11_ACCENT)
            .stroke(egui::Stroke::new(1.0, W11_ACCENT))
            .min_size(egui::Vec2::new(80.0, 28.0)),
    )
}

/// Standard dialog button (min 80×28 px).
pub(super) fn dialog_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).min_size(egui::Vec2::new(80.0, 28.0)))
}

/// Dialog frame (white background, 8px corner radius, drop shadow).
#[allow(dead_code)]
pub(super) fn dialog_frame() -> egui::Frame {
    egui::Frame::window(&egui::Style::default())
        .fill(W11_SURFACE)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(20))
}

// ─── Font initialization ─────────────────────────────────────────────────────

/// Loads CJK fonts in priority order and registers them for both Proportional and Monospace families.
/// Must be called exactly once at startup (not on language switch).
/// Registers CJK and material-icons fonts in a single set_fonts() call so that
/// the material-icons family has "cjk" as a fallback, eliminating the
/// "Failed to find replacement characters" warning.
pub(super) fn setup_font(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\yugothic.ttc",   // Yu Gothic (Windows 10+)
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\meiryo.ttc",      // Meiryo
        r"C:\Windows\Fonts\msgothic.ttc",    // MS Gothic
    ];
    let mut fonts = egui::FontDefinitions::default();
    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(data).into());
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "cjk".to_owned());
            break;
        }
    }

    // Register material-icons manually (instead of egui_material_icons::initialize)
    // so we can add "cjk" as fallback for replacement characters ('◻', '?').
    let fi = egui_material_icons::font_insert();
    fonts.font_data.insert(fi.name.clone(), fi.data.into());
    fonts.families.insert(
        egui::FontFamily::Name("material-icons".into()),
        vec![fi.name, "cjk".to_owned()],
    );

    ctx.set_fonts(fonts);
    // Note: egui_material_icons::initialize() is intentionally omitted —
    // the font is registered above with the cjk fallback included.
}
