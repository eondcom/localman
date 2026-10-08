//! 화면 다국어 (한국어 · English · 日本語).
//!
//! 한국어 원문을 그대로 키로 쓴다 — 코드를 읽을 때 무슨 문장인지 바로 보이고, 번역이 빠지면 한국어로 나온다.
//! - 고정 문장: `tr("프로젝트")`
//! - 값이 들어가는 문장: `trf("파일 {0}개를 올렸습니다", &[&n])` — 언어마다 어순이 달라 위치 번호를 쓴다
//!
//! 번역 표는 파일 묶음마다 나눠 둔다(t_*.rs). 소스의 tr()/trf() 원문이 표에 모두 있는지는
//! 테스트(`every_source_string_is_translated`)가 확인한다.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

mod t_db;
mod t_domain;
mod t_projects;
mod t_ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ko,
    En,
    Ja,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::En,
        2 => Lang::Ja,
        _ => Lang::Ko,
    }
}

/// "ko-KR", "en_US.UTF-8", "ja" 같은 로캘 문자열에서 언어를 고른다. 모르는 언어는 영어.
pub fn from_locale(locale: &str) -> Lang {
    let l = locale.trim().to_lowercase();
    if l.starts_with("ko") {
        Lang::Ko
    } else if l.starts_with("ja") {
        Lang::Ja
    } else {
        Lang::En
    }
}

fn tables() -> &'static [&'static [(&'static str, &'static str, &'static str)]] {
    &[t_ui::T, t_projects::T, t_db::T, t_domain::T]
}

fn index() -> &'static HashMap<&'static str, (&'static str, &'static str)> {
    static MAP: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();
    MAP.get_or_init(|| tables().iter().flat_map(|t| t.iter()).map(|(ko, en, ja)| (*ko, (*en, *ja))).collect())
}

/// 고정 문장 번역. 표에 없으면 한국어 원문을 그대로 돌려준다.
pub fn tr(ko: &'static str) -> &'static str {
    match lang() {
        Lang::Ko => ko,
        l => match index().get(ko) {
            Some((en, ja)) => {
                let s = if l == Lang::En { *en } else { *ja };
                if s.is_empty() { ko } else { s }
            }
            None => ko,
        },
    }
}

/// 값이 들어가는 문장 번역. 원문·번역의 {0}, {1} … 자리에 args 를 넣는다.
pub fn trf(ko: &'static str, args: &[&dyn Display]) -> String {
    let mut s = tr(ko).to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 소스에서 tr("…") / trf("…" 의 원문을 모은다 (문자열 안의 \" 는 고려하지 않는다 — 원문에 쓰지 않는다)
    fn source_keys() -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut stack = vec![std::path::PathBuf::from("src")];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !p.ends_with("i18n") {
                        stack.push(p);
                    }
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap();
                for pat in ["tr(", "trf("] {
                    let mut rest = src.as_str();
                    while let Some(i) = rest.find(pat) {
                        // 다른 이름(attr( 등)의 꼬리에 걸리지 않게 앞 글자를 본다
                        let before = rest[..i].chars().last();
                        rest = &rest[i + pat.len()..];
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        // 원문이 다음 줄에서 시작하는 호출(trf(\n    "…")도 잡는다
                        let trimmed = rest.trim_start();
                        let Some(lit) = trimmed.strip_prefix('"') else { continue };
                        if let Some(end) = lit.find('"') {
                            out.push((lit[..end].to_string(), p.display().to_string()));
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn every_source_string_is_translated() {
        let map = index();
        let missing: Vec<String> = source_keys()
            .into_iter()
            .filter(|(k, _)| match map.get(k.as_str()) {
                Some((en, ja)) => en.is_empty() || ja.is_empty(),
                None => true,
            })
            .map(|(k, f)| format!("{f}: {k}"))
            .collect();
        assert!(missing.is_empty(), "번역이 없는 문장 {}개:\n{}", missing.len(), missing.join("\n"));
    }

    #[test]
    fn placeholders_match_across_languages() {
        for t in tables() {
            for (ko, en, ja) in t.iter() {
                let count = |s: &str| (0..10).filter(|i| s.contains(&format!("{{{i}}}"))).count();
                assert_eq!(count(ko), count(en), "자리표시 개수 다름(en): {ko}");
                assert_eq!(count(ko), count(ja), "자리표시 개수 다름(ja): {ko}");
            }
        }
    }

    /// 같은 원문이 여러 표에 있어도 되지만, 번역은 같아야 한다 (어느 표가 이길지 모르므로)
    #[test]
    fn duplicate_keys_agree() {
        let mut seen: HashMap<&str, (&str, &str)> = HashMap::new();
        let mut bad = Vec::new();
        for (k, en, ja) in tables().iter().flat_map(|t| t.iter()) {
            if let Some(prev) = seen.insert(k, (en, ja)) {
                if prev != (*en, *ja) {
                    bad.push(format!("{k}: {prev:?} / {:?}", (en, ja)));
                }
            }
        }
        assert!(bad.is_empty(), "같은 원문의 번역이 표마다 다릅니다:\n{}", bad.join("\n"));
    }

    #[test]
    fn locale_detection() {
        assert_eq!(from_locale("ko-KR"), Lang::Ko);
        assert_eq!(from_locale("ja_JP.UTF-8"), Lang::Ja);
        assert_eq!(from_locale("en-US"), Lang::En);
        assert_eq!(from_locale("fr-FR"), Lang::En);
    }

    #[test]
    fn trf_fills_positions() {
        set_lang(Lang::Ko);
        assert_eq!(trf("{1} 다음 {0}", &[&"a", &2]), "2 다음 a");
    }
}
