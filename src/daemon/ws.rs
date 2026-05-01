use crate::daemon::DaemonState;
use crate::{log_debug, log_info, log_warn};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

const AUTH_HEADER: &str = "x-claude-code-ide-authorization";
const SUB: &str = "daemon";

pub async fn serve(listener: TcpListener, state: Arc<DaemonState>) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(err) => {
                log_warn!(SUB, "accept failed: {err}");
                continue;
            }
        };
        if !peer.ip().is_loopback() {
            log_warn!(SUB, "rejecting non-loopback peer {peer}");
            continue;
        }
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle(stream, peer, state).await {
                log_debug!(SUB, "ws connection ended peer={peer} err={err}");
            }
        });
    }
}

async fn handle(stream: TcpStream, peer: SocketAddr, state: Arc<DaemonState>) -> Result<()> {
    let expected = state.auth_token.clone();
    #[allow(clippy::result_large_err)]
    let auth_cb = move |req: &Request, response: Response| -> Result<Response, ErrorResponse> {
        let ok = req
            .headers()
            .get(AUTH_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s == expected)
            .unwrap_or(false);
        if ok {
            Ok(response)
        } else {
            let mut err = ErrorResponse::new(Some("unauthorized".into()));
            *err.status_mut() = StatusCode::UNAUTHORIZED;
            Err(err)
        }
    };

    let ws_stream = tokio_tungstenite::accept_hdr_async(stream, auth_cb).await?;
    log_info!(SUB, "claude client connected peer={peer}");

    {
        let mut n = state.connected.lock().await;
        *n += 1;
    }

    let (mut write, mut read) = ws_stream.split();
    let mut rx = state.tx.subscribe();

    // Replay any pending broadcasts (from on-disk queue replay or earlier
    // sends that arrived before any subscriber). Subscribe-then-drain order
    // matters: subscribing first ensures that any concurrent ipc broadcast
    // arriving while we drain still reaches us via `rx`. Re-broadcasting via
    // `tx.send` (rather than writing directly to this client) lets any
    // already-connected sibling clients also see the backlog.
    {
        let mut pending = state.pending.lock().await;
        for msg in pending.drain(..) {
            let _ = state.tx.send(msg);
        }
    }

    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    ping_interval.tick().await;

    let outcome: Result<()> = loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if let Err(err) = write.send(Message::Text(text.into())).await {
                            break Err(err.into());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log_warn!(SUB, "broadcast lagged n={n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break Ok(());
                    }
                }
            }
            _ = ping_interval.tick() => {
                if let Err(err) = write.send(Message::Ping(Vec::new().into())).await {
                    break Err(err.into());
                }
            }
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => {
                        break Ok(());
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = write.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        log_debug!(SUB, "ws recv text len={}", text.len());
                        if let Some(reply) = crate::daemon::protocol::handle(&text) {
                            log_debug!(SUB, "ws reply len={}", reply.len());
                            if let Err(err) = write.send(Message::Text(reply.into())).await {
                                break Err(err.into());
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => break Err(err.into()),
                }
            }
        }
    };

    {
        let mut n = state.connected.lock().await;
        *n = n.saturating_sub(1);
    }
    log_info!(SUB, "claude client disconnected peer={peer}");
    outcome
}
