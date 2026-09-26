//! egui 設定ウィンドウ。**別プロセス**(自 exe を `--settings-window` 付きで起動)として動かす。
//! GUIフレームワーク+GPUドライバ分のメモリ(100MB超)はウィンドウを閉じると
//! プロセスごと完全に解放され、常駐コアは数MBのまま保たれる。
//! 編集のたびに config.json へ保存し、常駐プロセスへ WM_APP_RELOAD を送って即時反映する
//! (保存ボタンはない)。

use crate::config::{
    parse_rgb_hex, to_rgb_hex, AppConfig, ImeMode, IndicatorPosition, IndicatorShape, Preset,
};
use crate::i18n::{self, tr, LangSetting};
use crate::shape::{self, Metrics};
use crate::sysint;
use eframe::egui::{self, Color32, ComboBox, FontId, RichText, Slider, Stroke};
use std::time::{Duration, Instant};
use windows::core::w;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateCaret, DestroyCaret, FindWindowW, PostMessageW, SetCaretPos, SetForegroundWindow, WM_APP,
};

/// 設定が保存されたとき常駐プロセスのメインウィンドウへ届くメッセージ(再読込を促す)。
pub const WM_APP_RELOAD: u32 = WM_APP + 3;
/// アンインストール時に常駐プロセスへ終了を要求するメッセージ。
pub const WM_APP_QUIT: u32 = WM_APP + 4;

const WINDOW_HEIGHT: f32 = 640.0;

/// 設定ウィンドウを開く(設定プロセスを起動する)。既に開いていれば子プロセス側が前面化して終了する。
pub fn open() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--settings-window").spawn();
    }
}

