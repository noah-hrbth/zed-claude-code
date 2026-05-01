use crate::{log_error, log_info, log_warn};
use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, UnixListener};
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

/// Maximum number of broadcasts buffered while no ws subscriber is connected.
/// On overflow we evict the oldest. New ws clients drain this buffer when they
/// subscribe; a live ipc broadcast also drains it.
pub const PENDING_CAP: usize = 32;

pub struct DaemonState {
    pub auth_token: String,
    /// Broadcast to all connected WS clients.
    pub tx: broadcast::Sender<String>,
    /// Count of currently connected clients; used for idle-shutdown.
    pub connected: Arc<Mutex<usize>>,
    /// Broadcasts captured while no ws subscriber was connected, plus any
    /// payloads drained from the on-disk queue at startup. Drained in order on
    /// the next ws connect or live ipc broadcast.
    pub pending: Arc<Mutex<VecDeque<String>>>,
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

    // Bind the IPC socket BEFORE writing the lockfile so a concurrent loser
    // daemon (from a fast double-send) bails here without ever publishing a
    // lockfile or running cleanup. Cleanup unconditionally removes the
    // per-worktree socket path, so a loser running cleanup would clobber the
    // winner's socket.
    //
    // Probe-before-unlink: if a live daemon is already bound at this path, a
    // bare `connect` succeeds. Bail before unlinking — otherwise we'd delete
    // the active socket file out from under the winner. Only unlink when the
    // path is stale (connect refused / no such file).
    let ipc_path = crate::util::ipc_socket_path(&worktree)?;
    if std::os::unix::net::UnixStream::connect(&ipc_path).is_ok() {
        anyhow::bail!("another daemon already bound {}", ipc_path.display());
    }
    let _ = tokio::fs::remove_file(&ipc_path).await;
    let ipc_listener = UnixListener::bind(&ipc_path)
        .with_context(|| format!("bind unix socket {}", ipc_path.display()))?;

    let auth_token = Uuid::new_v4().to_string();
    let (tx, _) = broadcast::channel::<String>(BROADCAST_CAP);
    let state = Arc::new(DaemonState {
        auth_token: auth_token.clone(),
        tx: tx.clone(),
        connected: Arc::new(Mutex::new(0)),
        pending: Arc::new(Mutex::new(VecDeque::with_capacity(PENDING_CAP))),
    });

    drain_queue_dir(&worktree, &state).await;

    let lockfile_path = lockfile::write(port, &worktree, &auth_token)?;
    log_info!(
        SUB,
        "daemon listening port={} worktree={} lockfile={}",
        port,
        worktree.display(),
        lockfile_path.display()
    );

    let ipc_state = Arc::clone(&state);
    let ipc_path_for_cleanup = ipc_path.clone();
    let ipc_handle = tokio::spawn(async move {
        if let Err(err) = ipc::serve(ipc_listener, ipc_state).await {
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

/// Read every queued payload file in `queue_dir(worktree)`, parse it, push the
/// resulting broadcast text into `state.pending`, and delete the file. Files are
/// processed in mtime order so message ordering is preserved across crashes.
/// Failures on individual files are logged and skipped; the queue dir itself
/// not existing is normal (no cold-path send happened).
async fn drain_queue_dir(worktree: &std::path::Path, state: &Arc<DaemonState>) {
    let dir = match crate::util::queue_dir(worktree) {
        Ok(d) => d,
        Err(err) => {
            log_warn!(SUB, "queue dir path: {err}");
            return;
        }
    };
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                log_warn!(SUB, "queue dir read {}: {err}", dir.display());
            }
            return;
        }
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    while let Ok(Some(ent)) = entries.next_entry().await {
        let path = ent.path();
        let mtime = ent
            .metadata()
            .await
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        files.push((mtime, path));
    }
    files.sort_by_key(|(t, _)| *t);

    for (_, path) in files {
        let body = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(err) => {
                log_warn!(SUB, "queue read {}: {err}", path.display());
                continue;
            }
        };
        let payload: ipc::IpcPayload = match serde_json::from_slice(&body) {
            Ok(p) => p,
            Err(err) => {
                log_warn!(SUB, "queue parse {}: {err}", path.display());
                let _ = tokio::fs::remove_file(&path).await;
                continue;
            }
        };
        match ipc::payload_to_broadcast(payload) {
            Ok(Some(text)) => {
                let mut pending = state.pending.lock().await;
                pending.push_back(text);
                while pending.len() > PENDING_CAP {
                    pending.pop_front();
                }
            }
            Ok(None) => {}
            Err(err) => log_warn!(SUB, "queue convert {}: {err}", path.display()),
        }
        let _ = tokio::fs::remove_file(&path).await;
    }
    log_info!(
        SUB,
        "queue drain complete pending_len={}",
        state.pending.lock().await.len()
    );
}
