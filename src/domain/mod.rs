//! OS와 무관한 핵심 로직. OS별 작업은 전부 `crate::platform`을 거친다.

pub mod apache;
pub mod database;
pub mod detect;
pub mod history;
pub mod integrations;
pub mod lan;
pub mod project;
pub mod server;
pub mod settings;
pub mod setup;
pub mod tls;
pub mod transfer;
#[cfg(test)]
mod test_util;

pub use database::{DbCredentials, DbEngine, list_databases, backup_database, restore_database, import_sql, create_database, drop_database, rename_database, list_users, create_user, drop_user, rename_user, change_user_password, grant_privileges, DbUser, load_db_connections, save_db_connection};
pub use detect::{auto_detect_start_command, auto_detect_next_command, detect_next_app_dir};
pub use integrations::{ensure_adminer_site, is_rhymix_project, rx_reset_admin_password};
pub use project::{VhostProject, ProjectType, list_projects, add_project, update_project, remove_project, auto_assign_port, join_dir};
pub use server::{ServerStatus, start_server, stop_server, server_status};
pub use setup::{setup_project, deps_ready};
