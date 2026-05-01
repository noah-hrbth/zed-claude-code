use crate::daemon::ipc::{IpcPayload, IpcReply};
use crate::{log_debug, log_info, log_warn};
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

const SUB: &str = "send";

pub fn run(worktree: PathBuf, file: PathBuf, row: u32, selection: String) -> Result<()> {
    let worktree = worktree
        .canonicalize()
        .with_context(|| format!("canonicalize {}", worktree.display()))?;
    let sock = crate::util::ipc_socket_path(&worktree)?;

    let range = crate::selection::derive_from_path(&file, &selection, row).unwrap_or(
        crate::selection::LineRange {
            start: row,
            end: row,
        },
    );
    log_info!(
        SUB,
        "selection file={} row={row} start={} end={} selection_len={}",
        file.display(),
        range.start,
        range.end,
        selection.len()
    );

    let payload = IpcPayload::AtMentioned {
        file_path: file.to_string_lossy().into_owned(),
        line_start: range.start,
        line_end: range.end,
    };

    // Warm path: daemon already up, deliver synchronously and exit.
    if let Ok(stream) = UnixStream::connect(&sock) {
        send_payload(stream, &payload)?;
        log_info!(
            SUB,
            "forwarded selection to existing daemon sock={}",
            sock.display()
        );
        return Ok(());
    }

    // Cold path: queue the payload to disk and spawn a daemon. The daemon will
    // drain the queue dir at startup and broadcast everything once a ws client
    // is subscribed. We exit immediately so the Zed task tab disappears
    // without waiting for the daemon to finish coming up.
    enqueue(&worktree, &payload)?;
    log_info!(
        SUB,
        "no daemon running; queued payload and spawning for sock={}",
        sock.display()
    );
    crate::fork::spawn_daemon(&worktree)?;
    Ok(())
}

/// Persist the payload to `queue_dir(worktree)/{uuid}.json` so the freshly
/// spawned daemon can broadcast it once it's up. One file per message keeps
/// concurrent writers from corrupting each other.
fn enqueue(worktree: &std::path::Path, payload: &IpcPayload) -> Result<()> {
    let dir = crate::util::queue_dir(worktree)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create queue dir {}", dir.display()))?;
    let path = dir.join(format!("{}.json", Uuid::new_v4()));
    let bytes = serde_json::to_vec(payload).context("serialize queue payload")?;
    std::fs::write(&path, &bytes)
        .with_context(|| format!("write queue file {}", path.display()))?;
    Ok(())
}

fn send_payload(mut stream: UnixStream, payload: &IpcPayload) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let bytes = serde_json::to_vec(payload)?;
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while buf.len() < 4096 {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(_) => break,
        }
    }
    match serde_json::from_slice::<IpcReply>(&buf) {
        Ok(IpcReply::Ok { clients }) => {
            if clients == 0 {
                log_warn!(SUB, "daemon accepted message but no Claude client is attached (run /ide inside Claude Code)");
            }
        }
        Ok(IpcReply::Err { message }) => anyhow::bail!("daemon rejected: {message}"),
        Err(err) => log_debug!(SUB, "reply parse failed (ignored): {err}"),
    }
    Ok(())
}
