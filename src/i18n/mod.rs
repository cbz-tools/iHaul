// i18n/mod.rs — internationalization
mod en;
mod ja;
mod zh_cn;

#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    #[default]
    En,
    Ja,
    ZhCn,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Self::En   => "English",
            Self::Ja   => "日本語",
            Self::ZhCn => "中文",
        }
    }
}

/// All static UI strings for a single locale.
pub struct S {
    // status bar
    pub waiting:          &'static str,
    pub connect_failed:   &'static str,
    pub error_prefix:     &'static str,
    pub disconnected_msg: &'static str,
    // sidebar
    #[allow(dead_code)]
    pub apps:    &'static str,
    pub no_apps: &'static str,
    pub sidebar_favorites: &'static str,
    pub sidebar_all_apps:  &'static str,
    // file panel
    pub select_app:      &'static str,
    pub no_files:        &'static str,
    #[allow(dead_code)]
    pub no_match:        &'static str,
    #[allow(dead_code)]
    pub filter_hint:     &'static str,
    pub selected_suffix: &'static str,  // " selected" / "件選択中" / "个已选"
    pub items_unit:      &'static str,  // " items" / "件" / "个"
    pub folder_kind:     &'static str,
    // column headers
    pub col_name:     &'static str,
    pub col_kind:     &'static str,
    pub col_size:     &'static str,
    pub col_modified: &'static str,
    // toolbar tooltips
    pub tip_back:       &'static str,
    pub tip_forward:    &'static str,
    pub tip_up:         &'static str,
    pub tip_upload:     &'static str,
    pub tip_new_folder: &'static str,
    pub tip_export:     &'static str,
    pub tip_rename:     &'static str,
    pub tip_delete:     &'static str,
    pub tip_cancel:     &'static str,
    pub tip_paste:      &'static str,
    pub tip_settings:   &'static str,
    // operation labels
    pub op_renaming:        &'static str,
    pub op_deleting:        &'static str,
    pub op_creating_folder: &'static str,
    pub op_loading:         &'static str,
    pub op_preparing:       &'static str,
    // context menu
    pub ctx_new_folder: &'static str,
    pub ctx_export:     &'static str,
    pub ctx_rename:     &'static str,
    pub ctx_delete:     &'static str,
    // buttons
    pub btn_ok:     &'static str,
    pub btn_delete: &'static str,
    pub btn_cancel: &'static str,
    pub btn_change: &'static str,
    pub btn_create: &'static str,
    // delete dialog
    pub del_confirm_title:      &'static str,
    pub del_confirm_single:     &'static str,  // single item: shown as-is
    pub del_confirm_pre_multi:  &'static str,  // 2+: "{pre_multi}{n}{suf_multi}"
    pub del_confirm_suf_multi:  &'static str,
    pub del_warn_folder:        &'static str,
    // rename dialog
    pub rename_title:          &'static str,
    pub rename_current_prefix: &'static str,
    // new folder dialog
    pub new_folder_title: &'static str,
    pub new_folder_hint:  &'static str,
    // transfer progress
    pub failed_suffix:   &'static str,  // "件失敗" / " failed" / "个失败"
    pub per_sec:         &'static str,  // "/s"
    pub xfer_active_pre: &'static str,  // "転送中" / "" / "传输中"
    pub xfer_active_suf: &'static str,  // "" / " active" / ""
    // time units
    pub time_sec:         &'static str,  // "秒" / "s" / "秒"
    pub time_min:         &'static str,  // "分" / "m " / "分"
    pub remaining_prefix: &'static str,  // "残り" / "" / "剩余"
    pub remaining_suffix: &'static str,  // "" / " remaining" / ""
    // command bar short labels
    pub lbl_upload:     &'static str,  // "Upload" / "追加" / "上传"
    pub lbl_export:     &'static str,  // "Export" / "エクスポート" / "导出"
    pub lbl_new_folder: &'static str,  // "New folder" / "新規フォルダ" / "新建"
    pub lbl_rename:     &'static str,  // "Rename" / "変更" / "重命名"
    pub lbl_delete:     &'static str,  // "Delete" / "削除" / "删除"
    pub lbl_paste:      &'static str,  // "Paste" / "貼り付け" / "粘贴"
    pub lbl_cancel:     &'static str,  // "Cancel" / "キャンセル" / "取消"
    // settings
    pub settings_title:         &'static str,
    pub settings_language:      &'static str,
    pub settings_threads:       &'static str,
    pub settings_reset_defaults: &'static str,
}

impl S {
    pub fn format_duration(&self, secs: u64) -> String {
        if secs < 60 {
            format!("{}{}", secs, self.time_sec)
        } else {
            format!("{}{}{}{}", secs / 60, self.time_min, secs % 60, self.time_sec)
        }
    }

    pub fn format_remaining(&self, secs: u64) -> String {
        let dur = self.format_duration(secs);
        format!("{}{}{}", self.remaining_prefix, dur, self.remaining_suffix)
    }
}

pub fn strings(lang: Lang) -> &'static S {
    match lang {
        Lang::En   => &en::S,
        Lang::Ja   => &ja::S,
        Lang::ZhCn => &zh_cn::S,
    }
}
