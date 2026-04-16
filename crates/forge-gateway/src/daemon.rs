//! Daemon 进程管理：PID 文件、单例控制、健康检查。

use std::path::{Path, PathBuf};

/// Daemon PID 文件的默认路径。
pub fn pid_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".codeforge").join("daemon.pid")
}

/// 从 PID 文件解析进程 ID。
pub fn parse_pid_file(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// 写入 PID 文件。
pub fn write_pid_file(path: &Path, pid: u32) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", pid))?;
    Ok(())
}

/// 删除 PID 文件。
pub fn remove_pid_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// 检查 PID 对应的进程是否存活。
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_process_alive(_pid: u32) -> bool {
    false
}

/// 检查 Daemon 是否正在运行（PID 文件存在 + 进程存活）。
pub fn is_running(pid_file: &Path) -> Option<u32> {
    let pid = parse_pid_file(pid_file)?;
    if is_process_alive(pid) {
        Some(pid)
    } else {
        // Stale PID file, clean up
        remove_pid_file(pid_file);
        None
    }
}

/// Daemon 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonStatus {
    Running { pid: u32 },
    Stopped,
}

/// 获取 Daemon 状态。
pub fn status(pid_file: &Path) -> DaemonStatus {
    match is_running(pid_file) {
        Some(pid) => DaemonStatus::Running { pid },
        None => DaemonStatus::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_daemon_pid_file_path() {
        let path = pid_path();
        assert!(path.to_str().unwrap().contains(".codeforge"));
        assert!(path.to_str().unwrap().contains("daemon.pid"));
    }

    #[test]
    fn test_daemon_parse_pid_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("daemon.pid");
        std::fs::write(&path, "12345\n").unwrap();

        let pid = parse_pid_file(&path);
        assert_eq!(pid, Some(12345));
    }

    #[test]
    fn test_daemon_is_running_check() {
        let tmp = TempDir::new().unwrap();
        let pid_file = tmp.path().join("daemon.pid");

        // No PID file → not running
        assert_eq!(status(&pid_file), DaemonStatus::Stopped);

        // Stale PID (process doesn't exist)
        std::fs::write(&pid_file, "999999\n").unwrap();
        assert_eq!(status(&pid_file), DaemonStatus::Stopped);
        // Stale file should be cleaned up
        assert!(!pid_file.exists());

        // Current process PID → running
        let current_pid = std::process::id();
        write_pid_file(&pid_file, current_pid).unwrap();
        assert_eq!(
            status(&pid_file),
            DaemonStatus::Running { pid: current_pid }
        );
    }
}
