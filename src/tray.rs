//! タスクトレイ常駐。アイコンは現在モードの色の角丸バッジ+バッジ文字
//! (オーバーレイ・アプリアイコンと同じモチーフ)。
//! 描画はオーバーレイと同じ Direct2D/DirectWrite で行い、SM_CXSMICON の実サイズで
//! レンダリングする(高DPIでも滲まない)。

use crate::shape;
use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetSystemMetrics, SetForegroundWindow, SetTimer, TrackPopupMenu, HICON, ICONINFO, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, SM_CXSMICON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP,
};

/// トレイアイコンからのコールバックメッセージ。
pub const WM_APP_TRAY: u32 = WM_APP + 2;

pub const CMD_OPEN: u32 = 1;
pub const CMD_PAUSE: u32 = 2;
pub const CMD_EXIT: u32 = 3;
pub const CMD_RELOAD: u32 = 4;

const TRAY_ID: u32 = 1;

/// トレイ登録に失敗したときの再試行タイマー(WM_TIMER の wParam)。
pub const TIMER_TRAY_RETRY: usize = 1;
pub const TRAY_RETRY_MS: u32 = 2000;

pub struct Tray {
    hwnd: HWND,
    icon: Option<HICON>,
    renderer: Option<BadgeIconRenderer>,
    tip: String,
    /// シェルに登録済みか。ログオン直後(Explorer 未準備)や Explorer 再起動で
    /// 登録が失われるため、false の間は呼び出し側が ensure_added で再試行する。
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        let mut renderer = BadgeIconRenderer::new().ok();
        let icon = renderer
            .as_mut()
            .and_then(|r| r.render((0, 0, 0), "A", None));
        let mut tray = Tray {
            hwnd,
            icon,
            renderer,
            tip: "IsImeOn".to_string(),
            added: false,
        };
        tray.add();
        tray
    }

    /// 未登録なら登録を試みる。登録済みになったら true。
    pub fn ensure_added(&mut self) -> bool {
        if !self.added {
            self.add();
        }
        self.added
    }

    /// Explorer 再起動(TaskbarCreated)時に呼ぶ。旧登録は消えているので登録し直す。
    pub fn readd(&mut self) -> bool {
        self.added = false;
        self.ensure_added()
    }

    fn full_nid(&self) -> NOTIFYICONDATAW {
        let mut nid = base_nid(self.hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        if let Some(h) = self.icon {
            nid.hIcon = h;
        }
        set_tip(&mut nid, &self.tip);
        nid
    }

    fn add(&mut self) {
        let nid = self.full_nid();
        // Explorer が高負荷だと NIM_ADD がタイムアウトで FALSE を返しつつ実際には
        // 追加されていることがある。その場合 NIM_ADD の再試行は失敗し続けるので、
        // NIM_MODIFY が通るかで登録済みかを確かめる。
        self.added = unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid).as_bool()
                || Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool()
        };
        if !self.added {
            // どの経路で失敗しても再試行が走るようにする(同IDの SetTimer は再設定になるだけ)
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_TRAY_RETRY, TRAY_RETRY_MS, None);
            }
        }
    }

    /// アイコン(モード色の角丸バッジ+文字)とツールチップを現在モードに合わせて更新。
    pub fn update(
        &mut self,
        color: (u8, u8, u8),
        label: &str,
        label_color: Option<(u8, u8, u8)>,
        tip: &str,
    ) {
        let new_icon = self
            .renderer
            .as_mut()
            .and_then(|r| r.render(color, label, label_color));
        let old_icon = if new_icon.is_some() {
            std::mem::replace(&mut self.icon, new_icon)
        } else {
            None
        };
        self.tip = tip.to_string();
        if self.added {
            let nid = self.full_nid();
            if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) }.as_bool() {
                // 登録が消えている(Explorer 再起動の取りこぼし等)。登録し直す
                self.add();
            }
        } else {
            self.add();
        }
        if let Some(old) = old_icon {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
    }

    /// 右クリックメニューを表示し、選ばれたコマンドID(CMD_*)を返す。0 = 選択なし。
    ///
    /// TrackPopupMenu は入れ子のメッセージループを回すため、この関数の実行中に
    /// wndproc が再入する。App の RefCell を借りたまま呼ぶと二重借用でパニックする
    /// (release は panic=abort なのでプロセスごと落ちる)ので、
    /// あえて &self を取らず hwnd と paused だけで動くようにしてある。
    pub fn show_menu(hwnd: HWND, paused: bool) -> u32 {
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
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                pt.x,
                pt.y,
                Some(0),
                hwnd,
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

/// モード色の角丸バッジ+文字のトレイアイコンを Direct2D で描く。
/// オーバーレイと同じ描画系なので、字形・自動文字色の見え方が完全に一致する。
struct BadgeIconRenderer {
    dwrite: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    mem_dc: HDC,
    dib: HBITMAP,
    bits: *mut u8,
    size: i32,
}

impl BadgeIconRenderer {
    fn new() -> windows::core::Result<BadgeIconRenderer> {
        unsafe {
            let size = GetSystemMetrics(SM_CXSMICON).max(16);
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = d2d.CreateDCRenderTarget(&props)?;
            let mem_dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            windows::Win32::Graphics::Gdi::SelectObject(mem_dc, HGDIOBJ(dib.0));
            Ok(BadgeIconRenderer {
                dwrite,
                rt,
                mem_dc,
                dib,
                bits: bits as *mut u8,
                size,
            })
        }
    }

    fn render(
        &mut self,
        color: (u8, u8, u8),
        label: &str,
        label_color: Option<(u8, u8, u8)>,
    ) -> Option<HICON> {
        let label = if label.is_empty() { "?" } else { label };
        let s = self.size as f32;
        unsafe {
            let rect = RECT { left: 0, top: 0, right: self.size, bottom: self.size };
            self.rt.BindDC(self.mem_dc, &rect).ok()?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            let to_f = |c: (u8, u8, u8)| D2D1_COLOR_F {
                r: c.0 as f32 / 255.0,
                g: c.1 as f32 / 255.0,
                b: c.2 as f32 / 255.0,
                a: 1.0,
            };
            let bg = self.rt.CreateSolidColorBrush(&to_f(color), None).ok()?;
            let corner = s * shape::BADGE_CORNER_RATIO;
            let rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: 0.5, top: 0.5, right: s - 0.5, bottom: s - 0.5 },
                radiusX: corner,
                radiusY: corner,
            };
            self.rt.FillRoundedRectangle(&rr, &bg);

            let text_rgb = label_color.unwrap_or_else(|| shape::auto_text_color(color));
            let text_brush = self.rt.CreateSolidColorBrush(&to_f(text_rgb), None).ok()?;
            let font_size = shape::badge_font_size(s * 0.98, label.chars().count());
            let format = self
                .dwrite
                .CreateTextFormat(
                    w!("Yu Gothic UI"),
                    None,
                    DWRITE_FONT_WEIGHT_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    w!("ja-jp"),
                )
                .ok()?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER).ok()?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER).ok()?;
            let utf16: Vec<u16> = label.encode_utf16().collect();
            let layout = D2D_RECT_F { left: 0.0, top: 0.0, right: s, bottom: s };
            self.rt.DrawText(
                &utf16,
                &format,
                &layout,
                &text_brush,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL,
            );
            self.rt.EndDraw(None, None).ok()?;

            // D2D はプリマルチプライド。アイコンはストレートアルファ前提なので戻す
            let n = (self.size * self.size) as usize;
            let px = std::slice::from_raw_parts_mut(self.bits, n * 4);
            for p in px.as_chunks_mut::<4>().0 {
                let a = p[3] as u32;
                if a > 0 && a < 255 {
                    p[0] = (p[0] as u32 * 255 / a).min(255) as u8;
                    p[1] = (p[1] as u32 * 255 / a).min(255) as u8;
                    p[2] = (p[2] as u32 * 255 / a).min(255) as u8;
                }
            }

            // デバッグビルドでは検証用に BMP を書き出す
            #[cfg(debug_assertions)]
            self.dump_debug_bmp(px);

            let mask = CreateBitmap(self.size, self.size, 1, 1, None);
            let info = ICONINFO {
                fIcon: true.into(),
                hbmColor: self.dib,
                hbmMask: mask,
                ..Default::default()
            };
            let icon = CreateIconIndirect(&info).ok(); // ビットマップは複製される
            let _ = DeleteObject(HGDIOBJ(mask.0));
            icon
        }
    }

    #[cfg(debug_assertions)]
    fn dump_debug_bmp(&self, px: &[u8]) {
        let s = self.size;
        let mut bmp = Vec::with_capacity(54 + px.len());
        let data_len = px.len() as u32;
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + data_len).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&s.to_le_bytes());
        bmp.extend_from_slice(&(-s).to_le_bytes()); // top-down
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(px);
        let path = std::env::temp_dir().join("isimeon_tray_dbg.bmp");
        let _ = std::fs::write(path, bmp);
    }
}

impl Drop for BadgeIconRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.dib.0));
            let _ = DeleteDC(self.mem_dc);
        }
    }
}
