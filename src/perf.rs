//! 性能ログ(オプトイン)。config の `PerfLog` が true のときだけ動く。
//! 処理ごとの所要時間を集計し、1分ごとに %APPDATA%\IsImeOn\perf.log へ1行のサマリーを書く
//! (回数・平均・最大と、プロセスのCPU時間・メモリ)。SLOW を超えた1回は前面アプリ名つきで即時に記録する。
//! 無効化・終了時は途中までの集計を書いてから止める(再現直後にオフにしても取りこぼさない)。
//! 無効時のコストは計測点ごとの AtomicBool 読み取り1回だけ。
//! ロック中は集計の更新だけを行い、ファイル書き込み・プロセス名の取得などはロック外で行う。

use crate::config::AppConfig;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// サマリーを書く間隔。
const FLUSH_EVERY: Duration = Duration::from_secs(60);
/// これを超えた1回は個別に記録する。
const SLOW: Duration = Duration::from_millis(50);
/// これを超えたら perf.old.log へ回す。
const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy)]
pub enum Metric {
    /// IME 状態の読み取り(ime::read_mode)
    Ime,
    /// システムキャレットの取得(GetGUIThreadInfo)
    Caret,
    /// UI Automation によるキャレット取得(システムキャレットがないアプリ)
    Uia,
    /// オーバーレイの再描画
    Render,
}

const METRICS: [(Metric, &str); 4] = [
    (Metric::Ime, "ime"),
    (Metric::Caret, "caret"),
    (Metric::Uia, "uia"),
    (Metric::Render, "render"),
];

#[derive(Clone, Copy, Default)]
struct Agg {
    count: u32,
    total: Duration,
    max: Duration,
}

struct Stats {
    since: Instant,
    cpu_at_since: Duration,
    polls: u32,
    events: u32,
    aggs: [Agg; 4],
}

impl Stats {
    fn new() -> Stats {
        Stats {
            since: Instant::now(),
            cpu_at_since: process_cpu_time(),
            polls: 0,
            events: 0,
            aggs: [Agg::default(); 4],
        }
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATS: Mutex<Option<Stats>> = Mutex::new(None);

/// config の設定を反映する。有効化時はヘッダー行を、無効化時は途中までの集計と stop 行を書く。
pub fn configure(config: &AppConfig) {
    let enable = config.perf_log;
    let was = ENABLED.swap(enable, Ordering::Relaxed);
    if enable && !was {
        let fresh = Stats::new();
        *STATS.lock().unwrap() = Some(fresh);
        append(&format!(
            "{} start v{} poll={}ms shape={} position={}\n",
            timestamp(),
            env!("CARGO_PKG_VERSION"),
            config.poll_interval_ms,
            config.shape,
            config.position,
        ));
    } else if !enable && was {
        stop();
    }
}

/// 終了時に呼ぶ。有効なら途中までの集計と stop 行を書く。
pub fn shutdown() {
    if ENABLED.swap(false, Ordering::Relaxed) {
        stop();
    }
}

fn stop() {
    let pending = STATS.lock().unwrap().take();
    if let Some(done) = pending {
        let mut text = summarize(&done, process_cpu_time());
        text.push_str(&format!("{} stop\n", timestamp()));
        append(&text);
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// 計測開始。無効なら None で、record は何もしない。
pub fn start() -> Option<Instant> {
    enabled().then(Instant::now)
}

/// 計測終了。`fg` は閾値超え時に記録する前面ウィンドウ(アプリ名の特定用)。
pub fn record(metric: Metric, started: Option<Instant>, fg: Option<HWND>) {
    let Some(started) = started else { return };
    let elapsed = started.elapsed();
    {
        let mut guard = STATS.lock().unwrap();
        let Some(stats) = guard.as_mut() else { return };
        let agg = &mut stats.aggs[metric as usize];
        agg.count += 1;
        agg.total += elapsed;
        agg.max = agg.max.max(elapsed);
    }
    // 遅い1回はまれなので、その場で書く(集計の flush を待つと、無効化や強制終了で失われうる)
    if elapsed >= SLOW {
        let app = fg.and_then(process_name).unwrap_or_else(|| "-".into());
        append(&format!(
            "{} slow {} {} ({app})\n",
            timestamp(),
            METRICS[metric as usize].1,
            fmt_dur(elapsed),
        ));
    }
}

/// ポーラーの1回分の読み取り。
pub fn count_poll() {
    if enabled()
        && let Some(stats) = STATS.lock().unwrap().as_mut()
    {
        stats.polls += 1;
    }
}

/// WinEvent フックからの通知。
pub fn count_event() {
    if enabled()
        && let Some(stats) = STATS.lock().unwrap().as_mut()
    {
        stats.events += 1;
    }
}

/// FLUSH_EVERY 経過していればサマリーを書く。ポーラースレッドから定期的に呼ぶ
/// (アイドル時も500msごとに回るので、書き出しが大きく遅れることはない)。
pub fn maybe_flush() {
    if !enabled() {
        return;
    }
    let due = STATS
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|s| s.since.elapsed() >= FLUSH_EVERY);
    if !due {
        return;
    }
    // 新しい区間の起点(CPU時間の取得)はロック外で作る
    let fresh = Stats::new();
    let cpu_now = fresh.cpu_at_since;
    let done = match STATS.lock().unwrap().as_mut() {
        Some(stats) => std::mem::replace(stats, fresh),
        None => return, // その間に無効化された
    };
    append(&summarize(&done, cpu_now));
}

/// 区間 `s` のサマリー1行。`cpu_now` は区間終了時点のプロセスCPU時間。
fn summarize(s: &Stats, cpu_now: Duration) -> String {
    let mut out = String::new();
    let (ws, private) = process_memory();
    let _ = write!(
        out,
        "{} interval={}s cpu={} ws={:.1}MB private={:.1}MB polls={} events={}",
        timestamp(),
        s.since.elapsed().as_secs(),
        fmt_dur(cpu_now.saturating_sub(s.cpu_at_since)),
        ws as f64 / 1048576.0,
        private as f64 / 1048576.0,
        s.polls,
        s.events,
    );
    for (metric, name) in METRICS {
        let a = s.aggs[metric as usize];
        if a.count > 0 {
            let _ = write!(
                out,
                " {name}={}/avg {}/max {}",
                a.count,
                fmt_dur(a.total / a.count),
                fmt_dur(a.max),
            );
        }
    }
    out.push('\n');
    out
}

pub fn log_path() -> PathBuf {
    AppConfig::dir_path().join("perf.log")
}

fn append(text: &str) {
    let path = log_path();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_file_name("perf.old.log"));
    }
    let _ = std::fs::create_dir_all(AppConfig::dir_path());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(text.as_bytes());
    }
}

