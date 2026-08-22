//! UI Automation によるキャレット位置取得(フォールバック)。
//! システムキャレットを使わないアプリ(Chrome・Windows Terminal・Electron系など)向け。
//!
//! 取得経路は次の順に試す:
//!
//! 1. TextPattern2.GetCaretRange (対応アプリ)
//! 2. TextPattern.GetSelection の先頭レンジ(キャレット=長さ0の選択。Windows Terminal など)
//!
//! レンジが折りたたまれて矩形が空の場合は1文字分に広げ、その左端をキャレットとみなす。
//! COM を使うため、呼び出しスレッドは CoInitializeEx 済みであること(ポーラースレッドで初期化)。

use crate::caret::CaretInfo;
use std::cell::RefCell;
use windows::core::{Interface, BOOL};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayUnaccessData,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationTextPattern2, IUIAutomationTextRange, TextPatternRangeEndpoint_End,
    TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPattern2Id, UIA_TextPatternId,
};

thread_local! {
    static UIA: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

fn with_uia<T>(f: impl FnOnce(&IUIAutomation) -> Option<T>) -> Option<T> {
    UIA.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = unsafe {
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()
            };
        }
        slot.as_ref().and_then(f)
    })
}

pub fn get_caret() -> Option<CaretInfo> {
    let el = with_uia(|uia| unsafe { uia.GetFocusedElement().ok() })?;
    // TextPattern2 が失敗したり使えない矩形を返しても TextPattern1 を試す
    from_caret_range(&el).or_else(|| from_selection(&el))
}

/// TextPattern2.GetCaretRange 経由(対応アプリ)。
fn from_caret_range(el: &IUIAutomationElement) -> Option<CaretInfo> {
    unsafe {
        let pattern = el.GetCurrentPattern(UIA_TextPattern2Id).ok()?;
        let tp2: IUIAutomationTextPattern2 = pattern.cast().ok()?;
        let mut active = BOOL(0);
        let range = tp2.GetCaretRange(&mut active).ok()?;
        range_to_caret(&range)
    }
}

/// TextPattern.GetSelection の先頭レンジ経由(Windows Terminal など)。
fn from_selection(el: &IUIAutomationElement) -> Option<CaretInfo> {
    unsafe {
        let pattern = el.GetCurrentPattern(UIA_TextPatternId).ok()?;
        let tp1: IUIAutomationTextPattern = pattern.cast().ok()?;
        let sel = tp1.GetSelection().ok()?;
        if sel.Length().ok()? == 0 {
            return None;
        }
        let range = sel.GetElement(0).ok()?;
        range_to_caret(&collapse_to_end(range))
    }
}

/// 選択範囲・IME変換範囲(非デジェネレート)が返ってきた場合、末尾に潰してキャレット位置にする。
/// Chromium は変換中に GetSelection で変換範囲全体を返すため、先頭矩形を使うと
/// 変換開始位置にズレる。キャレットは末尾側にある。
fn collapse_to_end(range: IUIAutomationTextRange) -> IUIAutomationTextRange {
    unsafe {
        let already_degenerate = range
            .CompareEndpoints(TextPatternRangeEndpoint_Start, &range, TextPatternRangeEndpoint_End)
            .map(|c| c == 0)
            .unwrap_or(true);
        if already_degenerate {
            return range;
        }
        match range.Clone() {
            Ok(collapsed) => {
                if collapsed
                    .MoveEndpointByRange(TextPatternRangeEndpoint_Start, &range, TextPatternRangeEndpoint_End)
                    .is_ok()
                {
                    collapsed
                } else {
                    range
                }
            }
            Err(_) => range,
        }
    }
}

fn range_to_caret(range: &IUIAutomationTextRange) -> Option<CaretInfo> {
    unsafe {
        let mut rects = bounding_rects(range);
        let mut expanded = false;
        if rects.len() < 4 {
            range.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
            rects = bounding_rects(range);
            expanded = true;
        }
        if rects.len() < 4 {
            return None;
        }

        // 先頭矩形 (x, y, w, h)。UIAの座標はスクリーン物理px。
        let (x, y) = (rects[0] as i32, rects[1] as i32);
        let (mut w, h) = (rects[2] as i32, rects[3] as i32);
        if !(2..=400).contains(&h) {
            return None;
        }

        // 1文字に広げた場合はセル左端を細いキャレットとして扱う
        if expanded {
            w = 1;
        }

        let mut x = x;
        // GetSelection 由来で範囲選択中(幅広)の場合は選択の右端をキャレット位置とみなす
        if w > 60 {
            x += w;
            w = 1;
        }

        Some(CaretInfo { x, y, width: w.max(1), height: h })
    }
}

/// SAFEARRAY(f64) → Vec<f64>。所有権を受け取り破棄まで行う。
unsafe fn bounding_rects(range: &IUIAutomationTextRange) -> Vec<f64> {
    let Ok(psa) = range.GetBoundingRectangles() else {
        return Vec::new();
    };
    if psa.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let lb = SafeArrayGetLBound(psa, 1).unwrap_or(0);
    let ub = SafeArrayGetUBound(psa, 1).unwrap_or(-1);
    let n = (ub - lb + 1).max(0) as usize;
    if n > 0 {
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        if SafeArrayAccessData(psa, &mut data).is_ok() {
            out.extend_from_slice(std::slice::from_raw_parts(data as *const f64, n));
            let _ = SafeArrayUnaccessData(psa);
        }
    }
    let _ = SafeArrayDestroy(psa);
    out
}
