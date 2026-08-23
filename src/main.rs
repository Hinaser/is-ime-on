//! IsImeOn — IME連動キャレットインジケーター(ネイティブ版)。
//! メインスレッド: 隠しウィンドウ + トレイ + WinEventフック + オーバーレイ描画。
//! ポーラースレッド: IME状態・キャレット位置の読み取り(engine.rs)。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod caret;
mod config;
mod engine;
mod ime;
mod overlay;
mod settings;
mod shape;
mod sysint;
mod tray;
mod uia;

use config::{parse_rgb_hex, AppConfig, ImeMode};
use engine::{Engine, WM_APP_STATE};
use overlay::{DrawParams, Overlay};
use std::cell::RefCell;
use tray::{Tray, CMD_EXIT, CMD_OPEN, CMD_PAUSE, CMD_RELOAD, WM_APP_TRAY};
use windows::core::w;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, EVENT_OBJECT_FOCUS,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, MB_ICONINFORMATION, MB_OK, MSG,
    OBJID_CARET, WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_DESTROY,
    WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

struct App {
    config: AppConfig,
    overlay: Overlay,
    tray: Tray,
    engine: Engine,
    paused: bool,
    current_mode: ImeMode,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn visible_by_mode(config: &AppConfig) -> [bool; 7] {
    let mut v = [false; 7];
    for mode in ImeMode::ALL {
        v[mode.index()] = config.for_mode(mode).visible;
    }
    v
}

fn main() {
    // 設定ウィンドウ専用プロセスとして起動された場合(トレイの「設定を開く」から)
    if std::env::args().any(|a| a == "--settings-window") {
        settings::run_settings_process();
        return;
    }

    unsafe {
        // 二重起動防止(既に起動していれば知らせて終了する)
        let _mutex = CreateMutexW(None, true, w!("Local\\IsImeOn_SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            MessageBoxW(
                None,
                w!("IsImeOn は既に起動しています。タスクトレイのアイコンから設定を開けます。"),
                w!("IsImeOn"),
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }

        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let hinstance = GetModuleHandleW(None).expect("GetModuleHandle");
        let class_name = w!("IsImeOnMain");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("IsImeOn"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .expect("CreateWindowExW");

        // exe を移動された場合に備え、自動起動の登録先を現在のパスへ追随させる
        sysint::refresh_startup_if_moved();

        let config = AppConfig::load();
        let overlay = Overlay::new().expect("overlay");
        let tray = Tray::new(hwnd);
        let engine = Engine::start(
            hwnd,
            config.poll_interval_ms.max(0) as u32,
            visible_by_mode(&config),
        );
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                config,
                overlay,
                tray,
                engine,
                paused: false,
                current_mode: ImeMode::Off,
            });
        });
        with_app(update_tray); // 初期アイコン(IMEオフ色)

        // 省電力の要: フォアグラウンド切替・フォーカス移動・キャレット移動で即時 poke。
        // イベントが途絶えたらポーラーは自動でバックオフする。
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let hooks: Vec<HWINEVENTHOOK> = [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
        ]
        .iter()
        .map(|&(min, max)| SetWinEventHook(min, max, None, Some(win_event_proc), 0, 0, flags))
        .collect();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        for hook in hooks {
            let _ = UnhookWinEvent(hook);
        }
    }
}

/// APP への可変アクセス。
///
/// モーダルなAPI(TrackPopupMenu / MessageBox など)は入れ子のメッセージループを回すため、
/// 借用中に wndproc が再入しうる。二重借用でプロセスを落とさないよう try_borrow_mut を使い、
/// 借用中の再入は「今回の更新を捨てる」で済ませる(状態は次の Tick で追いつく)。
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().map(f),
        Err(_) => None,
    })
}

extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    idobject: i32,
    _idchild: i32,
    _thread: u32,
    _time: u32,
) {
    // LOCATIONCHANGE はカーソル移動などでも大量に来るため、キャレットのみ拾う
    if event == EVENT_OBJECT_LOCATIONCHANGE && idobject != OBJID_CARET.0 {
        return;
    }
    let _ = with_app(|app| app.engine.poke());
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_STATE => {
            with_app(|app| {
                let (mode, caret) = app.engine.take_latest();
                apply_state(app, mode, caret);
            });
            LRESULT(0)
        }
        // 設定プロセスからの one-shot 要求。WM_APP_STATE と違い「次のTickで追いつく」が
        // 効かないため、借用競合(モーダルループ中)で実行できなかった場合は再ポストして
        // 取りこぼさない。
        settings::WM_APP_RELOAD => {
            let done = with_app(|app| {
                app.config = AppConfig::load();
                app.engine.set_visible_by_mode(visible_by_mode(&app.config));
                update_tray(app);
            })
            .is_some();
            if !done {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), msg, wp, lp);
                }
            }
            LRESULT(0)
        }
        // 設定ウィンドウからのアンインストール要求。exe のロックを外すため終了する。
        // DestroyWindow は借用不要なので競合の心配はない。
        settings::WM_APP_QUIT => {
            handle_command(hwnd, CMD_EXIT);
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let event = (lp.0 as u32) & 0xFFFF;
            match event {
                WM_RBUTTONUP => {
                    // paused を読むだけの短い借用で済ませ、メニュー表示中は借用を持たない
                    let paused = with_app(|app| app.paused).unwrap_or(false);
                    let cmd = Tray::show_menu(hwnd, paused);
                    handle_command(hwnd, cmd);
                }
                WM_LBUTTONDBLCLK => handle_command(hwnd, CMD_OPEN),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // Drop: トレイ削除・エンジン停止・オーバーレイ破棄。
            // 万一借用中でもパニック(=abort)せず、後始末をスキップして終了を優先する
            APP.with(|cell| {
                if let Ok(mut slot) = cell.try_borrow_mut() {
                    slot.take();
                }
            });
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn apply_state(app: &mut App, mode: ImeMode, caret: Option<caret::CaretInfo>) {
    let changed = mode != app.current_mode;
    app.current_mode = mode;

    if !app.paused {
        let setting = app.config.for_mode(mode);
        match caret {
            Some(c) if setting.visible => {
                app.overlay.show(DrawParams {
                    caret: c,
                    color: parse_rgb_hex(&setting.color).unwrap_or((0, 0, 0)),
                    size: setting.size.clamp(1, 5),
                    shape: app.config.shape_enum(),
                    label: setting.label.clone(),
                    label_color: parse_rgb_hex(&setting.label_color),
                });
            }
            _ => app.overlay.hide(),
        }
    }

    if changed {
        update_tray(app);
    }
}

fn update_tray(app: &mut App) {
    let setting = app.config.for_mode(app.current_mode);
    // 一時停止中はグレーにして「動いていない」ことを見せる
    let color = if app.paused {
        (0x80, 0x80, 0x80)
    } else {
        parse_rgb_hex(&setting.color).unwrap_or((0, 0, 0))
    };
    let suffix = if app.paused { "(一時停止中)" } else { "" };
    let tip = format!("IsImeOn — {}{}", app.current_mode.display_name(), suffix);
    app.tray
        .update(color, &setting.label, parse_rgb_hex(&setting.label_color), &tip);
}

fn handle_command(hwnd: HWND, cmd: u32) {
    match cmd {
        CMD_OPEN => settings::open(),
        CMD_RELOAD => {
            with_app(|app| {
                app.config = AppConfig::load();
                app.engine.set_visible_by_mode(visible_by_mode(&app.config));
                update_tray(app);
            });
        }
        CMD_PAUSE => {
            with_app(|app| {
                app.paused = !app.paused;
                app.engine.set_paused(app.paused);
                if app.paused {
                    app.overlay.hide();
                }
                update_tray(app);
            });
        }
        CMD_EXIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}