fn fmt_dur(d: Duration) -> String {
    let us = d.as_micros();
    if us >= 1000 {
        format!("{:.1}ms", us as f64 / 1000.0)
    } else {
        format!("{us}us")
    }
}

fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn process_cpu_time() -> Duration {
    let (mut c, mut e, mut k, mut u) = Default::default();
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.is_err() {
        return Duration::ZERO;
    }
    let ticks = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    // FILETIME は 100ns 単位
    Duration::from_nanos((ticks(k) + ticks(u)) * 100)
}

/// (ワーキングセット, プライベート) バイト。
fn process_memory() -> (usize, usize) {
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut _, pmc.cb)
    };
    if ok.is_err() {
        return (0, 0);
    }
    (pmc.WorkingSetSize, pmc.PrivateUsage)
}

/// ウィンドウを持つプロセスの exe 名(例: chrome.exe)。
fn process_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_dur_switches_units() {
        assert_eq!(fmt_dur(Duration::from_micros(250)), "250us");
        assert_eq!(fmt_dur(Duration::from_micros(2500)), "2.5ms");
    }

    #[test]
    fn metric_names_match_enum_order() {
        for (i, (metric, _)) in METRICS.iter().enumerate() {
            assert_eq!(*metric as usize, i);
        }
    }

    #[test]
    fn summary_lists_only_measured_metrics() {
        let mut s = Stats::new();
        s.polls = 3;
        s.aggs[Metric::Ime as usize] = Agg {
            count: 2,
            total: Duration::from_micros(60),
            max: Duration::from_micros(40),
        };
        let line = summarize(&s, s.cpu_at_since);
        assert!(line.contains(" polls=3 "));
        assert!(line.contains(" ime=2/avg 30us/max 40us"));
        assert!(!line.contains(" uia="));
        assert!(line.ends_with('\n'));
    }
}
