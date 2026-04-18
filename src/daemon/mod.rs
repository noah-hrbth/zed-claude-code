use crate::{log_error, log_info, log_warn};
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

pub mod ipc;
pub mod jsonrpc;
pub mod lockfile;
pub mod protocol;
pub mod ws;

pub use jsonrpc::{AtMentioned, JsonRpcNotification};

const SUB: &str = "daemon";

/// Idle timeout: if no Claude client connects or reconnects for this long, shut down.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Broadcast channel buffer.
const BROADCAST_CAP: usize = 64;

pub struct DaemonState {
    pub auth_token: String,
    /// Broadcast to all connected WS clients.
    pub tx: broadcast::Sender<String>,
    /// Count of currently connected clients; used for idle-shutdown.
    pub connected: Arc<Mutex<usize>>,
}

pub async fn run(worktree: PathBuf) -> Result<()> {
    if !worktree.exists() {
        anyhow::bail!("worktree path does not exist: {}", worktree.display());
    }
    let worktree = worktree
        .canonicalize()
        .with_context(|| format!("canonicalize {}", worktree.display()))?;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind 127.0.0.1:0")?;
    let local: SocketAddr = listener.local_addr()?;
    let port = local.port();

    let auth_token = Uuid::new_v4().to_string();
    let (tx, _) = broadcast::channel::<String>(BROADCAST_CAP);
    let state = Arc::new(DaemonState {
        auth_token: auth_token.clone(),
        tx: tx.clone(),
        connected: Arc::new(Mutex::new(0)),
    });

    let lockfile_path = lockfile::write(port, &worktree, &auth_token)?;
    log_info!(
        SUB,
        "daemon listening port={} worktree={} lockfile={}",
        port,
        worktree.display(),
        lockfile_path.display()
    );

    let ipc_path = crate::util::ipc_socket_path(&worktree)?;
    let ipc_state = Arc::clone(&state);
    let ipc_path_for_cleanup = ipc_path.clone();
    let ipc_handle = tokio::spawn(async move {
        if let Err(err) = ipc::serve(ipc_path, ipc_state).await {
            log_error!(SUB, "ipc server exited with error: {err}");
        }
    });

    let ws_state = Arc::clone(&state);
    let ws_handle = tokio::spawn(async move {
        ws::serve(listener, ws_state).await;
    });

    let idle_state = Arc::clone(&state);
    let idle_handle = tokio::spawn(async move {
        idle_watchdog(idle_state).await;
    });

    tokio::select! {
        _ = shutdown_signal() => {
            log_info!(SUB, "shutdown signal received");
        }
        _ = idle_handle => {
            log_info!(SUB, "idle timeout reached");
        }
        res = ws_handle => {
            log_warn!(SUB, "ws server ended: {res:?}");
        }
        res = ipc_handle => {
            log_warn!(SUB, "ipc server ended: {res:?}");
        }
    }

    cleanup(&lockfile_path, &ipc_path_for_cleanup).await;
    Ok(())
}

async fn idle_watchdog(state: Arc<DaemonState>) {
    let mut last_connected = false;
    let mut idle_since = std::time::Instant::now();
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let connected = *state.connected.lock().await > 0;
        if connected {
            idle_since = std::time::Instant::now();
            last_connected = true;
        } else if last_connected {
            idle_since = std::time::Instant::now();
            last_connected = false;
        } else if idle_since.elapsed() >= IDLE_TIMEOUT {
            return;
        }
    }
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {},
        _ = int.recv() => {},
    }
}

async fn cleanup(lockfile_path: &std::path::Path, ipc_path: &std::path::Path) {
    if let Err(err) = tokio::fs::remove_file(lockfile_path).await {
        if err.kind() != std::io::ErrorKind::NotFound {
            log_warn!(SUB, "remove lockfile {}: {err}", lockfile_path.display());
        }
    }
    let _ = tokio::fs::remove_file(ipc_path).await;
}
