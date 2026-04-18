use crate::daemon::{AtMentioned, DaemonState, JsonRpcNotification};
use crate::{log_info, log_warn};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const SUB: &str = "daemon";

/// Wire payload sent from `zcc send` to the per-worktree daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcPayload {
    AtMentioned {
        file_path: String,
        line_start: u32,
        line_end: u32,
    },
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IpcReply {
    Ok { clients: usize },
    Err { message: String },
}

pub async fn serve(socket: PathBuf, state: Arc<DaemonState>) -> Result<()> {
    let _ = tokio::fs::remove_file(&socket).await;
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("bind unix socket {}", socket.display()))?;
    log_info!(SUB, "ipc listening path={}", socket.display());
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(v) => v,
            Err(err) => {
                log_warn!(SUB, "ipc accept failed: {err}");
                continue;
            }
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle(stream, state).await {
                log_warn!(SUB, "ipc client ended with error: {err}");
            }
        });
    }
}

async fn handle(stream: tokio::net::UnixStream, state: Arc<DaemonState>) -> Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut lines = BufReader::new(rd).lines();
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<IpcPayload>(&line) {
            Ok(IpcPayload::AtMentioned { file_path, line_start, line_end }) => {
                // IPC uses 1-indexed row numbers (matching $ZED_ROW). The Claude /ide
                // protocol uses 0-indexed lines (per coder/claudecode.nvim reference),
                // so convert here at the wire boundary.
                let wire_start = line_start.saturating_sub(1);
                let wire_end = line_end.saturating_sub(1);
                let notif = JsonRpcNotification::new(
                    "at_mentioned",
                    AtMentioned {
                        file_path: file_path.clone(),
                        line_start: wire_start,
                        line_end: wire_end,
                    },
                );
                log_info!(SUB, "at_mentioned file={file_path} start={wire_start} end={wire_end} (from 1-indexed {line_start}..{line_end})");
                match serde_json::to_string(&notif) {
                    Ok(text) => {
                        let clients = state.tx.receiver_count();
                        let _ = state.tx.send(text);
                        IpcReply::Ok { clients }
                    }
                    Err(err) => IpcReply::Err { message: format!("serialize: {err}") },
                }
            }
            Ok(IpcPayload::Ping) => IpcReply::Ok { clients: state.tx.receiver_count() },
            Err(err) => IpcReply::Err { message: format!("parse: {err}") },
        };
        let bytes = serde_json::to_vec(&reply)?;
        wr.write_all(&bytes).await?;
        wr.write_all(b"\n").await?;
    }
    Ok(())
}
