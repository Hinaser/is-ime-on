//! 更新の確認(オプトイン)。config の `UpdateCheck` が true のときだけ GitHub の Releases API へ
//! 問い合わせ、新しいバージョンがあれば知らせる。ダウンロードや置き換えはしない(通知のみ)。
//! 通信は WinHTTP(OS のプロキシ設定に従う)。開くリンクは固定の RELEASES_URL だけで、
//! API の応答に含まれる URL は使わない。

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// 新しいバージョンが見つかったときにメインウィンドウへ届くメッセージ。
pub const WM_APP_UPDATE: u32 = WM_APP + 5;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 通知から開くページ。
pub const RELEASES_URL: &str = "https://github.com/Hinaser/is-ime-on/releases/latest";

const API_HOST: PCWSTR = w!("api.github.com");
const API_PATH: PCWSTR = w!("/repos/Hinaser/is-ime-on/releases/latest");
const HTTPS_PORT: u16 = 443;
const TIMEOUT_MS: i32 = 15_000;
/// 応答の上限(latest リリースの JSON は数KB)。
const MAX_BODY: usize = 1024 * 1024;

/// 起動直後(ログオン直後はネットワークが未接続のことがある)の待ち。
const FIRST_DELAY: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// "v1.2.3" / "1.2.3" → (1, 2, 3)。プレリリース接尾辞("-beta" など)は無視する。
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// latest リリース JSON の tag_name。
fn parse_tag(body: &[u8]) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    serde_json::from_slice::<Release>(body)
        .map(|r| r.tag_name)
        .map_err(|e| format!("invalid response: {e}"))
}

/// 新しければ Ok(Some(タグ))、最新なら Ok(None)、失敗なら Err(理由)。
pub type CheckResult = Result<Option<String>, String>;

/// 最新リリースを問い合わせる。
pub fn check() -> CheckResult {
    let tag = parse_tag(&fetch_latest_release()?)?;
    if parse_version(&tag).is_none() {
        return Err(format!("unexpected tag: {tag}"));
    }
    Ok(is_newer(&tag, CURRENT_VERSION).then_some(tag))
}

/// WinHTTP のハンドルを確実に閉じる。
struct Handle(*mut std::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn open(h: *mut std::ffi::c_void, what: &str) -> Result<Handle, String> {
    if h.is_null() {
        Err(describe(what, windows::core::Error::from_win32()))
    } else {
        Ok(Handle(h))
    }
}

/// WinHTTP のエラーコード(12xxx)はシステムのメッセージ表にないため、よくあるものは自前で説明する。
fn describe(what: &str, e: windows::core::Error) -> String {
    let code = (e.code().0 as u32) & 0xFFFF;
    let reason = match code {
        12002 => "timed out".to_string(),
        12007 => "could not resolve api.github.com".to_string(),
        12029 | 12030 => "could not connect".to_string(),
        12175 => "secure connection failed".to_string(),
        _ => {
            let m = e.message();
            if m.is_empty() { "unknown error".to_string() } else { m }
        }
    };
    format!("{reason} ({what}, {code})")
}

fn fetch_latest_release() -> Result<Vec<u8>, String> {
    let err = describe;
    let agent: Vec<u16> = format!("IsImeOn/{CURRENT_VERSION}").encode_utf16().chain([0]).collect();
    let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n".encode_utf16().collect();
    unsafe {
        let session = open(
            WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
            .map_err(|e| err("WinHttpSetTimeouts", e))?;
        let connect = open(WinHttpConnect(session.0, API_HOST, HTTPS_PORT, 0), "WinHttpConnect")?;
        let request = open(
            WinHttpOpenRequest(
                connect.0,
                w!("GET"),
                API_PATH,
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            "WinHttpOpenRequest",
        )?;
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)
            .map_err(|e| err("WinHttpSendRequest", e))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| err("WinHttpReceiveResponse", e))?;

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut std::ffi::c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| err("WinHttpQueryHeaders", e))?;
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }

        let mut body = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
                &mut read,
            )
            .map_err(|e| err("WinHttpReadData", e))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > MAX_BODY {
                return Err("response too large".into());
            }
        }
        Ok(body)
    }
}

