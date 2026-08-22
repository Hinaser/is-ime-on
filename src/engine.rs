//! 監視エンジン。バックグラウンドスレッド(MTA COM)でIME状態とキャレット位置を読み、
//! メインスレッドへ PostMessage(WM_APP_STATE)で通知する。
//! ハングしたアプリ相手でもUIが固まらず、UIA呼び出しも推奨形態(MTA・非UIスレッド)になる。
//!
//! 省電力設計: WinEvent フック(フォアグラウンド切替・フォーカス移動・キャレット移動)からの
//! poke() で即時再読込しつつ、IDLE_AFTER 以上イベントがなければポーリングを IDLE_INTERVAL へ
//! バックオフする。イベントが来れば即座に通常間隔へ復帰する。

use crate::caret::{self, CaretInfo};
use crate::config::ImeMode;
use crate::ime;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// ポーラーが新しい状態を用意したときにメインウィンドウへ届くメッセージ。
pub const WM_APP_STATE: u32 = WM_APP + 1;

/// この時間イベントがなければアイドルとみなしてバックオフ。
const IDLE_AFTER: Duration = Duration::from_secs(10);
/// アイドル時のポーリング間隔。
const IDLE_INTERVAL: Duration = Duration::from_millis(500);

struct Inner {
    poked: bool,
    quit: bool,
    paused: bool,
    /// index = ImeMode::index()。モードが非表示ならキャレット取得(UIA含む)を丸ごと省く。
    visible_by_mode: [bool; 7],
    last_event: Instant,
}

pub struct Engine {
    state: Arc<(Mutex<Inner>, Condvar)>,
    /// 最後に読み取った (モード, キャレット)。メインスレッドが WM_APP_STATE 受信時に取り出す。
    latest: Arc<Mutex<(ImeMode, Option<CaretInfo>)>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// `main_hwnd` は WM_APP_STATE の送り先。`poll_ms` は通常時のポーリング間隔。
    pub fn start(main_hwnd: HWND, poll_ms: u32, visible_by_mode: [bool; 7]) -> Engine {
        let state = Arc::new((
            Mutex::new(Inner {
                poked: true, // 起動直後に一度読む
                quit: false,
                paused: false,
                visible_by_mode,
                last_event: Instant::now(),
            }),
            Condvar::new(),
        ));
        let latest = Arc::new(Mutex::new((ImeMode::Off, None)));

        let hwnd_raw = main_hwnd.0 as usize; // HWND は Send でないため生値で渡す
        let state2 = Arc::clone(&state);
        let latest2 = Arc::clone(&latest);
        let poll = Duration::from_millis(poll_ms.clamp(30, 1000) as u64);

        let thread = std::thread::Builder::new()
            .name("poller".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                poller_loop(state2, latest2, hwnd_raw, poll);
            })
            .expect("spawn poller");

        Engine { state, latest, thread: Some(thread) }
    }

    /// WinEvent フックなどから: 直ちに再読込させ、アイドルバックオフを解除する。
    pub fn poke(&self) {
        let (lock, cvar) = &*self.state;
        let mut inner = lock.lock().unwrap();
        inner.poked = true;
        inner.last_event = Instant::now();
        cvar.notify_one();
    }

    pub fn set_paused(&self, paused: bool) {
        let (lock, cvar) = &*self.state;
        let mut inner = lock.lock().unwrap();
        inner.paused = paused;
        inner.poked = true;
        cvar.notify_one();
    }

    pub fn set_visible_by_mode(&self, visible: [bool; 7]) {
        let (lock, cvar) = &*self.state;
        let mut inner = lock.lock().unwrap();
        inner.visible_by_mode = visible;
        inner.poked = true;
        inner.last_event = Instant::now();
        cvar.notify_one();
    }

    /// メインスレッドが WM_APP_STATE 受信時に呼ぶ。
    pub fn take_latest(&self) -> (ImeMode, Option<CaretInfo>) {
        *self.latest.lock().unwrap()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        {
            let (lock, cvar) = &*self.state;
            lock.lock().unwrap().quit = true;
            cvar.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn poller_loop(
    state: Arc<(Mutex<Inner>, Condvar)>,
    latest: Arc<Mutex<(ImeMode, Option<CaretInfo>)>>,
    hwnd_raw: usize,
    poll: Duration,
) {
    let (lock, cvar) = &*state;
    loop {
        // 次の読み取りタイミングまで待機(poke で即時解除)
        let (paused, visible_by_mode) = {
            let mut inner = lock.lock().unwrap();
            if !inner.poked {
                let idle = inner.last_event.elapsed() > IDLE_AFTER;
                let interval = if idle { IDLE_INTERVAL } else { poll };
                let (guard, _) = cvar.wait_timeout(inner, interval).unwrap();
                inner = guard;
            }
            if inner.quit {
                return;
            }
            inner.poked = false;
            (inner.paused, inner.visible_by_mode)
        };

        // ロック外で読み取り(ハング中のアプリで最大200ms×2+UIA分ブロックしうる)
        let (fg, info) = caret::foreground_thread_info();
        if fg.is_invalid() {
            publish(&latest, hwnd_raw, ImeMode::Off, None);
            continue;
        }
        // 子コントロールがフォーカスを持つ場合はそちらを優先
        let target = info
            .filter(|i| !i.hwndFocus.is_invalid())
            .map(|i| i.hwndFocus)
            .unwrap_or(fg);
        let mode = ime::read_mode(target);

        let want_caret = !paused && visible_by_mode[mode.index()];
        let caret = if want_caret { caret::get_caret() } else { None };
        publish(&latest, hwnd_raw, mode, caret);
    }
}

fn publish(
    latest: &Arc<Mutex<(ImeMode, Option<CaretInfo>)>>,
    hwnd_raw: usize,
    mode: ImeMode,
    caret: Option<CaretInfo>,
) {
    *latest.lock().unwrap() = (mode, caret);
    unsafe {
        let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
        let _ = PostMessageW(Some(hwnd), WM_APP_STATE, WPARAM(0), LPARAM(0));
    }
}