/// `--settings-window` で起動されたプロセスの本体。ウィンドウを閉じたら戻る(=プロセス終了)。
pub fn run_settings_process() {
    unsafe {
        // 設定ウィンドウの二重起動防止(既存があれば前面化のみ)
        let _mutex = CreateMutexW(None, true, w!("Local\\IsImeOn_Settings"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // 既存ウィンドウは別の言語で開かれているかもしれないので両方の題名で探す
            for title in [i18n::JA.window_title, i18n::EN.window_title] {
                let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
                if let Ok(existing) = FindWindowW(None, windows::core::PCWSTR(title.as_ptr())) {
                    let _ = SetForegroundWindow(existing);
                    break;
                }
            }
            return;
        }
    }
    run_window();
}

/// 保存を常駐プロセスへ通知する。
fn notify_main() {
    post_to_main(WM_APP_RELOAD);
}

fn post_to_main(msg: u32) {
    unsafe {
        if let Ok(main) = FindWindowW(w!("IsImeOnMain"), None) {
            let _ = PostMessageW(Some(main), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn run_window() {
    // DX12 を明示する(Win10+ なら常に使える)。既定の backends は PRIMARY|GL だが、
    // コンパイルされているのが Vulkan/GL だけだと、Vulkan ドライバが貧弱な安価な
    // ラップトップで GL に落ち、デスクトップ全体のちらつき(1fps提示に引きずられる)が再発する。
    let wgpu_options = eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(eframe::egui_wgpu::WgpuSetupCreateNew {
            instance_descriptor: wgpu::InstanceDescriptor {
                backends: wgpu::Backends::DX12 | wgpu::Backends::GL,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let options = eframe::NativeOptions {
        wgpu_options,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([tr().window_width, WINDOW_HEIGHT])
            .with_title(tr().window_title)
            .with_icon(std::sync::Arc::new(egui::IconData {
                rgba: include_bytes!("../assets/icon/icon-64.rgba").to_vec(),
                width: 64,
                height: 64,
            })),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "IsImeOn",
        options,
        Box::new(move |cc| {
            install_jp_fonts(&cc.egui_ctx);
            Ok(Box::new(SettingsApp::new(window_hwnd(cc))))
        }),
    );
}

/// eframe が作ったウィンドウの HWND(システムキャレットの作成先)。
fn window_hwnd(cc: &eframe::CreationContext<'_>) -> Option<HWND> {
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut std::ffi::c_void)),
        _ => None,
    }
}

/// egui 標準フォントは CJK を含まないため、システムの日本語フォントを追加する。
fn install_jp_fonts(ctx: &egui::Context) {
    // (ファイル, フェイス番号)。いずれも「UI」向けフェイスを選ぶこと。
    // 本文用フェイス(index 0)は hhea の lineGap が 1024/2048em = 0.5em もあり、
    // egui はそれを行の高さに算入するため、ベースラインが行の中央あたりに落ちて
    // ウィジェット下側に大きな余白ができてしまう。UIフェイスは lineGap = 0。
    let candidates: [(&str, u32); 4] = [
        (r"C:\Windows\Fonts\YuGothM.ttc", 1),  // Yu Gothic UI
        (r"C:\Windows\Fonts\YuGothR.ttc", 1),  // Yu Gothic UI Semilight
        (r"C:\Windows\Fonts\meiryo.ttc", 2),   // Meiryo UI
        (r"C:\Windows\Fonts\msgothic.ttc", 1), // MS UI Gothic
    ];
    let Some((bytes, index)) = candidates
        .iter()
        .find_map(|(path, i)| std::fs::read(path).ok().map(|b| (b, *i)))
    else {
        return;
    };
    let mut font = egui::FontData::from_owned(bytes);
    font.index = index;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("jp".to_owned(), std::sync::Arc::new(font));
    // 既定フォント(欧文)の「後ろ」に足すと、1行の中で欧文と和文が別フォントになり、
    // フォントごとのメトリクス差でベースラインがずれる(例:「IMEオフ」)。
    // 和文フォントは欧文字形も持っているので、先頭に入れて全文字を同一フォントで描く。
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "jp".to_owned());
    // 等幅は欧文の等幅性を保ちたいので、和文は fallback のままにする
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("jp".to_owned());
    ctx.set_fonts(fonts);
}

struct SettingsApp {
    config: AppConfig,
    /// 一覧で選択中(プレビュー対象)のモード。
    selected: ImeMode,
    trial: String,
    preset_name: String,
    selected_preset: usize,
    status: String,
    startup: bool,
    /// Run キーに登録されている exe のパス(未登録なら None)
    startup_path: Option<String>,
    /// 登録先が「今動いている exe」と一致しているか
    startup_is_current: bool,
    /// 「完全に削除して終了」の確認待ち
    confirm_uninstall: bool,
    /// 「試し打ち」用にシステムキャレットを作るウィンドウ
    hwnd: Option<HWND>,
    caret_active: bool,
    /// 現在のシステムキャレットの高さ(px)。DPI変化の検出用
    caret_height: i32,
    os_indicator: bool,
    last_sys_check: Instant,
}

impl SettingsApp {
    fn new(hwnd: Option<HWND>) -> SettingsApp {
        SettingsApp {
            hwnd,
            caret_active: false,
            caret_height: 0,
            config: AppConfig::load(),
            selected: ImeMode::Hiragana,
            trial: String::new(),
            preset_name: String::new(),
            selected_preset: 0,
            status: String::new(),
            startup: sysint::is_startup_registered(),
            startup_path: sysint::registered_exe().map(|p| p.display().to_string()),
            startup_is_current: sysint::is_startup_current(),
            confirm_uninstall: false,
            os_indicator: sysint::is_os_indicator_active(),
            last_sys_check: Instant::now(),
        }
    }

    fn save_and_apply(&mut self, message: &str) {
        self.config.save();
        notify_main();
        self.status = message.into();
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 2秒ごとにシステム状態(自動起動の登録・OS標準インジケーターの起動)を再確認
        if self.last_sys_check.elapsed() > Duration::from_secs(2) {
            self.startup = sysint::is_startup_registered();
            self.startup_path = sysint::registered_exe().map(|p| p.display().to_string());
            self.startup_is_current = sysint::is_startup_current();
            self.os_indicator = sysint::is_os_indicator_active();
            self.last_sys_check = Instant::now();
        }
        // 可変リフレッシュレート(G-SYNC等)の環境では、フォーカス中ウィンドウの
        // 提示レートにディスプレイのリフレッシュが追従する。キャレット点滅などで
        // 1〜2fps だけ提示すると、リフレッシュが最低値まで落ちてパネル全体が
        // ちらついて見える(VRRの低レート輝度フリッカ)。レートが一定なら
        // ちらつかないため、開いている間は 60fps で提示し続ける。
        // DX12 の Present は正しくブロックするので CPU はほぼ消費しない
        // (GL は SwapBuffers がビジーウェイトで1コア使い切るため不可)。
        // 常駐コアとは別プロセスなので、閉じればこのコストは完全に消える。
        // VRRが追従するのはフォーカス中のウィンドウだけなので、
        // フォーカスを失っている間は低頻度に戻して無駄な描画をやめる。
        if ctx.input(|i| i.focused) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        let before = self.config.clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.ui_os_indicator_warning(ui);
                self.ui_shape(ui);
                ui.separator();
                self.ui_mode_table(ui);
                ui.separator();
                self.ui_preview(ui, ctx);
                ui.separator();
                self.ui_presets(ui);
                ui.separator();
                self.ui_language(ui, ctx);
                self.ui_startup(ui);
                self.ui_diagnostics(ui);
                self.ui_uninstall(ui);
                if !self.status.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(&self.status).weak());
                }
            });
        });

        if self.config != before {
            self.save_and_apply(tr().saved);
        }
    }
}

