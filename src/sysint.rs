//! Windows 側の状態(スタートアップ登録・競合ツール検出)。すべて HKCU のみ。

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SZ,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("IsImeOn");

/// Windows標準のテキストカーソルインジケーター(EoAExperiences.exe)が動作中か。
/// IsImeOn は自前描画なので不要であり、動いていると二重表示になるため警告に使う。
pub fn is_os_indicator_active() -> bool {
    !find_processes("EoAExperiences.exe").is_empty()
}

pub fn stop_os_indicator() {
    for pid in find_processes("EoAExperiences.exe") {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 0);
                let _ = CloseHandle(h);
            }
        }
    }
}

fn find_processes(name: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return pids;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let exe = String::from_utf16_lossy(
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)],
                );
                if exe.eq_ignore_ascii_case(name) {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    pids
}

pub fn open_cursor_settings() {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:easeofaccess-cursor"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

pub fn is_startup_registered() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_READ, &mut key) != ERROR_SUCCESS {
            return false;
        }
        let mut size = 0u32;
        let found = RegQueryValueExW(key, RUN_VALUE, None, None, None, Some(&mut size))
            == ERROR_SUCCESS
            && size > 0;
        let _ = RegCloseKey(key);
        found
    }
}

pub fn set_startup() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // 起動時は常にトレイ常駐なので、引数は不要。
    let value = format!("\"{}\"", exe.display());
    let utf16: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let bytes =
            std::slice::from_raw_parts(utf16.as_ptr() as *const u8, utf16.len() * 2);
        let ok = RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes)) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        ok
    }
}

pub fn remove_startup() {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_SET_VALUE, &mut key)
            == ERROR_SUCCESS
        {
            let _ = RegDeleteValueW(key, RUN_VALUE);
            let _ = RegCloseKey(key);
        }
    }
}
