//! 설정 탭: 화면(테마), HTTPS(로컬 인증기관), 정보.

use iced::{
    widget::{column, Space},
    Element, Task,
};

use super::theme::{self, Icon, Kind, Tone, btn, chip, group, muted, result_line, section_label, segmented, setting_row, switch};
use crate::domain::apache::{renew_certs, set_https};
use crate::domain::settings::{LangMode, ThemeMode, data_dir, load_settings, save_settings};
use crate::i18n::{self, Lang, tr};
use crate::domain::tls::{ca_cert_path, ca_exists};
use crate::platform::{ca_trusted, open_url, system_prefers_dark};

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    SetTheme(ThemeMode),
    SetLang(LangMode),
    /// 시스템 테마를 따를 때 OS 설정이 바뀌었는지 주기적으로 본다
    CheckSystemTheme,
    SetHttps(bool),
    HttpsSet(Result<Vec<String>, String>),
    RenewCerts,
    CertsRenewed(Vec<String>),
    OpenDataDir,
    OpenSite,
}

pub struct SettingsState {
    theme: ThemeMode,
    lang: LangMode,
    https: bool,
    ca_trusted: bool,
    https_busy: bool,
    https_log: Vec<String>,
}

/// 저장된 언어 설정을 적용한다 (시스템이면 OS 언어)
pub fn apply_lang(mode: LangMode) {
    i18n::set_lang(match mode {
        LangMode::Ko => Lang::Ko,
        LangMode::En => Lang::En,
        LangMode::Ja => Lang::Ja,
        LangMode::System => i18n::from_locale(&crate::platform::system_language()),
    });
}

/// 저장된 테마 설정을 실제 다크/라이트로 바꿔 적용한다.
pub fn apply_theme(mode: ThemeMode) {
    theme::set_dark(match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => system_prefers_dark(),
    });
}

impl SettingsState {
    pub fn new() -> Self {
        let s = load_settings();
        apply_theme(s.theme);
        apply_lang(s.lang);
        Self {
            theme: s.theme,
            lang: s.lang,
            https: s.https,
            ca_trusted: ca_exists() && ca_trusted(&ca_cert_path()),
            https_busy: false,
            https_log: Vec::new(),
        }
    }

    pub fn https_on(&self) -> bool {
        self.https
    }

    fn refresh(&mut self) {
        self.https = load_settings().https;
        self.ca_trusted = ca_exists() && ca_trusted(&ca_cert_path());
    }

