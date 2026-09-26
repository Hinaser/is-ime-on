//! UI 文字列の日英切り替え。
//! 言語は config の `Language`("auto" | "ja" | "en")で決まり、"auto" は Windows の表示言語が
//! 日本語なら日本語、それ以外は英語。常駐プロセスと設定プロセスはそれぞれ起動時と
//! 設定の再読込時に `set` する。

use std::sync::atomic::{AtomicU8, Ordering};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ja,
    En,
}

/// 言語の設定値。config には小文字の文字列で保存する。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LangSetting {
    Auto,
    Ja,
    En,
}

impl LangSetting {
    pub const ALL: [LangSetting; 3] = [LangSetting::Auto, LangSetting::Ja, LangSetting::En];

    pub fn parse(s: &str) -> LangSetting {
        match s.to_ascii_lowercase().as_str() {
            "ja" => LangSetting::Ja,
            "en" => LangSetting::En,
            _ => LangSetting::Auto,
        }
    }

    pub fn to_config_str(self) -> &'static str {
        match self {
            LangSetting::Auto => "auto",
            LangSetting::Ja => "ja",
            LangSetting::En => "en",
        }
    }

    pub fn resolve(self) -> Lang {
        match self {
            LangSetting::Ja => Lang::Ja,
            LangSetting::En => Lang::En,
            LangSetting::Auto => os_lang(),
        }
    }

    /// 言語選択肢の表示名。言語名は切り替えても読めるよう各言語の自称で書く。
    pub fn label(self) -> &'static str {
        match self {
            LangSetting::Auto => tr().lang_auto,
            LangSetting::Ja => "日本語",
            LangSetting::En => "English",
        }
    }
}

fn os_lang() -> Lang {
    const LANG_JAPANESE: u16 = 0x11;
    // 下位10bitが主言語ID
    if unsafe { GetUserDefaultUILanguage() } & 0x3FF == LANG_JAPANESE {
        Lang::Ja
    } else {
        Lang::En
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Ja
    }
}

pub fn tr() -> &'static Strings {
    match current() {
        Lang::Ja => &JA,
        Lang::En => &EN,
    }
}

