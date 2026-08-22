//! Windows 側の状態(スタートアップ登録・競合ツール検出)。すべて HKCU のみ。

use std::path::{Path, PathBuf};
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

/// Run キーに登録されているコマンド文字列(未登録なら None)。
fn startup_command() -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_READ, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let mut size = 0u32;
        let rc = RegQueryValueExW(key, RUN_VALUE, None, None, None, Some(&mut size));
        if rc != ERROR_SUCCESS || size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let mut sz = size;
        let rc = RegQueryValueExW(
            key,
            RUN_VALUE,
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut sz),
        );
        let _ = RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return None;
        }
        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let text = String::from_utf16_lossy(&utf16);
        let text = text.trim_end_matches(char::from(0)).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}

/// 登録されている実行ファイルのパス(引用符を外したもの)。
pub fn registered_exe() -> Option<PathBuf> {
    let cmd = startup_command()?;
    let path = match cmd.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or("").to_string(),
        None => cmd.split_whitespace().next().unwrap_or("").to_string(),
    };
    if path.is_empty() { None } else { Some(PathBuf::from(path)) }
}

/// 同じ実行ファイルを指しているか(大文字小文字・短縮名の差を吸収)。
fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(x), Ok(y)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return x == y;
    }
    a.to_string_lossy()
        .eq_ignore_ascii_case(b.to_string_lossy().as_ref())
}

/// 登録済みで、かつ「今動いている実行ファイル」を指しているか。
pub fn is_startup_current() -> bool {
    match (registered_exe(), std::env::current_exe().ok()) {
        (Some(reg), Some(cur)) => same_file(&reg, &cur),
        _ => false,
    }
}

/// exe を移動された場合、Run キーを現在のパスへ貼り直す。
/// Windows は登録時の絶対パスをそのまま起動するため、これをしないと
/// 移動・改名・アップグレード後に自動起動が黙って効かなくなる。
/// 未登録なら何もしない(勝手に登録はしない)。
pub fn refresh_startup_if_moved() {
    if is_startup_registered() && !is_startup_current() {
        let _ = set_startup();
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

/// winget の portable パッケージとして導入されているか。
/// その場合 exe を自前で消すと winget のパッケージ情報と食い違うため、
/// 削除は `winget uninstall` に任せる。
pub fn is_winget_managed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    exe.to_string_lossy()
        .to_lowercase()
        .contains(r"\microsoft\winget\packages\")
}

/// 自動起動の登録と設定ファイルを削除する。戻り値は失敗した項目の説明。
pub fn remove_traces() -> Vec<String> {
    let mut errors = Vec::new();
    remove_startup();
    let dir = crate::config::AppConfig::dir_path();
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            errors.push(format!("{}: {e}", dir.display()));
        }
    }
    errors
}

/// 自分自身の exe を、プロセス終了後に削除するよう仕込む。
/// 実行中の exe は自分で削除できない(イメージがロックされる)ため、
/// 切り離した cmd に少し待たせてから削除させる。
pub fn schedule_self_delete() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new("cmd")
        .raw_arg(format!(
            "/C ping 127.0.0.1 -n 4 >nul & del /f /q \"{}\"",
            exe.display()
        ))
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .is_ok()
}
