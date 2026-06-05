use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VhostProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub domain: String, // id.localhost
    pub port: u16,
}

fn data_path() -> PathBuf {
    let mut p = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("localman");
    fs::create_dir_all(&p).ok();
    p.push("projects.json");
    p
}

pub fn list_projects() -> Vec<VhostProject> {
    let path = data_path();
    let data = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn add_project(project: VhostProject) -> Result<(), String> {
    let mut list = list_projects();
    list.push(project.clone());
    save_projects(&list)?;
    write_vhost(&project)?;
    update_hosts(&project.domain, true)?;
    Ok(())
}

pub fn remove_project(id: &str) -> Result<(), String> {
    let mut list = list_projects();
    if let Some(p) = list.iter().find(|p| p.id == id).cloned() {
        update_hosts(&p.domain, false)?;
        remove_vhost(&p)?;
    }
    list.retain(|p| p.id != id);
    save_projects(&list)
}

fn save_projects(list: &[VhostProject]) -> Result<(), String> {
    let data = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(data_path(), data).map_err(|e| e.to_string())
}

fn write_vhost(p: &VhostProject) -> Result<(), String> {
    let conf = format!(
        "<VirtualHost *:80>\n    ServerName {}\n    DocumentRoot {}\n    <Directory {}>\n        AllowOverride All\n        Require all granted\n    </Directory>\n</VirtualHost>\n",
        p.domain, p.path, p.path
    );
    let conf_path = format!("/etc/apache2/sites-available/{}.conf", p.id);
    let enable_path = format!("/etc/apache2/sites-enabled/{}.conf", p.id);

    fs::write(&conf_path, &conf).map_err(|e| format!("vhost 파일 쓰기 실패: {e}"))?;
    std::process::Command::new("pkexec")
        .args(["ln", "-sf", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    std::process::Command::new("pkexec")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn remove_vhost(p: &VhostProject) -> Result<(), String> {
    let conf_path = format!("/etc/apache2/sites-available/{}.conf", p.id);
    let enable_path = format!("/etc/apache2/sites-enabled/{}.conf", p.id);
    std::process::Command::new("pkexec")
        .args(["rm", "-f", &conf_path, &enable_path])
        .output()
        .map_err(|e| e.to_string())?;
    std::process::Command::new("pkexec")
        .args(["systemctl", "reload", "apache2"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn update_hosts(domain: &str, add: bool) -> Result<(), String> {
    let hosts = fs::read_to_string("/etc/hosts").map_err(|e| e.to_string())?;
    let entry = format!("127.0.0.1\t{domain}");

    let new_hosts = if add {
        if hosts.contains(domain) {
            return Ok(());
        }
        format!("{hosts}\n{entry}\n")
    } else {
        hosts
            .lines()
            .filter(|l| !l.contains(domain))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };

    let tmp = "/tmp/localman_hosts";
    fs::write(tmp, new_hosts).map_err(|e| e.to_string())?;
    std::process::Command::new("pkexec")
        .args(["cp", tmp, "/etc/hosts"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(())
}
