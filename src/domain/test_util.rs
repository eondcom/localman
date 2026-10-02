//! 테스트 공용 도우미.

use super::project::{ProjectType, VhostProject};
use std::fs;
use std::path::PathBuf;

/// 테스트용 임시 디렉토리 (tempfile 의존성 없이)
pub struct TmpDir(pub PathBuf);

impl TmpDir {
    pub fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!("localman-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
    pub fn path(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
    /// 하위 경로에 파일을 만든다 (상위 디렉토리 자동 생성)
    pub fn write(&self, rel: &str, body: &str) {
        let full = self.0.join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, body).unwrap();
    }
    pub fn mkdir(&self, rel: &str) {
        fs::create_dir_all(self.0.join(rel)).unwrap();
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn project(t: ProjectType, path: &str, app_dir: &str, port: u16) -> VhostProject {
    VhostProject {
        id: "demo".into(),
        name: "Demo".into(),
        path: path.into(),
        domain: "demo.localhost".into(),
        project_type: t,
        port,
        start_command: String::new(),
        app_dir: app_dir.into(),
    }
}
