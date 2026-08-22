//! キャレット位置に重ねてインジケーターを描く透明・クリック透過・非アクティブのオーバーレイ。
//! 32bpp DIB に Direct2D(DCRenderTarget)で描き、UpdateLayeredWindow で表示する。
//! 座標・サイズはすべて物理px(プロセスは Per-Monitor DPI aware)。

use crate::caret::CaretInfo;
use crate::config::IndicatorShape;
use crate::shape::{self, Metrics};
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows_numerics::Vector2;
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SetWindowPos, ShowWindow, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNA, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::Win32::UI::WindowsAndMessaging::UpdateLayeredWindow;

/// 描画パラメータ一式。前回と同一なら再描画・再表示をスキップする。
#[derive(Clone, PartialEq)]
pub struct DrawParams {
    pub caret: CaretInfo,
    pub color: (u8, u8, u8),
    pub size: i32,
    pub shape: IndicatorShape,
    pub label: String,
    pub label_color: Option<(u8, u8, u8)>,
}

pub struct Overlay {
    hwnd: HWND,
    dwrite: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    mem_dc: HDC,
    dib: Option<(HBITMAP, i32, i32)>, // (bitmap, w, h) grow-only
    visible: bool,
    last: Option<DrawParams>,
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl Overlay {
    pub fn new() -> Result<Overlay> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class_name = w!("IsImeOnOverlay");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
                    | WS_EX_TOPMOST,
                class_name,
                w!("IsImeOn overlay"),
                WS_POPUP,
                -32000,
                -32000,
                10,
                10,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;

            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // SOFTWARE: 数十px の図形描画に GPU デバイスは過剰で、D3D デバイス分のメモリ
            // (数十MB)を節約できる。CPUラスタライズでも描画は数十µs程度。
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
            // RT が factory を COM 参照で保持するため、factory 自体は持ち続けなくてよい
            let rt = d2d.CreateDCRenderTarget(&props)?;
            let mem_dc = CreateCompatibleDC(None);

            Ok(Overlay {
                hwnd,
                dwrite,
                rt,
                mem_dc,
                dib: None,
                visible: false,
                last: None,
            })
        }
    }

    /// DIB を必要サイズ以上に確保(grow-only)。
    fn ensure_dib(&mut self, w: i32, h: i32) -> Result<()> {
        if let Some((_, cw, ch)) = self.dib {
            if cw >= w && ch >= h {
                return Ok(());
            }
        }
        unsafe {
            if let Some((old, _, _)) = self.dib.take() {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
            let w = w.max(64);
            let h = h.max(64);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            SelectObject(self.mem_dc, HGDIOBJ(dib.0));
            self.dib = Some((dib, w, h));
        }
        Ok(())
    }

    pub fn show(&mut self, params: DrawParams) {
        if self.visible && self.last.as_ref() == Some(&params) {
            return; // 変化なし: 再描画・再配置ともに不要
        }
        if self.render(&params).is_ok() {
            self.visible = true;
            self.last = Some(params);
        }
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        self.last = None;
    }

    fn render(&mut self, p: &DrawParams) -> Result<()> {
        let metrics = Metrics::overlay(p.size);
        let margin = metrics.overlay_margin(p.shape);
        let win_x = p.caret.x + p.caret.width / 2 - margin;
        let win_y = p.caret.y - margin;
        let win_w = margin * 2;
        let win_h = p.caret.height + margin * 2;

        self.ensure_dib(win_w, win_h)?;

        let cx = margin as f32;
        let caret_top = margin as f32;
        let caret_bottom = (margin + p.caret.height) as f32;
        let color = color_f(p.color, 1.0);

        unsafe {
            let bind_rect = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: win_w,
                bottom: win_h,
            };
            self.rt.BindDC(self.mem_dc, &bind_rect)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            let brush = self.rt.CreateSolidColorBrush(&color, None)?;
            let r = metrics.blob_r;
            match p.shape {
                IndicatorShape::Teardrop => {
                    self.rt.FillEllipse(&ellipse(shape::top_circle(r, cx, caret_top)), &brush);
                    self.rt
                        .FillEllipse(&ellipse(shape::lower_circle(r, cx, caret_bottom)), &brush);
                }
                IndicatorShape::TopCircle => {
                    self.rt.FillEllipse(&ellipse(shape::top_circle(r, cx, caret_top)), &brush);
                }
                IndicatorShape::MidCircle => {
                    let translucent = self
                        .rt
                        .CreateSolidColorBrush(&color_f(p.color, shape::MID_CIRCLE_ALPHA), None)?;
                    let e = ellipse(shape::mid_circle(r, cx, caret_top, caret_bottom));
                    self.rt.FillEllipse(&e, &translucent);
                    self.rt.DrawEllipse(&e, &brush, 1.2, None);
                }
                IndicatorShape::Badge => {
                    self.draw_badge(p, metrics.badge_side, cx, caret_top)?;
                }
            }

            self.rt.EndDraw(None, None)?;

            // 画面へ転送(位置・サイズも同時に設定される)
            let screen_dc = GetDC(None);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dst = POINT { x: win_x, y: win_y };
            let size = SIZE { cx: win_w, cy: win_h };
            let src = POINT { x: 0, y: 0 };
            let result = UpdateLayeredWindow(
                self.hwnd,
                Some(screen_dc),
                Some(&dst),
                Some(&size),
                Some(self.mem_dc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen_dc);
            result?;

            if !self.visible {
                let _ = ShowWindow(self.hwnd, SW_SHOWNA);
            }
            // 最前面の維持(他の topmost ウィンドウ対策)
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Ok(())
    }

    fn draw_badge(&self, p: &DrawParams, side: f32, cx: f32, caret_top: f32) -> Result<()> {
        let label = if p.label.is_empty() { "?" } else { &p.label };
        let (left, top) = shape::badge_origin(side, cx, caret_top);
        let rect = D2D_RECT_F {
            left,
            top,
            right: left + side,
            bottom: top + side,
        };
        unsafe {
            let bg = self.rt.CreateSolidColorBrush(&color_f(p.color, 1.0), None)?;
            let rounded = D2D1_ROUNDED_RECT {
                rect,
                radiusX: side * shape::BADGE_CORNER_RATIO,
                radiusY: side * shape::BADGE_CORNER_RATIO,
            };
            self.rt.FillRoundedRectangle(&rounded, &bg);

            // 文字色: 指定色、または背景の明度で白/黒を自動選択
            let text_rgb = p
                .label_color
                .unwrap_or_else(|| shape::auto_text_color(p.color));
            let text_brush = self.rt.CreateSolidColorBrush(&color_f(text_rgb, 1.0), None)?;

            let font_size = shape::badge_font_size(side, label.chars().count());
            let format = self.dwrite.CreateTextFormat(
                w!("Yu Gothic UI"),
                None,
                DWRITE_FONT_WEIGHT_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font_size,
                w!("ja-jp"),
            )?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

            let utf16: Vec<u16> = label.encode_utf16().collect();
            self.rt.DrawText(
                &utf16,
                &format,
                &rect,
                &text_brush,
                windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if let Some((dib, _, _)) = self.dib.take() {
                let _ = DeleteObject(HGDIOBJ(dib.0));
            }
            let _ = DeleteDC(self.mem_dc);
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
    }
}

fn color_f(rgb: (u8, u8, u8), a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: rgb.0 as f32 / 255.0,
        g: rgb.1 as f32 / 255.0,
        b: rgb.2 as f32 / 255.0,
        a,
    }
}

fn ellipse((x, y, r): shape::Circle) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: Vector2 { X: x, Y: y },
        radiusX: r,
        radiusY: r,
    }
}
