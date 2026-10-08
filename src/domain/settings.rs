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

/// pma.localhost 로 열 DB 관리 도구
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PmaTool {
    #[default]
    Adminer,
    PhpMyAdmin,
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
    #[serde(default)]
    pub pma_tool: PmaTool,
    /// 앱을 켤 때 꺼져 있는 Apache·DB 서버를 켠다
    #[serde(default)]
    pub autostart_services: bool,
    /// 로컬 HTTP API (127.0.0.1)
    #[serde(default)]
    pub api_enabled: bool,
    #[serde(default)]
    pub api_token: String,
    /// 화면 크기 (%) — macOS 기본 앱과 비슷한 밀도가 되도록 기본 90
    #[serde(default = "default_ui_scale")]
    pub ui_scale: u8,
}

fn default_ui_scale() -> u8 {
    90
}

impl Settings {
    /// 저장된 화면 크기 (설정 파일이 없어 Default 로 만든 0 은 기본값으로)
    pub fn scale_percent(&self) -> u8 {
        if self.ui_scale == 0 { default_ui_scale() } else { self.ui_scale }
    }
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
