use forge_mcp::{ServerConfig, ServerManager, ServerStatus};

fn sleep_config(name: &str, secs: u32) -> ServerConfig {
    ServerConfig {
        name: name.to_string(),
        command: "sleep".to_string(),
        args: vec![secs.to_string()],
        env: Default::default(),
    }
}

fn cat_config(name: &str) -> ServerConfig {
    // `cat` with no args reads stdin forever — stays alive until killed
    ServerConfig {
        name: name.to_string(),
        command: "cat".to_string(),
        args: vec![],
        env: Default::default(),
    }
}

#[tokio::test]
async fn test_mcp_server_start() {
    let mgr = ServerManager::new();
    let pid = mgr.start(cat_config("echo")).await.unwrap();
    assert!(pid > 0);
    assert_eq!(mgr.status("echo"), Some(ServerStatus::Running));

    // Cleanup
    mgr.stop("echo").await.unwrap();
}

#[tokio::test]
async fn test_mcp_server_stop() {
    let mgr = ServerManager::new();
    mgr.start(cat_config("srv")).await.unwrap();
    assert_eq!(mgr.status("srv"), Some(ServerStatus::Running));

    mgr.stop("srv").await.unwrap();
    assert_eq!(mgr.status("srv"), Some(ServerStatus::Stopped));
    assert_eq!(mgr.pid("srv"), None);
}

#[tokio::test]
async fn test_mcp_server_stop_force() {
    // `cat` ignores nothing special, SIGTERM should work.
    // This test verifies the stop flow completes within timeout.
    let mgr = ServerManager::new();
    mgr.start(cat_config("force")).await.unwrap();

    let start = std::time::Instant::now();
    mgr.stop("force").await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(mgr.status("force"), Some(ServerStatus::Stopped));
    // Should complete well within 3 seconds
    assert!(elapsed.as_secs() < 3);
}

#[tokio::test]
async fn test_mcp_server_auto_restart() {
    let mgr = ServerManager::new();
    // Start a short-lived process
    let pid1 = mgr.start(sleep_config("restartable", 100)).await.unwrap();

    // Kill it
    #[cfg(unix)]
    unsafe {
        libc::kill(pid1 as i32, libc::SIGKILL);
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Restart
    let restarted = mgr.restart("restartable").await.unwrap();
    assert!(restarted);

    let pid2 = mgr.pid("restartable");
    assert!(pid2.is_some());
    // New PID (may or may not differ, but status should be Running)
    assert_eq!(mgr.status("restartable"), Some(ServerStatus::Running));

    // Cleanup
    mgr.stop("restartable").await.unwrap();
}

#[tokio::test]
async fn test_mcp_server_restart_exhausted() {
    let mgr = ServerManager::new();
    mgr.start(cat_config("exhaust")).await.unwrap();

    // Exhaust 3 restarts
    for _ in 0..3 {
        mgr.stop("exhaust").await.unwrap();
        let _ = mgr.restart("exhaust").await;
    }

    // 4th restart should fail
    mgr.stop("exhaust").await.unwrap();
    let result = mgr.restart("exhaust").await.unwrap();
    assert!(!result, "should not restart after exhausting limit");
    assert_eq!(mgr.status("exhaust"), Some(ServerStatus::Failed));
}

#[tokio::test]
async fn test_mcp_multiple_servers() {
    let mgr = ServerManager::new();
    let pid_a = mgr.start(cat_config("server-a")).await.unwrap();
    let pid_b = mgr.start(cat_config("server-b")).await.unwrap();

    assert!(pid_a > 0);
    assert!(pid_b > 0);
    assert_eq!(mgr.status("server-a"), Some(ServerStatus::Running));
    assert_eq!(mgr.status("server-b"), Some(ServerStatus::Running));

    let names = mgr.list();
    assert!(names.contains(&"server-a".to_string()));
    assert!(names.contains(&"server-b".to_string()));

    mgr.stop("server-a").await.unwrap();
    mgr.stop("server-b").await.unwrap();
}

#[tokio::test]
async fn test_mcp_server_isolation() {
    let mgr = ServerManager::new();
    mgr.start(cat_config("iso-a")).await.unwrap();
    mgr.start(cat_config("iso-b")).await.unwrap();

    // Kill server-a
    mgr.stop("iso-a").await.unwrap();
    assert_eq!(mgr.status("iso-a"), Some(ServerStatus::Stopped));

    // server-b should still be running
    assert_eq!(mgr.status("iso-b"), Some(ServerStatus::Running));

    mgr.stop("iso-b").await.unwrap();
}
