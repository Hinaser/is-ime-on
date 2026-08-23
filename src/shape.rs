//! インジケーター形状の幾何計算。オーバーレイ(Direct2D・物理px)と
//! 設定プレビュー(egui・DIP)で描画APIは異なるが、形の定義はここに一本化する。
//! 片方だけ変更して見た目がずれるのを防ぐため、比率・しきい値は必ずこの定数を使うこと。

use crate::config::IndicatorShape;

/// キャレット上の丸をどれだけ持ち上げるか(半径比)。
const TOP_OFFSET: f32 = 0.55;
/// しずく下側の丸の半径比と、キャレット下端からの離し具合。
const LOWER_RATIO: f32 = 0.62;
const LOWER_OFFSET: f32 = 0.7;
/// バッジの角丸半径(辺長比)とキャレットとの隙間。
pub const BADGE_CORNER_RATIO: f32 = 0.18;
pub const BADGE_GAP: f32 = 2.0;
/// バッジ背景がこの明度より明るければ黒文字にする。
const LUMA_THRESHOLD: f32 = 135.0;

/// size(1〜5)から求めた寸法。オーバーレイとプレビューで基準スケールが異なる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// 丸・しずくの半径。
    pub blob_r: f32,
    /// バッジ(正方形)の一辺。
    pub badge_side: f32,
}

impl Metrics {
    /// オーバーレイ用(物理px)。
    pub fn overlay(size: i32) -> Metrics {
        let s = size.clamp(1, 5) as f32;
        Metrics { blob_r: 4.0 + s * 2.4, badge_side: 12.0 + s * 4.0 }
    }

    /// 設定プレビュー用(DIP。実物より小さめに描く)。
    pub fn preview(size: i32) -> Metrics {
        let s = size.clamp(1, 5) as f32;
        Metrics { blob_r: 3.5 + s * 1.9, badge_side: 10.0 + s * 3.2 }
    }

    /// オーバーレイウィンドウがキャレットの周囲に必要とする余白(px)。
    pub fn overlay_margin(&self, shape: IndicatorShape) -> i32 {
        if shape == IndicatorShape::Badge {
            (self.badge_side + 6.0).ceil() as i32
        } else {
            (self.blob_r * 2.0 + 4.0).ceil() as i32
        }
    }
}

/// 円ひとつ分の配置(中心x, 中心y, 半径)。
pub type Circle = (f32, f32, f32);

/// キャレット上に置く丸。TopCircle と Teardrop の上側で共用。
pub fn top_circle(r: f32, cx: f32, caret_top: f32) -> Circle {
    (cx, caret_top - r * TOP_OFFSET, r)
}

/// しずく下側の小さい丸。
pub fn lower_circle(r: f32, cx: f32, caret_bottom: f32) -> Circle {
    let r2 = r * LOWER_RATIO;
    (cx, caret_bottom + r2 * LOWER_OFFSET, r2)
}

/// バッジ矩形の左上座標(辺長は Metrics::badge_side)。
pub fn badge_origin(side: f32, cx: f32, caret_top: f32) -> (f32, f32) {
    (cx - side / 2.0, caret_top - BADGE_GAP - side)
}

/// バッジ文字のフォントサイズ。2文字以上なら小さくして収める。
pub fn badge_font_size(side: f32, char_count: usize) -> f32 {
    side * if char_count > 1 { 0.48 } else { 0.66 }
}

/// バッジ文字色の自動選択(背景の明度で白/黒)。
/// 純緑(luma 149.7)や水色(141.7)を黒文字にするしきい値。
pub fn auto_text_color(bg: (u8, u8, u8)) -> (u8, u8, u8) {
    let luma = 0.299 * bg.0 as f32 + 0.587 * bg.1 as f32 + 0.114 * bg.2 as f32;
    if luma > LUMA_THRESHOLD { (0, 0, 0) } else { (255, 255, 255) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_clamp_size() {
        assert_eq!(Metrics::overlay(0), Metrics::overlay(1));
        assert_eq!(Metrics::overlay(99), Metrics::overlay(5));
    }

    #[test]
    fn overlay_is_larger_than_preview() {
        for size in 1..=5 {
            assert!(Metrics::overlay(size).blob_r > Metrics::preview(size).blob_r);
        }
    }

    #[test]
    fn badge_margin_accounts_for_badge_size() {
        let m = Metrics::overlay(5);
        assert!(m.overlay_margin(IndicatorShape::Badge) > m.badge_side as i32);
    }

    #[test]
    fn auto_text_color_picks_black_on_bright() {
        assert_eq!(auto_text_color((0, 255, 0)), (0, 0, 0)); // 純緑 luma 149.7
        assert_eq!(auto_text_color((0, 0, 255)), (255, 255, 255)); // 純青 luma 29
    }

    /// リファクタ前(定数がoverlay.rs/settings.rsに直書きだった頃)と同じ値を返すことの固定テスト。
    /// 形を意図的に変える場合のみ、この期待値を更新すること。
    #[test]
    fn geometry_matches_pre_refactor_values() {
        let m = Metrics::overlay(3);
        assert_eq!(m.blob_r, 4.0 + 3.0 * 2.4); // 11.2
        assert_eq!(m.badge_side, 12.0 + 3.0 * 4.0); // 24.0
        assert_eq!(m.overlay_margin(IndicatorShape::Teardrop), 27); // ceil(11.2*2+4)
        assert_eq!(m.overlay_margin(IndicatorShape::Badge), 30); // ceil(24+6)

        let p = Metrics::preview(3);
        assert_eq!(p.blob_r, 3.5 + 3.0 * 1.9); // 9.2
        assert_eq!(p.badge_side, 10.0 + 3.0 * 3.2); // 19.6

        let (r, cx, top, bottom) = (10.0, 50.0, 100.0, 120.0);
        assert_eq!(top_circle(r, cx, top), (50.0, 100.0 - 10.0 * 0.55, 10.0));
        let (lx, ly, lr) = lower_circle(r, cx, bottom);
        assert_eq!((lx, lr), (50.0, 10.0 * 0.62));
        assert_eq!(ly, 120.0 + (10.0 * 0.62) * 0.7);

        assert_eq!(badge_origin(24.0, 50.0, 100.0), (50.0 - 12.0, 100.0 - 2.0 - 24.0));
        assert_eq!(badge_font_size(24.0, 1), 24.0 * 0.66);
        assert_eq!(badge_font_size(24.0, 2), 24.0 * 0.48);
        assert_eq!(BADGE_CORNER_RATIO, 0.18);
    }

    #[test]
    fn teardrop_lower_circle_is_smaller_and_below() {
        let (_, top_y, top_r) = top_circle(10.0, 0.0, 100.0);
        let (_, low_y, low_r) = lower_circle(10.0, 0.0, 120.0);
        assert!(low_r < top_r);
        assert!(low_y > top_y);
    }
}