struct Inner {
    enabled: bool,
    quit: bool,
    /// 設定が変わった(有効化された)ので待ちを打ち切る。
    changed: bool,
}

/// 常駐プロセス用の定期確認スレッド。新しいバージョンを見つけたら WM_APP_UPDATE を送る。
pub struct Updater {
    state: Arc<(Mutex<Inner>, Condvar)>,
    /// 見つかった新しいバージョンのタグ。
    available: Arc<Mutex<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Updater {
    pub fn start(main_hwnd: HWND, enabled: bool) -> Updater {
        let state = Arc::new((
            Mutex::new(Inner { enabled, quit: false, changed: false }),
            Condvar::new(),
        ));
        let available = Arc::new(Mutex::new(None));
        let hwnd_raw = main_hwnd.0 as usize; // HWND は Send でないため生値で渡す
        let (state2, available2) = (Arc::clone(&state), Arc::clone(&available));
        let thread = std::thread::Builder::new()
            .name("update".into())
            .spawn(move || updater_loop(state2, available2, hwnd_raw))
            .expect("spawn update");
        Updater { state, available, thread: Some(thread) }
    }

    /// 無効化時の available の消去は状態ロックを持ったまま行う(確認スレッドの書き込みと直列化)。
    pub fn set_enabled(&self, enabled: bool) {
        let (lock, cvar) = &*self.state;
        let mut inner = lock.lock().unwrap();
        if inner.enabled != enabled {
            inner.enabled = enabled;
            inner.changed = true;
            cvar.notify_one();
        }
        if !enabled {
            *self.available.lock().unwrap() = None;
        }
    }

    pub fn available(&self) -> Option<String> {
        self.available.lock().unwrap().clone()
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        {
            let (lock, cvar) = &*self.state;
            lock.lock().unwrap().quit = true;
            cvar.notify_one();
        }
        // 通信中ならタイムアウト(最大 TIMEOUT_MS 程度)まで待たされるため join しない。
        // プロセス終了とともに消える
        self.thread.take();
    }
}

fn updater_loop(
    state: Arc<(Mutex<Inner>, Condvar)>,
    available: Arc<Mutex<Option<String>>>,
    hwnd_raw: usize,
) {
    let (lock, cvar) = &*state;
    let mut wait = FIRST_DELAY;
    loop {
        {
            let mut inner = lock.lock().unwrap();
            // 無効の間は有効化されるまで眠る。有効化されたら待ち時間の残りは捨てて確認する
            loop {
                if inner.quit {
                    return;
                }
                if inner.changed {
                    inner.changed = false;
                    if inner.enabled {
                        break;
                    }
                }
                if !inner.enabled {
                    inner = cvar.wait(inner).unwrap();
                    continue;
                }
                let (guard, timeout) = cvar.wait_timeout(inner, wait).unwrap();
                inner = guard;
                if timeout.timed_out() && inner.enabled && !inner.quit {
                    break;
                }
            }
        }

        let result = check();
        // 通信中に無効化(または再設定)されていたら結果を捨てる。available の更新は状態ロック下で
        // 行い、set_enabled(false) による消去と前後しないようにする
        let inner = lock.lock().unwrap();
        if !inner.enabled || inner.changed || inner.quit {
            continue;
        }
        wait = match result {
            Ok(found) => {
                let newer = found.is_some();
                *available.lock().unwrap() = found;
                if newer {
                    unsafe {
                        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
                        let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE, WPARAM(0), LPARAM(0));
                    }
                }
                CHECK_EVERY
            }
            Err(_) => RETRY_AFTER,
        };
        drop(inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_accepts_tags() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn is_newer_compares_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.9", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    /// 実際に GitHub へ問い合わせる。`cargo test live_check -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_check() {
        let tag = parse_tag(&fetch_latest_release().unwrap()).unwrap();
        println!("latest={tag} current={CURRENT_VERSION} check={:?}", check());
        assert!(parse_version(&tag).is_some());
    }

    #[test]
    fn parse_tag_reads_tag_name() {
        let body = br#"{"tag_name":"v0.3.0","html_url":"https://example.com","draft":false}"#;
        assert_eq!(parse_tag(body).unwrap(), "v0.3.0");
        assert!(parse_tag(b"{}").is_err());
    }
}
