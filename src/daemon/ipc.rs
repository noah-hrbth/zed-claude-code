use crate::daemon::{AtMentioned, DaemonState, JsonRpcNotification};
use crate::{log_info, log_warn};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
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

/// Convert an inbound IPC payload to the JSON-RPC notification text that gets
/// broadcast to ws clients. Returns `None` for payloads that have no broadcast
/// shape (e.g. `Ping`). Used both by the live IPC handler and by the
/// queue-replay path at daemon startup.
pub fn payload_to_broadcast(payload: IpcPayload) -> Result<Option<String>> {
    match payload {
        IpcPayload::AtMentioned {
            file_path,
            line_start,
            line_end,
        } => {
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
            let text = serde_json::to_string(&notif)
                .map_err(|err| anyhow!("serialize at_mentioned: {err}"))?;
            Ok(Some(text))
        }
        IpcPayload::Ping => Ok(None),
    }
}

pub async fn serve(listener: UnixListener, state: Arc<DaemonState>) -> Result<()> {
    log_info!(SUB, "ipc accepting");
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(v) => v,
            Err(err) => {
                log_warn!(SUB, "ipc accept failed: {err}");
                continue;
            }
        };
        // same-user-only: reject any peer whose uid != ours. zcc never drops
        // privileges, so the peer's effective uid == our real uid; compare sound
        let our_uid = crate::util::current_uid();
        match stream.peer_cred() {
            Ok(cred) if cred.uid() == our_uid => {}
            Ok(cred) => {
                log_warn!(SUB, "ipc rejecting peer uid={} (not {our_uid})", cred.uid());
                continue;
            }
            Err(err) => {
                log_warn!(SUB, "ipc peer_cred failed, rejecting: {err}");
                continue;
            }
        }
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
            Ok(payload) => match payload_to_broadcast(payload) {
                Ok(Some(text)) => {
                    let clients = enqueue_and_maybe_drain(&state, text).await;
                    IpcReply::Ok { clients }
                }
                Ok(None) => IpcReply::Ok {
                    clients: state.tx.receiver_count(),
                },
                Err(err) => IpcReply::Err {
                    message: format!("serialize: {err}"),
                },
            },
            Err(err) => IpcReply::Err {
                message: format!("parse: {err}"),
            },
        };
        let bytes = serde_json::to_vec(&reply)?;
        wr.write_all(&bytes).await?;
        wr.write_all(b"\n").await?;
    }
    Ok(())
}

/// Push the broadcast text into the pending buffer (capped, oldest-evicting).
/// If any ws subscriber is connected, drain pending into the broadcast channel
/// in order so live messages always carry any backlog. Returns the receiver
/// count observed at decision time (used for the `clients` field of IpcReply,
/// which `zcc send` surfaces as the "no Claude attached" warning).
pub async fn enqueue_and_maybe_drain(state: &Arc<DaemonState>, text: String) -> usize {
    let mut pending = state.pending.lock().await;
    pending.push_back(text);
    while pending.len() > crate::daemon::PENDING_CAP {
        pending.pop_front();
    }
    let clients = state.tx.receiver_count();
    if clients > 0 {
        for msg in pending.drain(..) {
            let _ = state.tx.send(msg);
        }
    }
    clients
}