    pub fn update(&mut self, msg: SettingsMessage) -> Task<SettingsMessage> {
        match msg {
            SettingsMessage::SetTheme(mode) => {
                self.theme = mode;
                apply_theme(mode);
                let mut s = load_settings();
                s.theme = mode;
                let _ = save_settings(&s);
                Task::none()
            }
            SettingsMessage::SetLang(mode) => {
                self.lang = mode;
                apply_lang(mode);
                let mut s = load_settings();
                s.lang = mode;
                let _ = save_settings(&s);
                Task::none()
            }
            SettingsMessage::CheckSystemTheme => {
                if self.theme == ThemeMode::System {
                    apply_theme(ThemeMode::System);
                }
                Task::none()
            }
            SettingsMessage::SetHttps(on) => {
                self.https_busy = true;
                self.https_log.clear();
                Task::perform(
                    async move { tokio::task::spawn_blocking(move || set_https(on)).await.unwrap_or_else(|e| Err(e.to_string())) },
                    SettingsMessage::HttpsSet,
                )
            }
            SettingsMessage::HttpsSet(result) => {
                self.https_busy = false;
                match result {
                    Ok(log) => self.https_log = log,
                    Err(e) => self.https_log = vec![format!("✗ {e}")],
                }
                self.refresh();
                Task::none()
            }
            SettingsMessage::RenewCerts => Task::perform(
                async { tokio::task::spawn_blocking(renew_certs).await.unwrap_or_default() },
                SettingsMessage::CertsRenewed,
            ),
            SettingsMessage::CertsRenewed(log) => {
                if !log.is_empty() {
                    self.https_log = log;
                }
                Task::none()
            }
            SettingsMessage::OpenDataDir => {
                open_url(&data_dir().to_string_lossy());
                Task::none()
            }
            SettingsMessage::OpenSite => {
                open_url("https://eond.com");
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, SettingsMessage> {
        let screen = group(vec![
            setting_row(
                tr("테마"),
                None,
                segmented(
                    &[(ThemeMode::Dark, tr("다크")), (ThemeMode::Light, tr("라이트")), (ThemeMode::System, tr("시스템"))],
                    &self.theme,
                    SettingsMessage::SetTheme,
                ),
            ),
            // 언어 이름은 번역하지 않는다 — 모르는 언어 화면에서도 자기 언어를 찾을 수 있게 ("시스템"만 번역)
            setting_row(
                tr("언어"),
                None,
                segmented(
                    &[(LangMode::System, tr("시스템")), (LangMode::Ko, "한국어"), (LangMode::En, "English"), (LangMode::Ja, "日本語")],
                    &self.lang,
                    SettingsMessage::SetLang,
                ),
            ),
        ]);

        let (status, tone) = if !self.https {
            (tr("꺼짐"), Tone::Neutral)
        } else if self.ca_trusted {
            (tr("인증기관 신뢰됨"), Tone::Success)
        } else {
            (tr("신뢰 안 됨 · 브라우저 경고"), Tone::Warning)
        };
        let toggle = if self.https_busy {
            Element::from(muted(tr("처리 중…")))
        } else {
            switch(self.https).on_toggle(SettingsMessage::SetHttps).into()
        };
        let mut https_rows = vec![
            setting_row(
                tr("모든 프로젝트를 HTTPS로도 열기"),
                Some(tr("로컬 인증기관으로 https://도메인 인증서를 만들고 만료 30일 전에 자동 갱신합니다")),
                toggle,
            ),
            setting_row(tr("상태"), None, chip(status, tone)),
        ];
        if self.https && !self.ca_trusted {
            https_rows.push(setting_row(
                tr("신뢰 등록 다시 시도"),
                Some(tr("끄고 다시 켜면 인증기관을 OS에 다시 등록합니다")),
                Space::with_width(0).into(),
            ));
        }
        if !self.https_log.is_empty() {
            let lines = self.https_log.iter().fold(column![].spacing(4), |c, l| c.push(result_line(l)));
            https_rows.push(lines.into());
        }

        let info = group(vec![
            setting_row(tr("버전"), None, chip(concat!("v", env!("CARGO_PKG_VERSION")), Tone::Neutral)),
            setting_row(
                tr("데이터 폴더"),
                Some(tr("프로젝트 목록·DB 접속·인증서·이전 기록이 저장됩니다")),
                btn(tr("열기"), Some(Icon::FolderOpen), Kind::Flat).on_press(SettingsMessage::OpenDataDir).into(),
            ),
            setting_row(
                tr("만든 곳"),
                None,
                btn(tr("이온디 · eond.com"), Some(Icon::Globe), Kind::Ghost).on_press(SettingsMessage::OpenSite).into(),
            ),
        ]);

        column![
            theme::page_header(tr("설정"), tr("화면, HTTPS, 앱 정보"), None),
            Space::with_height(16),
            section_label(tr("화면")),
            screen,
            Space::with_height(14),
            section_label("HTTPS"),
            group(https_rows),
            Space::with_height(14),
            section_label(tr("정보")),
            info,
        ]
        .into()
    }
}

/// 사이드바 아래 요약에 쓰는 한 줄
pub fn https_summary(on: bool) -> (&'static str, Tone) {
    if on { (tr("HTTPS 켜짐"), Tone::Primary) } else { (tr("HTTPS 꺼짐"), Tone::Neutral) }
}
