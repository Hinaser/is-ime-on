//! %APPDATA%\IsImeOn\config.json の読み書き。JSONのキーは PascalCase。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 検出する入力モードの分類。
/// `key()` の文字列が config.json のキーになるため、変更すると既存の設定が読めなくなる。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ImeMode {
    Off,
    Other,
    HalfAlnum,
    HalfKana,
    WideAlnum,
    Hiragana,
    WideKana,
}

impl ImeMode {
    /// 設定UIでの表示順。「その他」はフォールバックなので最後。
    pub const ALL: [ImeMode; 7] = [
        ImeMode::Off,
        ImeMode::HalfAlnum,
        ImeMode::HalfKana,
        ImeMode::WideAlnum,
        ImeMode::Hiragana,
        ImeMode::WideKana,
        ImeMode::Other,
    ];

    /// 配列インデックス用の連番(0..7)。
    pub fn index(self) -> usize {
        match self {
            ImeMode::Off => 0,
            ImeMode::Other => 1,
            ImeMode::HalfAlnum => 2,
            ImeMode::HalfKana => 3,
            ImeMode::WideAlnum => 4,
            ImeMode::Hiragana => 5,
            ImeMode::WideKana => 6,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ImeMode::Off => "Off",
            ImeMode::Other => "Other",
            ImeMode::HalfAlnum => "HalfAlnum",
            ImeMode::HalfKana => "HalfKana",
            ImeMode::WideAlnum => "WideAlnum",
            ImeMode::Hiragana => "Hiragana",
            ImeMode::WideKana => "WideKana",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ImeMode::Off => "IMEオフ",
            ImeMode::Other => "その他",
            ImeMode::HalfAlnum => "半角英数",
            ImeMode::HalfKana => "半角カタカナ",
            ImeMode::WideAlnum => "全角英数",
            ImeMode::Hiragana => "ひらがな",
            ImeMode::WideKana => "全角カタカナ",
        }
    }
}

/// インジケーターの形状。config には小文字の文字列で保存する。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IndicatorShape {
    Teardrop,
    TopCircle,
    Badge,
}

impl IndicatorShape {
    pub fn parse(s: &str) -> IndicatorShape {
        match s.to_ascii_lowercase().as_str() {
            "topcircle" => IndicatorShape::TopCircle,
            "badge" => IndicatorShape::Badge,
            _ => IndicatorShape::Teardrop,
        }
    }

    pub fn to_config_str(self) -> &'static str {
        match self {
            IndicatorShape::Teardrop => "teardrop",
            IndicatorShape::TopCircle => "topcircle",
            IndicatorShape::Badge => "badge",
        }
    }
}

/// "#RRGGBB" → (r, g, b)。
pub fn parse_rgb_hex(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn to_rgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// モードごとの色・サイズ(1〜5)・表示有無・バッジ文字。
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct ModeSetting {
    pub color: String,
    pub size: i32,
    /// false のモードでは何も描かない(素のキャレットのまま)。
    pub visible: bool,
    /// 形状「文字バッジ」で表示する文字(1〜2文字)。
    pub label: String,
    /// バッジ文字の色。"" = 自動(背景の明度で白/黒)。
    pub label_color: String,
}

impl Default for ModeSetting {
    fn default() -> Self {
        ModeSetting {
            color: "#000000".into(),
            size: 3,
            visible: true,
            label: String::new(),
            label_color: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Preset {
    pub name: String,
    pub values: BTreeMap<String, ModeSetting>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase", default)]
pub struct AppConfig {
    /// 設定ファイルの形式バージョン。互換性のない変更をしたときに上げ、load() で移行する。
    pub config_version: i32,
    pub modes: BTreeMap<String, ModeSetting>,
    pub poll_interval_ms: i32,
    /// "teardrop" | "topcircle" | "badge"
    pub shape: String,
    pub presets: Vec<Preset>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            config_version: 1,
            modes: default_modes(),
            poll_interval_ms: 100,
            shape: "badge".into(),
            presets: Vec::new(),
        }
    }
}

/// 既定: IMEオフは非表示(素のキャレット)。
pub fn default_modes() -> BTreeMap<String, ModeSetting> {
    fn m(color: &str, size: i32, visible: bool, label: &str) -> ModeSetting {
        ModeSetting {
            color: color.into(),
            size,
            visible,
            label: label.into(),
            label_color: String::new(),
        }
    }
    let mut map = BTreeMap::new();
    map.insert("Off".into(), m("#000000", 1, false, "A"));
    map.insert("Other".into(), m("#808080", 3, true, "?"));
    map.insert("HalfAlnum".into(), m("#FF0000", 3, true, "A"));
    map.insert("HalfKana".into(), m("#FFC000", 3, true, "ｶ"));
    map.insert("WideAlnum".into(), m("#0000FF", 3, true, "Ａ"));
    map.insert("Hiragana".into(), m("#00FF00", 3, true, "あ"));
    map.insert("WideKana".into(), m("#00C0FF", 3, true, "カ"));
    map
}

impl AppConfig {
    pub fn shape_enum(&self) -> IndicatorShape {
        IndicatorShape::parse(&self.shape)
    }

    pub fn for_mode(&self, mode: ImeMode) -> ModeSetting {
        self.modes
            .get(mode.key())
            .cloned()
            .unwrap_or_else(|| default_modes().remove(mode.key()).unwrap())
    }

    /// 設定を置くディレクトリ(%APPDATA%\IsImeOn)。
    pub fn dir_path() -> PathBuf {
        let appdata = std::env::var_os("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("IsImeOn")
    }

    pub fn file_path() -> PathBuf {
        Self::dir_path().join("config.json")
    }

    pub fn load() -> AppConfig {
        let path = Self::file_path();
        let mut cfg: AppConfig = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // 欠けているモードは既定値で補完
        for (key, def) in default_modes() {
            cfg.modes.entry(key).or_insert(def);
        }
        cfg
    }

    pub fn save(&self) {
        let path = Self::file_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rgb_hex_accepts_valid() {
        assert_eq!(parse_rgb_hex("#FF0000"), Some((0xFF, 0, 0)));
        assert_eq!(parse_rgb_hex("00C0FF"), Some((0, 0xC0, 0xFF)));
        assert_eq!(parse_rgb_hex("  #00ff00  "), Some((0, 0xFF, 0)));
    }

    #[test]
    fn parse_rgb_hex_rejects_invalid() {
        assert_eq!(parse_rgb_hex(""), None);
        assert_eq!(parse_rgb_hex("#FFF"), None);
        assert_eq!(parse_rgb_hex("0xFF0000"), None);
        assert_eq!(parse_rgb_hex("red"), None);
    }

    #[test]
    fn shape_round_trips() {
        for shape in [
            IndicatorShape::Teardrop,
            IndicatorShape::TopCircle,
            IndicatorShape::Badge,
        ] {
            assert_eq!(IndicatorShape::parse(shape.to_config_str()), shape);
        }
        assert_eq!(IndicatorShape::parse("unknown"), IndicatorShape::Teardrop);
    }

    #[test]
    fn default_modes_cover_all() {
        let defaults = default_modes();
        for mode in ImeMode::ALL {
            assert!(defaults.contains_key(mode.key()), "{:?}", mode);
        }
        assert!(!defaults["Off"].visible);
    }

    #[test]
    fn config_json_uses_pascal_case() {
        let json = serde_json::to_string(&AppConfig::default()).unwrap();
        assert!(json.contains("\"ConfigVersion\""));
        assert!(json.contains("\"PollIntervalMs\""));
        assert!(json.contains("\"LabelColor\""));
    }
}
