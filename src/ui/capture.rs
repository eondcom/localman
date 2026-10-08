//! 소개 영상용 화면 캡처 모드 — `LOCALMAN_CAPTURE=<steps.json>` 으로 켠다.
//!
//! 앱이 단계 목록대로 탭·검색·메뉴를 바꾸고 자기 창을 PNG 로 찍은 뒤 끝난다.
//! 사용자의 마우스·화면을 쓰지 않는다. 데모 데이터는 HOME 을 바꿔 띄워 따로 둔다.
//!
//! steps.json: `{"out": "폴더", "steps": [{"tab":"projects"}, {"wait":600}, {"shot":"projects"}, …]}`
//! 한 단계에 동작 하나(+ 선택 "shot"). 동작: tab · search · menu · deploy · close_deploy · site_tool ·
//! lang · theme · scale · scroll · db_tool · wait · shot

use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Plan {
    pub out: PathBuf,
    pub steps: Vec<Value>,
}

/// 캡처 모드면 단계 목록을 읽는다
pub fn plan() -> Option<Plan> {
    let path = std::env::var("LOCALMAN_CAPTURE").ok()?;
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    let out = PathBuf::from(v.get("out")?.as_str()?);
    let _ = std::fs::create_dir_all(&out);
    Some(Plan { out, steps: v.get("steps")?.as_array()?.clone() })
}

/// 데모 모드 (실제 MAMP 등 남의 자료를 화면에 띄우지 않는다)
pub fn demo() -> bool {
    std::env::var("LOCALMAN_CAPTURE").is_ok()
}

/// RGBA → PNG
pub fn save_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}
