//! キャレット位置(スクリーン座標・物理px)の取得。
//! まずシステムキャレット(GetGUIThreadInfo)、取れないアプリでは UI Automation にフォールバック。

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CaretInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// フォアグラウンドウィンドウと、そのスレッドの GUITHREADINFO を取得する。
pub fn foreground_thread_info() -> (HWND, Option<GUITHREADINFO>) {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return (fg, None);
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        match GetGUIThreadInfo(tid, &mut info) {
            Ok(()) => (fg, Some(info)),
            Err(_) => (fg, None),
        }
    }
}

/// システムキャレット(GetGUIThreadInfo)由来のキャレット位置。取れなければ None。
fn system_caret() -> Option<CaretInfo> {
    let (_, info) = foreground_thread_info();
    let info = info?;
    if info.hwndCaret.is_invalid() {
        return None;
    }

    let mut tl = POINT { x: info.rcCaret.left, y: info.rcCaret.top };
    let mut br = POINT { x: info.rcCaret.right, y: info.rcCaret.bottom };
    unsafe {
        if !ClientToScreen(info.hwndCaret, &mut tl).as_bool()
            || !ClientToScreen(info.hwndCaret, &mut br).as_bool()
        {
            return None;
        }
    }

    let w = br.x - tl.x;
    let h = br.y - tl.y;
    if !(2..=400).contains(&h) {
        return None; // キャレットとして妥当な高さのみ
    }

    Some(CaretInfo { x: tl.x, y: tl.y, width: w.max(1), height: h })
}

/// システムキャレット → UIA の順で取得。どちらも不明なら None。
pub fn get_caret() -> Option<CaretInfo> {
    system_caret().or_else(crate::uia::get_caret)
}