pub struct Strings {
    /// ImeMode::index() の順。
    pub mode_names: [&'static str; 7],
    pub already_running: &'static str,
    pub tray_open: &'static str,
    pub tray_reload: &'static str,
    pub tray_pause: &'static str,
    pub tray_exit: &'static str,
    pub tip_paused: &'static str,

    pub window_title: &'static str,
    /// 設定ウィンドウの幅。英語はモード名が長く列がはみ出すため広げる。
    pub window_width: f32,
    pub saved: &'static str,
    pub os_indicator_warning: &'static str,
    pub os_indicator_stop: &'static str,
    pub os_indicator_stopped: &'static str,
    pub open_windows_settings: &'static str,
    pub shape: &'static str,
    pub shape_teardrop: &'static str,
    pub shape_circle: &'static str,
    pub shape_badge: &'static str,
    pub position: &'static str,
    pub position_above: &'static str,
    pub position_below: &'static str,
    pub col_mode: &'static str,
    pub col_visible: &'static str,
    pub col_color: &'static str,
    pub col_size: &'static str,
    pub col_badge_text: &'static str,
    pub col_text_color: &'static str,
    /// 自動 / 白 / 黒
    pub text_colors: [&'static str; 3],
    pub preview: fn(&str) -> String,
    pub preview_sample: &'static str,
    pub trial: &'static str,
    pub trial_hint: &'static str,
    pub presets: &'static str,
    pub preset_none: &'static str,
    pub preset_apply: &'static str,
    pub preset_delete: &'static str,
    pub preset_name_hint: &'static str,
    pub preset_save: &'static str,
    pub preset_applied: fn(&str) -> String,
    pub preset_deleted: fn(&str) -> String,
    pub preset_saved: fn(&str) -> String,
    pub preset_name_required: &'static str,
    pub startup: &'static str,
    pub startup_added: &'static str,
    pub startup_failed: &'static str,
    pub startup_removed: &'static str,
    pub startup_path: fn(&str) -> String,
    pub startup_other_exe: &'static str,
    pub startup_reregister: &'static str,
    pub startup_updated: &'static str,
    pub language: &'static str,
    pub lang_auto: &'static str,
    pub updates: &'static str,
    pub update_check: &'static str,
    pub update_note: &'static str,
    pub check_now: &'static str,
    pub checking: &'static str,
    pub up_to_date: fn(&str) -> String,
    pub update_available: fn(&str) -> String,
    pub open_release_page: &'static str,
    pub update_failed: fn(&str) -> String,
    pub update_winget_hint: &'static str,
    /// トレイメニュー項目(アクセラレータつき)
    pub update_menu: fn(&str) -> String,
    pub update_title: &'static str,
    /// (タグ, winget 管理か)
    pub update_balloon: fn(&str, bool) -> String,
    pub diagnostics: &'static str,
    pub perf_log: &'static str,
    pub perf_log_note: &'static str,
    pub open_log_folder: &'static str,
    pub uninstall: &'static str,
    pub uninstall_winget: &'static str,
    pub uninstall_button: &'static str,
    pub uninstall_note: &'static str,
    pub uninstall_confirm: &'static str,
    pub uninstall_delete: &'static str,
    pub cancel: &'static str,
    pub uninstall_errors: fn(&str) -> String,
}

pub static JA: Strings = Strings {
    mode_names: [
        "IMEオフ",
        "その他",
        "半角英数",
        "半角カタカナ",
        "全角英数",
        "ひらがな",
        "全角カタカナ",
    ],
    already_running: "IsImeOn は既に起動しています。タスクトレイのアイコンから設定を開けます。",
    tray_open: "設定を開く(&O)",
    tray_reload: "設定を再読込(&R)",
    tray_pause: "一時停止(&P)",
    tray_exit: "終了(&X)",
    tip_paused: "(一時停止中)",

    window_title: "IsImeOn — 設定",
    window_width: 500.0,
    saved: "変更を適用・保存しました",
    os_indicator_warning: "⚠ Windows標準のテキストカーソルインジケーターが動作中です。二重表示になるため停止を推奨します。",
    os_indicator_stop: "停止する",
    os_indicator_stopped: "OS標準インジケーターを停止しました",
    open_windows_settings: "Windowsの設定を開く",
    shape: "形状:",
    shape_teardrop: "しずく(OS風・上下)",
    shape_circle: "丸",
    shape_badge: "文字バッジ",
    position: "位置:",
    position_above: "キャレットの上",
    position_below: "キャレットの下",
    col_mode: "モード",
    col_visible: "表示",
    col_color: "色",
    col_size: "サイズ",
    col_badge_text: "バッジ文字",
    col_text_color: "文字色",
    text_colors: ["自動", "白", "黒"],
    preview: |mode| format!("プレビュー: {mode}"),
    preview_sample: "あいうえお",
    trial: "試し打ち(IMEを切り替えて確認):",
    trial_hint: "ここで入力",
    presets: "プリセット",
    preset_none: "(なし)",
    preset_apply: "適用",
    preset_delete: "削除",
    preset_name_hint: "プリセット名",
    preset_save: "現在の設定を保存",
    preset_applied: |name| format!("プリセット「{name}」を適用しました"),
    preset_deleted: |name| format!("プリセット「{name}」を削除しました"),
    preset_saved: |name| format!("プリセット「{name}」を保存しました"),
    preset_name_required: "プリセット名を入力してください",
    startup: "サインイン時に自動起動する",
    startup_added: "スタートアップに登録しました",
    startup_failed: "スタートアップ登録に失敗しました",
    startup_removed: "スタートアップ登録を解除しました",
    startup_path: |path| format!("登録先: {path}"),
    startup_other_exe: "⚠ 別の場所の exe が登録されています(このままでは自動起動しません)",
    startup_reregister: "この exe に登録し直す",
    startup_updated: "登録先を更新しました",
    language: "言語 / Language:",
    lang_auto: "自動(Windows に合わせる)",
    updates: "更新",
    update_check: "更新を自動で確認する(起動時と1日1回)",
    update_note: "有効にすると GitHub(api.github.com)へ最新リリースを問い合わせます。知らせるだけで、ダウンロードや更新は自動では行いません。",
    check_now: "今すぐ確認",
    checking: "確認中…",
    up_to_date: |v| format!("最新版です(v{v})"),
    update_available: |tag| format!("新しいバージョン {tag} があります"),
    open_release_page: "リリースページを開く",
    update_failed: |e| format!("確認できませんでした: {e}"),
    update_winget_hint: "winget で導入されています。`winget upgrade Hinaser.IsImeOn` で更新できます。",
    update_menu: |tag| format!("新しいバージョン {tag} があります(&U)"),
    update_title: "IsImeOn の新しいバージョンがあります",
    update_balloon: |tag, winget| {
        if winget {
            format!("{tag} が公開されました。winget upgrade Hinaser.IsImeOn で更新できます。")
        } else {
            format!("{tag} が公開されました。クリックするとリリースページを開きます。")
        }
    },
    diagnostics: "診断",
    perf_log: "パフォーマンスログを記録する",
    perf_log_note: "1分ごとに処理時間・CPU時間・メモリを日時つきで perf.log に記録します。50ms を超えた処理はその時の前面アプリ名も記録します。アプリがログを外部へ送信することはありません。ログは削除するまで残ります。",
    open_log_folder: "ログのフォルダーを開く",
    uninstall: "アンインストール",
    uninstall_winget: "winget で導入されています。削除は `winget uninstall Hinaser.IsImeOn` を使ってください。",
    uninstall_button: "完全に削除して終了",
    uninstall_note: "設定・自動起動の登録・IsImeOn.exe をすべて削除します",
    uninstall_confirm: "本当に削除しますか?この操作は元に戻せません。",
    uninstall_delete: "削除する",
    cancel: "キャンセル",
    uninstall_errors: |errors| format!("削除できない項目があります: {errors}"),
};

pub static EN: Strings = Strings {
    mode_names: [
        "IME Off",
        "Other",
        "Half-width Alphanumeric",
        "Half-width Katakana",
        "Full-width Alphanumeric",
        "Hiragana",
        "Full-width Katakana",
    ],
    already_running: "IsImeOn is already running. You can open its settings from the icon in the notification area.",
    tray_open: "&Open settings",
    tray_reload: "&Reload settings",
    tray_pause: "&Pause",
    tray_exit: "E&xit",
    tip_paused: " (paused)",

    window_title: "IsImeOn — Settings",
    window_width: 600.0,
    saved: "Changes applied and saved",
    os_indicator_warning: "⚠ The Windows text cursor indicator is on, so two indicators will appear. We recommend turning it off.",
    os_indicator_stop: "Turn off",
    os_indicator_stopped: "Windows text cursor indicator turned off",
    open_windows_settings: "Open Windows Settings",
    shape: "Shape:",
    shape_teardrop: "Teardrop (Windows style)",
    shape_circle: "Circle",
    shape_badge: "Text badge",
    position: "Position:",
    position_above: "Above text cursor",
    position_below: "Below text cursor",
    col_mode: "Mode",
    col_visible: "Show",
    col_color: "Color",
    col_size: "Size",
    col_badge_text: "Badge text",
    col_text_color: "Text color",
    text_colors: ["Auto", "White", "Black"],
    preview: |mode| format!("Preview: {mode}"),
    preview_sample: "Hello",
    trial: "Try it (switch input modes to test):",
    trial_hint: "Type here",
    presets: "Presets",
    preset_none: "(none)",
    preset_apply: "Apply",
    preset_delete: "Delete",
    preset_name_hint: "Preset name",
    preset_save: "Save current settings",
    preset_applied: |name| format!("Applied preset \"{name}\""),
    preset_deleted: |name| format!("Deleted preset \"{name}\""),
    preset_saved: |name| format!("Saved preset \"{name}\""),
    preset_name_required: "Enter a preset name",
    startup: "Start when I sign in",
    startup_added: "Added to startup",
    startup_failed: "Couldn't add to startup",
    startup_removed: "Removed from startup",
    startup_path: |path| format!("Startup entry: {path}"),
    startup_other_exe: "⚠ Startup points to an exe in another location (this copy won't start at sign-in)",
    startup_reregister: "Use this exe for startup",
    startup_updated: "Startup entry updated",
    language: "言語 / Language:",
    lang_auto: "Auto (match Windows)",
    updates: "Updates",
    update_check: "Check for updates automatically (at startup and once a day)",
    update_note: "When this is on, IsImeOn asks GitHub (api.github.com) for the latest release. It only lets you know; nothing is downloaded or installed automatically.",
    check_now: "Check now",
    checking: "Checking…",
    up_to_date: |v| format!("You're up to date (v{v})"),
    update_available: |tag| format!("Version {tag} is available"),
    open_release_page: "Open release page",
    update_failed: |e| format!("Couldn't check for updates: {e}"),
    update_winget_hint: "Installed with winget. To update, run `winget upgrade Hinaser.IsImeOn`.",
    update_menu: |tag| format!("&Update available: {tag}"),
    update_title: "IsImeOn update available",
    update_balloon: |tag, winget| {
        if winget {
            format!("{tag} is available. To update, run winget upgrade Hinaser.IsImeOn.")
        } else {
            format!("{tag} is available. Click to open the release page.")
        }
    },
    diagnostics: "Diagnostics",
    perf_log: "Record a performance log",
    perf_log_note: "Writes timestamped processing times, CPU time, and memory usage to perf.log once a minute. For operations that take longer than 50 ms, the name of the foreground app is also recorded. IsImeOn doesn't send the log anywhere, and it stays until you delete it.",
    open_log_folder: "Open log folder",
    uninstall: "Uninstall",
    uninstall_winget: "Installed with winget. To uninstall, run `winget uninstall Hinaser.IsImeOn`.",
    uninstall_button: "Remove everything and exit",
    uninstall_note: "Deletes the settings, the startup entry, and IsImeOn.exe",
    uninstall_confirm: "Remove IsImeOn? This can't be undone.",
    uninstall_delete: "Remove",
    cancel: "Cancel",
    uninstall_errors: |errors| format!("Some items could not be removed: {errors}"),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_setting_round_trips() {
        for s in LangSetting::ALL {
            assert_eq!(LangSetting::parse(s.to_config_str()), s);
        }
        assert_eq!(LangSetting::parse("unknown"), LangSetting::Auto);
    }

    // 現在言語はプロセス全体の状態なので、並列実行されるテストからは書き換えない
    #[test]
    fn explicit_settings_resolve_without_os() {
        assert_eq!(LangSetting::Ja.resolve(), Lang::Ja);
        assert_eq!(LangSetting::En.resolve(), Lang::En);
    }
}
