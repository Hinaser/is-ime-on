//! フォアグラウンドウィンドウのIME状態を IMM32 経由で読む。
//! デフォルトIMEウィンドウへ WM_IME_CONTROL(IMC_GETOPENSTATUS / IMC_GETCONVERSIONMODE)を送る方式。
//! ハング中のウィンドウで固まらないよう SendMessageTimeout を使う。

use crate::config::ImeMode;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SMTO_ABORTIFHUNG,
};

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_GETOPENSTATUS: usize = 0x0005;
const TIMEOUT_MS: u32 = 200;

// IME_CMODE_NATIVE(1) | IME_CMODE_KATAKANA(2) | IME_CMODE_FULLSHAPE(8)。
// IME_CMODE_ROMAN(16) はローマ字/かな入力の別なのでマスクして無視する。
const BASE_MODE_MASK: usize = 0x0B;

fn try_send(hwnd: HWND, command: usize) -> Option<usize> {
    let mut result: usize = 0;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_IME_CONTROL,
            WPARAM(command),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            TIMEOUT_MS,
            Some(&mut result),
        )
    };
    if ok.0 == 0 {
        None
    } else {
        Some(result)
    }
}

/// 指定ウィンドウ(通常はフォーカスコントロール)のIMEモードを返す。
pub fn read_mode(target: HWND) -> ImeMode {
    let ime_wnd = unsafe { ImmGetDefaultIMEWnd(target) };
    if ime_wnd.is_invalid() {
        return ImeMode::Off; // IMEコンテキストなし(コンソール等)
    }

    match try_send(ime_wnd, IMC_GETOPENSTATUS) {
        None | Some(0) => return ImeMode::Off,
        Some(_) => {}
    }

    let Some(conv) = try_send(ime_wnd, IMC_GETCONVERSIONMODE) else {
        return ImeMode::Other;
    };

    match conv & BASE_MODE_MASK {
        0x0 => ImeMode::HalfAlnum,
        0x3 => ImeMode::HalfKana,
        0x8 => ImeMode::WideAlnum,
        0x9 => ImeMode::Hiragana,
        0xB => ImeMode::WideKana,
        _ => ImeMode::Other,
    }
}