impl SettingsApp {
    fn ui_os_indicator_warning(&mut self, ui: &mut egui::Ui) {
        if !self.os_indicator {
            return;
        }
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(0x5A, 0x46, 0x00))
            .show(ui, |ui| {
                ui.label(tr().os_indicator_warning);
                ui.horizontal(|ui| {
                    if ui.button(tr().os_indicator_stop).clicked() {
                        sysint::stop_os_indicator();
                        self.os_indicator = sysint::is_os_indicator_active();
                        self.status = tr().os_indicator_stopped.into();
                    }
                    if ui.button(tr().open_windows_settings).clicked() {
                        sysint::open_cursor_settings();
                    }
                });
            });
        ui.add_space(6.0);
    }

    fn ui_shape(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(tr().shape);
            let mut shape = self.config.shape_enum();
            ComboBox::from_id_salt("shape")
                .selected_text(shape_label(shape))
                .show_ui(ui, |ui| {
                    for s in [
                        IndicatorShape::Teardrop,
                        IndicatorShape::TopCircle,
                        IndicatorShape::Badge,
                    ] {
                        ui.selectable_value(&mut shape, s, shape_label(s));
                    }
                });
            if shape != self.config.shape_enum() {
                self.config.shape = shape.to_config_str().into();
            }

            ui.add_space(12.0);
            ui.label(tr().position);
            let mut position = self.config.position_enum();
            ui.selectable_value(&mut position, IndicatorPosition::Above, tr().position_above);
            ui.selectable_value(&mut position, IndicatorPosition::Below, tr().position_below);
            if position != self.config.position_enum() {
                self.config.position = position.to_config_str().into();
            }
        });
    }

    fn ui_mode_table(&mut self, ui: &mut egui::Ui) {
        let is_badge = self.config.shape_enum() == IndicatorShape::Badge;
        egui::Grid::new("modes")
            .striped(true)
            .min_col_width(40.0)
            .show(ui, |ui| {
                let t = tr();
                ui.label(RichText::new(t.col_mode).strong());
                ui.label(RichText::new(t.col_visible).strong());
                ui.label(RichText::new(t.col_color).strong());
                ui.label(RichText::new(t.col_size).strong());
                if is_badge {
                    ui.label(RichText::new(t.col_badge_text).strong());
                    ui.label(RichText::new(t.col_text_color).strong());
                }
                ui.end_row();

                for (i, mode) in ImeMode::ALL.iter().enumerate() {
                    let mode = *mode;
                    let key = mode.key().to_string();
                    let setting = self.config.modes.get_mut(&key).unwrap();

                    if ui
                        .selectable_label(self.selected == mode, mode.display_name())
                        .clicked()
                    {
                        self.selected = mode;
                    }
                    ui.checkbox(&mut setting.visible, "");

                    let mut rgb = parse_rgb_hex(&setting.color)
                        .map(|(r, g, b)| [r, g, b])
                        .unwrap_or([0, 0, 0]);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        setting.color = to_rgb_hex(rgb[0], rgb[1], rgb[2]);
                        self.selected = mode;
                    }

                    let mut size = setting.size.clamp(1, 5);
                    if ui.add(Slider::new(&mut size, 1..=5)).changed() {
                        setting.size = size;
                        self.selected = mode;
                    }

                    if is_badge {
                        let mut label = setting.label.clone();
                        if ui
                            .add(egui::TextEdit::singleline(&mut label).desired_width(48.0))
                            .changed()
                        {
                            setting.label = label.trim().chars().take(2).collect();
                            self.selected = mode;
                        }

                        // "" = 自動(config.json 直編集による任意色も「自動」表示に含める)
                        let old_idx = match setting.label_color.as_str() {
                            "#FFFFFF" => 1,
                            "#000000" => 2,
                            _ => 0,
                        };
                        let mut idx = old_idx;
                        let names = tr().text_colors;
                        ComboBox::from_id_salt(format!("lc{i}"))
                            .selected_text(names[idx])
                            .width(56.0)
                            .show_ui(ui, |ui| {
                                for (j, name) in names.iter().enumerate() {
                                    ui.selectable_value(&mut idx, j, *name);
                                }
                            });
                        if idx != old_idx {
                            setting.label_color = ["", "#FFFFFF", "#000000"][idx].into();
                            self.selected = mode;
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn ui_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let shape = self.config.shape_enum();
        let position = self.config.position_enum();
        let mode = self.selected;
        let setting = self.config.for_mode(mode);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label((tr().preview)(mode.display_name()));
                draw_preview(ui, &setting, shape, position);
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(tr().trial);
                let out = egui::TextEdit::singleline(&mut self.trial)
                    .desired_width(220.0)
                    .hint_text(tr().trial_hint)
                    .show(ui);
                // IME の変換確定 Enter も egui には Enter として届き、singleline は
                // フォーカスを手放してしまう。試し打ち欄は連続入力する場所なので、
                // Enter でフォーカスが外れたら握り直す
                if out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    out.response.request_focus();
                }
                self.sync_system_caret(ctx, &out);
            });
        });
    }

    /// egui は自前でキャレットを描くだけで、OS にはキャレットを公開しない。
    /// そのままでは本ソフト自身のインジケーターが「試し打ち」欄に出ないため、
    /// フォーカス中だけ同じ位置にシステムキャレットを作って位置を知らせる。
    /// ShowCaret は呼ばない(呼ぶと egui のキャレットと二重に描画されるため)。
    fn sync_system_caret(&mut self, ctx: &egui::Context, out: &egui::text_edit::TextEditOutput) {
        let Some(hwnd) = self.hwnd else { return };
        let cursor = if out.response.has_focus() {
            out.cursor_range.map(|r| r.primary)
        } else {
            None
        };
        let Some(cursor) = cursor else {
            self.destroy_system_caret();
            return;
        };

        let rect = out
            .galley
            .pos_from_cursor(&cursor)
            .translate(out.galley_pos.to_vec2());
        // ScrollArea で欄が見えない位置へスクロールされてもフォーカスは残る。
        // そのまま公開すると無関係な場所にインジケーターが出るため、見えている間だけ公開する。
        if !out.text_clip_rect.intersects(rect) {
            self.destroy_system_caret();
            return;
        }
        let ppp = ctx.pixels_per_point();
        let x = (rect.left() * ppp).round() as i32;
        let y = (rect.top() * ppp).round() as i32;
        let h = (rect.height().max(1.0) * ppp).round() as i32;
        unsafe {
            // SetCaretPos はサイズを変えられないので、DPI変化などで高さが変わったら作り直す
            if self.caret_active && self.caret_height != h {
                let _ = DestroyCaret();
                self.caret_active = false;
            }
            if !self.caret_active {
                self.caret_active = CreateCaret(hwnd, None, 1, h).is_ok();
                self.caret_height = h;
            }
            if self.caret_active {
                let _ = SetCaretPos(x, y);
            }
        }
    }

    fn destroy_system_caret(&mut self) {
        if self.caret_active {
            // 失敗しても active のままにしない: キャレットはフォーカス移動で
            // OS 側でも破棄されるため、こちらの所有権記録だけ確実に手放す
            unsafe {
                let _ = DestroyCaret();
            }
            self.caret_active = false;
        }
    }

    fn ui_presets(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(tr().presets).strong());
        ui.horizontal(|ui| {
            let names: Vec<String> = self.config.presets.iter().map(|p| p.name.clone()).collect();
            let current = names
                .get(self.selected_preset)
                .cloned()
                .unwrap_or_else(|| tr().preset_none.into());
            ComboBox::from_id_salt("preset")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_preset, i, name);
                    }
                });
            let has = self.selected_preset < self.config.presets.len();
            if ui.add_enabled(has, egui::Button::new(tr().preset_apply)).clicked() {
                let preset = self.config.presets[self.selected_preset].clone();
                apply_preset(&mut self.config, &preset);
                self.status = (tr().preset_applied)(&preset.name);
            }
            if ui.add_enabled(has, egui::Button::new(tr().preset_delete)).clicked() {
                let removed = self.config.presets.remove(self.selected_preset);
                self.selected_preset = 0;
                self.status = (tr().preset_deleted)(&removed.name);
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .desired_width(160.0)
                    .hint_text(tr().preset_name_hint),
            );
            if ui.button(tr().preset_save).clicked() {
                let name = self.preset_name.trim().to_string();
                if name.is_empty() {
                    self.status = tr().preset_name_required.into();
                } else {
                    let preset = Preset {
                        name: name.clone(),
                        values: self.config.modes.clone(),
                    };
                    self.config.presets.retain(|p| p.name != name);
                    self.config.presets.push(preset);
                    self.selected_preset = self.config.presets.len() - 1;
                    self.status = (tr().preset_saved)(&name);
                }
            }
        });
    }

    fn ui_language(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            // どの言語で表示中でも見つけられるよう、見出しは日英併記
            ui.label(tr().language);
            let mut lang = self.config.language_enum();
            ComboBox::from_id_salt("language")
                .selected_text(lang.label())
                .show_ui(ui, |ui| {
                    for l in LangSetting::ALL {
                        ui.selectable_value(&mut lang, l, l.label());
                    }
                });
            if lang != self.config.language_enum() {
                self.config.language = lang.to_config_str().into();
                self.config.apply_language();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(tr().window_title.into()));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    tr().window_width,
                    WINDOW_HEIGHT,
                )));
            }
        });
    }

    fn ui_startup(&mut self, ui: &mut egui::Ui) {
        let mut startup = self.startup;
        if ui
            .checkbox(&mut startup, tr().startup)
            .changed()
        {
            if startup {
                if sysint::set_startup() {
                    self.status = tr().startup_added.into();
                } else {
                    self.status = tr().startup_failed.into();
                }
            } else {
                sysint::remove_startup();
                self.status = tr().startup_removed.into();
            }
            self.refresh_startup_state();
        }

        // Windows は登録時の絶対パスをそのまま起動するので、どこを指しているかを見せる
        if let Some(path) = self.startup_path.clone() {
            ui.label(RichText::new((tr().startup_path)(&path)).weak().small());
            if !self.startup_is_current {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        Color32::from_rgb(0xE0, 0xA0, 0x30),
                        tr().startup_other_exe,
                    );
                    if ui.button(tr().startup_reregister).clicked() {
                        sysint::set_startup();
                        self.refresh_startup_state();
                        self.status = tr().startup_updated.into();
                    }
                });
            }
        }
    }

    fn ui_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new(tr().diagnostics).strong());
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.perf_log, tr().perf_log);
            if ui.button(tr().open_log_folder).clicked() {
                let _ = std::fs::create_dir_all(AppConfig::dir_path());
                sysint::open_folder(&AppConfig::dir_path());
            }
        });
        ui.label(RichText::new(tr().perf_log_note).weak().small());
    }

    fn ui_uninstall(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new(tr().uninstall).strong());

        if sysint::is_winget_managed() {
            ui.label(RichText::new(tr().uninstall_winget).weak());
            return;
        }

        if !self.confirm_uninstall {
            if ui.button(tr().uninstall_button).clicked() {
                self.confirm_uninstall = true;
            }
            ui.label(
                RichText::new(tr().uninstall_note)
                    .weak()
                    .small(),
            );
            return;
        }

        ui.colored_label(
            Color32::from_rgb(0xE0, 0xA0, 0x30),
            tr().uninstall_confirm,
        );
        ui.horizontal(|ui| {
            if ui.button(tr().uninstall_delete).clicked() {
                self.run_uninstall(ui.ctx());
            }
            if ui.button(tr().cancel).clicked() {
                self.confirm_uninstall = false;
            }
        });
    }

    fn run_uninstall(&mut self, ctx: &egui::Context) {
        let errors = sysint::remove_traces();
        if !errors.is_empty() {
            // 消せないものがあるときは exe を消さずに知らせる(消し残しを隠さない)
            self.confirm_uninstall = false;
            self.refresh_startup_state();
            self.status = (tr().uninstall_errors)(&errors.join(" / "));
            return;
        }
        post_to_main(WM_APP_QUIT); // 常駐プロセスを終了させる(exe のロックを外す)
        sysint::schedule_self_delete();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn refresh_startup_state(&mut self) {
        self.startup = sysint::is_startup_registered();
        self.startup_path = sysint::registered_exe().map(|p| p.display().to_string());
        self.startup_is_current = sysint::is_startup_current();
    }
}

