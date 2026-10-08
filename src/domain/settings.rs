//! 앱 전역 설정 (settings.json). PC마다 다른 값이라 백업 묶음에는 담지 않는다.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
    /// OS 설정을 따른다
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LangMode {
    /// OS 언어를 따른다 (한국어·일본어가 아니면 영어)
    #[default]
    System,
    Ko,
    En,
    Ja,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// 모든 프로젝트를 https 로도 서빙할지 (로컬 인증기관 인증서)
    #[serde(default)]
    pub https: bool,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default)]
    pub lang: LangMode,
}

pub(crate) fn data_dir() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    let _ = fs::create_dir_all(&p);
    p
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    let data = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    super::write_atomic(&settings_path(), data.as_bytes())
}
