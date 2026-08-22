//! タスクトレイ常駐。アイコンは現在モードの色の丸。

use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    SetForegroundWindow, TrackPopupMenu, HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

/// トレイアイコンからのコールバックメッセージ。
pub const WM_APP_TRAY: u32 = WM_APP + 2;

pub const CMD_OPEN: u32 = 1;
pub const CMD_PAUSE: u32 = 2;
pub const CMD_EXIT: u32 = 3;
pub const CMD_RELOAD: u32 = 4;

const TRAY_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    icon: Option<HICON>,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        let icon = create_circle_icon((0, 0, 0));
        let mut nid = base_nid(hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        if let Some(h) = icon {
            nid.hIcon = h;
        }
        set_tip(&mut nid, "IsImeOn");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        Tray { hwnd, icon }
    }

    /// アイコンの色とツールチップを現在モードに合わせて更新。
    pub fn update(&mut self, color: (u8, u8, u8), tip: &str) {
        let new_icon = create_circle_icon(color);
        let mut nid = base_nid(self.hwnd);
        nid.uFlags = NIF_ICON | NIF_TIP;
        if let Some(h) = new_icon {
            nid.hIcon = h;
        }
        set_tip(&mut nid, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            if let Some(old) = self.icon.take() {
                let _ = DestroyIcon(old);
            }
        }
        self.icon = new_icon;
    }

    /// 右クリックメニューを表示し、選ばれたコマンドID(CMD_*)を返す。0 = 選択なし。
    pub fn show_menu(&self, paused: bool) -> u32 {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return 0;
            };
            let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN as usize, w!("設定を開く(&O)"));
            let _ = AppendMenuW(menu, MF_STRING, CMD_RELOAD as usize, w!("設定を再読込(&R)"));
            let pause_flags = if paused { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, pause_flags, CMD_PAUSE as usize, w!("一時停止(&P)"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT as usize, w!("終了(&X)"));

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // メニューを閉じられるようにするための定石
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                pt.x,
                pt.y,
                Some(0),
                self.hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
            cmd.0 as u32
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let nid = base_nid(self.hwnd);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            if let Some(icon) = self.icon.take() {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

fn set_tip(nid: &mut NOTIFYICONDATAW, tip: &str) {
    let utf16: Vec<u16> = tip.encode_utf16().take(127).collect();
    nid.szTip[..utf16.len()].copy_from_slice(&utf16);
    nid.szTip[utf16.len()] = 0;
}

/// 16x16 のアンチエイリアス付き丸アイコン(塗り=指定色、縁=グレー)を生成する。
fn create_circle_icon(rgb: (u8, u8, u8)) -> Option<HICON> {
    const N: i32 = 16;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: N,
            biHeight: -N, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (N * N) as usize);

        let (cx, cy) = (8.0f32, 8.0f32);
        let r_outer = 6.5f32; // 縁の外径
        let r_inner = 5.3f32; // 塗りの半径
        let border = (90u8, 90u8, 90u8);
        for y in 0..N {
            for x in 0..N {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                // 縁の円の上に塗りの円を重ねる(BGRA・プリマルチプライド)
                let a_outer = (r_outer - d + 0.5).clamp(0.0, 1.0);
                let a_inner = (r_inner - d + 0.5).clamp(0.0, 1.0);
                let mix = |b: u8, f: u8| -> f32 {
                    b as f32 * (1.0 - a_inner) + f as f32 * a_inner
                };
                let (r, g, b) = (
                    mix(border.0, rgb.0),
                    mix(border.1, rgb.1),
                    mix(border.2, rgb.2),
                );
                let a = a_outer;
                let pm = |c: f32| -> u32 { (c * a) as u32 };
                px[(y * N + x) as usize] =
                    ((a * 255.0) as u32) << 24 | pm(r) << 16 | pm(g) << 8 | pm(b);
            }
        }

        let mask = CreateBitmap(N, N, 1, 1, None);
        let info = ICONINFO {
            fIcon: true.into(),
            hbmColor: dib,
            hbmMask: mask,
            ..Default::default()
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}