fn apply_preset(config: &mut AppConfig, preset: &Preset) {
    for mode in ImeMode::ALL {
        let Some(entry) = preset.values.get(mode.key()) else {
            continue;
        };
        let setting = config.modes.get_mut(mode.key()).unwrap();
        if parse_rgb_hex(&entry.color).is_some() {
            setting.color = entry.color.clone();
        }
        setting.size = entry.size;
        setting.visible = entry.visible;
        if !entry.label.is_empty() {
            setting.label = entry.label.clone();
        }
        setting.label_color = entry.label_color.clone();
    }
}

fn shape_label(shape: IndicatorShape) -> &'static str {
    match shape {
        IndicatorShape::Teardrop => tr().shape_teardrop,
        IndicatorShape::TopCircle => tr().shape_circle,
        IndicatorShape::Badge => tr().shape_badge,
    }
}

/// 設定画面のプレビュー: テキスト欄風の背景+サンプル文字+キャレット+インジケーター。
fn draw_preview(
    ui: &mut egui::Ui,
    setting: &crate::config::ModeSetting,
    shape: IndicatorShape,
    position: IndicatorPosition,
) {
    let (resp, p) = ui.allocate_painter(egui::vec2(220.0, 100.0), egui::Sense::hover());
    let rect = resp.rect;

    p.rect_filled(rect, 4.0, Color32::WHITE);
    p.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0_f32, Color32::from_rgb(0xC8, 0xC8, 0xC8)),
        egui::StrokeKind::Inside,
    );

    const FONT_SIZE: f32 = 18.0;
    let text_pos = egui::pos2(rect.left() + 14.0, rect.center().y);
    let text_rect = p.text(
        text_pos,
        egui::Align2::LEFT_CENTER,
        tr().preview_sample,
        FontId::proportional(FONT_SIZE),
        Color32::BLACK,
    );

    // p.text() が返すのはフォントの ascent/descent を含む galley 矩形で、
    // 実際の字面よりかなり上下に広い。そのまま使うとインジケーターが浮いて見えるため、
    // 実キャレット(字面の高さ + わずかな余白)に近い矩形を字面中央から作る。
    let caret_x = text_rect.right() + 3.0;
    let caret_h = FONT_SIZE * 1.15;
    let center_y = text_rect.center().y;
    let caret_top = center_y - caret_h / 2.0;
    let caret_bottom = center_y + caret_h / 2.0;
    p.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(caret_x, caret_top),
            egui::vec2(1.6, caret_bottom - caret_top),
        ),
        0.0,
        Color32::BLACK,
    );

    if !setting.visible {
        return;
    }

    let rgb = parse_rgb_hex(&setting.color).unwrap_or((0, 0, 0));
    let color = Color32::from_rgb(rgb.0, rgb.1, rgb.2);
    let metrics = Metrics::preview(setting.size);
    let r = metrics.blob_r;
    let cx = caret_x + 0.8;
    let span = shape::CaretSpan { top: caret_top, bottom: caret_bottom, position };
    let fill_circle = |c: shape::Circle| {
        let (x, y, r) = span.place_circle(c);
        p.circle_filled(egui::pos2(x, y), r, color);
    };

    match shape {
        IndicatorShape::Teardrop => {
            fill_circle(shape::top_circle(r, cx, caret_top));
            fill_circle(shape::lower_circle(r, cx, caret_bottom));
        }
        IndicatorShape::TopCircle => {
            fill_circle(shape::top_circle(r, cx, caret_top));
        }
        IndicatorShape::Badge => {
            let side = metrics.badge_side;
            let (left, top) = span.badge_origin(side, cx);
            let badge =
                egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(side, side));
            p.rect_filled(badge, side * shape::BADGE_CORNER_RATIO, color);
            let (tr, tg, tb) = parse_rgb_hex(&setting.label_color)
                .unwrap_or_else(|| shape::auto_text_color(rgb));
            let label = if setting.label.is_empty() { "?" } else { &setting.label };
            p.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(shape::badge_font_size(side, label.chars().count())),
                Color32::from_rgb(tr, tg, tb),
            );
        }
    }
}
