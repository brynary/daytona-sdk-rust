//! Interactive PTY sessions over the toolbox WebSocket.
//!
//! Mirrors the TypeScript SDK's `PtyHandle` contract and Go's channel
//! shape: outbound input is always a binary frame; inbound binary frames
//! are terminal output; inbound text frames are tried as JSON control
//! messages first and fall back to output. Environment variables and the
//! exit-control capability ride WebSocket subprotocol tokens, matching
//! the daemon's `create-connect` protocol.

use std::collections::HashMap;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::TOOLBOX_SDK_VERSION;
use crate::error::DaytonaError;
use crate::process::{build_ws_url, ensure_rustls_crypto_provider, extract_host};
use crate::types::PtyResult;

type ToolboxConfig = daytona_toolbox_client::apis::configuration::Configuration;
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Capability token advertised on PTY WebSocket connects so the daemon
/// sends the `exited` control message; clients that don't send it only
/// get the close frame. (Matches TS `PTY_EXIT_CONTROL_SUBPROTOCOL`.)
const PTY_EXIT_CONTROL_SUBPROTOCOL: &str = "X-Daytona-Pty-Exit-Control";
/// Subprotocol prefix carrying the PTY environment as base64url (no
/// padding) of the JSON object — not a header or query param, so it
/// forwards uniformly across runtimes.
const PTY_ENVS_SUBPROTOCOL_PREFIX: &str = "X-Daytona-Pty-Envs~";
/// Buffered output chunks before the reader backpressures (matches Go's
/// channel capacity).
const OUTPUT_CHANNEL_CAPACITY: usize = 100;
/// Handshake budget, hardcoded to 10 seconds in both reference SDKs.
const CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Connection lifecycle as observed from control messages and the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ConnectionState {
    Pending,
    Connected,
    /// The PTY exited (possibly before any output); the connection was
    /// established but is no longer live.
    Exited,
    Failed(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlMessage {
    #[serde(rename = "type")]
    kind: String,
    status: Option<String>,
    exit_code: Option<i32>,
    exit_reason: Option<String>,
    error: Option<String>,
}

/// Close frames may carry the exit outcome as JSON in the reason.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloseReason {
    exit_code: Option<i32>,
    exit_reason: Option<String>,
    error: Option<String>,
}

/// A live interactive PTY session.
///
/// Obtained from [`crate::ProcessService::create_pty`] or
/// [`crate::ProcessService::connect_pty`], both of which await the
/// connection handshake before returning (TypeScript behavior; Go makes
/// the caller wait explicitly).
///
/// Terminal output arrives on an internal channel: consume it with
/// [`PtyHandle::recv`], or move the receiver into your own task with
/// [`PtyHandle::take_output_receiver`] (Go's `DataChan` shape). The
/// channel closes when the PTY exits. Note the reader backpressures once
/// the channel holds [`OUTPUT_CHANNEL_CAPACITY`] undelivered chunks, so
/// exit notification of a chatty PTY can lag until output is drained.
pub struct PtyHandle {
    session_id: String,
    config: ToolboxConfig,
    writer: Mutex<WsSink>,
    output: Option<mpsc::Receiver<Vec<u8>>>,
    connection: watch::Receiver<ConnectionState>,
    exit: watch::Receiver<Option<PtyResult>>,
    reader: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for PtyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyHandle")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl PtyHandle {
    /// Open the WebSocket at `path` (relative to the toolbox base) and
    /// start the reader. Awaits the `connected` control message — on a
    /// failed handshake the socket is closed rather than leaked.
    pub(crate) async fn connect(
        config: ToolboxConfig,
        session_id: String,
        path: &str,
        envs: Option<&HashMap<String, String>>,
    ) -> Result<Self, DaytonaError> {
        ensure_rustls_crypto_provider();

        let ws_url = build_ws_url(&config.base_path, path)?;
        let mut subprotocols: Vec<String> = Vec::new();
        if let Some(envs) = envs {
            if !envs.is_empty() {
                let json = serde_json::to_string(envs)
                    .map_err(|e| DaytonaError::general(format!("failed to encode envs: {e}")))?;
                subprotocols.push(format!(
                    "{PTY_ENVS_SUBPROTOCOL_PREFIX}{}",
                    base64url_no_pad(json.as_bytes())
                ));
            }
        }
        subprotocols.push(PTY_EXIT_CONTROL_SUBPROTOCOL.to_string());

        let mut request = tungstenite::http::Request::builder()
            .uri(&ws_url)
            .header("Host", extract_host(&ws_url))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .header("Sec-WebSocket-Protocol", subprotocols.join(", "))
            .header("X-Daytona-Source", "rust-sdk")
            .header("X-Daytona-SDK-Version", TOOLBOX_SDK_VERSION);
        if let Some(token) = &config.bearer_access_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(user_agent) = &config.user_agent {
            request = request.header(reqwest::header::USER_AGENT.as_str(), user_agent);
        }
        let request = request
            .body(())
            .map_err(|e| DaytonaError::general(format!("failed to build PTY request: {e}")))?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| DaytonaError::general(format!("failed to connect PTY: {e}")))?;
        let (writer, reader_half) = ws_stream.split();

        let (output_tx, output_rx) = mpsc::channel(OUTPUT_CHANNEL_CAPACITY);
        let (connection_tx, connection_rx) = watch::channel(ConnectionState::Pending);
        let (exit_tx, exit_rx) = watch::channel(None);
        let reader = tokio::spawn(read_loop(reader_half, output_tx, connection_tx, exit_tx));

        let mut handle = PtyHandle {
            session_id,
            config,
            writer: Mutex::new(writer),
            output: Some(output_rx),
            connection: connection_rx,
            exit: exit_rx,
            reader: Some(reader),
        };

        // Both reference SDKs offer waitForConnection; TypeScript awaits
        // it inside createPty/connectPty and disconnects on failure so a
        // failed handshake does not leak the socket. Follow TypeScript.
        if let Err(error) = handle.wait_for_connection().await {
            let _ = handle.disconnect().await;
            return Err(error);
        }
        Ok(handle)
    }

    /// The PTY session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Whether the connection is established and still live.
    pub fn is_connected(&self) -> bool {
        *self.connection.borrow() == ConnectionState::Connected
    }

    /// Exit code, once the PTY has exited.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit.borrow().as_ref().and_then(|r| r.exit_code)
    }

    /// Connection or exit error, when one was reported.
    pub fn error(&self) -> Option<String> {
        self.exit
            .borrow()
            .as_ref()
            .and_then(|r| r.error.clone())
            .or_else(|| match &*self.connection.borrow() {
                ConnectionState::Failed(message) => Some(message.clone()),
                _ => None,
            })
    }

    /// Wait for the connection handshake, bounded by the SDKs' hardcoded
    /// 10-second budget.
    ///
    /// An instantly-exiting PTY still counts as connected — the handshake
    /// must not hang or reject just because the process was short-lived.
    pub async fn wait_for_connection(&mut self) -> Result<(), DaytonaError> {
        let deadline = tokio::time::Instant::now() + CONNECTION_TIMEOUT;
        loop {
            match &*self.connection.borrow() {
                ConnectionState::Connected | ConnectionState::Exited => return Ok(()),
                ConnectionState::Failed(message) => {
                    return Err(DaytonaError::general(format!(
                        "PTY connection failed: {message}"
                    )));
                }
                ConnectionState::Pending => {}
            }
            let changed = tokio::time::timeout_at(deadline, self.connection.changed()).await;
            match changed {
                Err(_) => return Err(DaytonaError::timeout("PTY connection timeout")),
                Ok(Err(_)) => {
                    // Reader ended without ever reporting a state: the
                    // socket died during the handshake.
                    return Err(DaytonaError::general(
                        "PTY connection closed before it was established",
                    ));
                }
                Ok(Ok(())) => {}
            }
        }
    }

    /// Send input to the PTY. Strings and bytes are both accepted and are
    /// always sent as a binary frame (matching both reference SDKs).
    pub async fn send_input(&self, data: impl AsRef<[u8]>) -> Result<(), DaytonaError> {
        if !self.is_connected() {
            return Err(DaytonaError::general("PTY is not connected"));
        }
        self.writer
            .lock()
            .await
            .send(Message::Binary(data.as_ref().to_vec().into()))
            .await
            .map_err(|e| DaytonaError::general(format!("failed to send PTY input: {e}")))
    }

    /// Receive the next chunk of terminal output; `None` once the PTY has
    /// exited and the stream is drained.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        match self.output.as_mut() {
            Some(receiver) => receiver.recv().await,
            None => None,
        }
    }

    /// Move the output receiver out of the handle, e.g. into a task that
    /// pumps it to a terminal (Go's `DataChan` shape). Subsequent
    /// [`PtyHandle::recv`] calls return `None`.
    pub fn take_output_receiver(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output.take()
    }

    /// Resize the PTY via the REST endpoint (not the socket), matching
    /// both reference SDKs.
    pub async fn resize(
        &self,
        cols: u16,
        rows: u16,
    ) -> Result<daytona_toolbox_client::models::PtySessionInfo, DaytonaError> {
        crate::ProcessService {
            config: self.config.clone(),
        }
        .resize_pty_session(&self.session_id, cols, rows)
        .await
    }

    /// Kill the PTY process via the REST endpoint.
    pub async fn kill(&self) -> Result<(), DaytonaError> {
        crate::ProcessService {
            config: self.config.clone(),
        }
        .kill_pty_session(&self.session_id)
        .await
    }

    /// Close the WebSocket without killing the PTY process. Close errors
    /// are swallowed, matching the reference SDKs.
    pub async fn disconnect(&mut self) -> Result<(), DaytonaError> {
        let _ = self.writer.lock().await.close().await;
        if let Some(reader) = self.reader.take() {
            let _ = reader.await;
        }
        Ok(())
    }

    /// Wait for the PTY to exit and return its outcome. Multiple callers
    /// may wait concurrently.
    pub async fn wait(&self) -> Result<PtyResult, DaytonaError> {
        let mut exit = self.exit.clone();
        loop {
            if let Some(result) = exit.borrow().clone() {
                return Ok(result);
            }
            if exit.changed().await.is_err() {
                // Reader ended without recording an exit — the connection
                // dropped without a close frame.
                return Err(DaytonaError::general(
                    self.error()
                        .unwrap_or_else(|| "Connection closed".to_string()),
                ));
            }
        }
    }
}

async fn read_loop(
    mut reader: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    output: mpsc::Sender<Vec<u8>>,
    connection: watch::Sender<ConnectionState>,
    exit: watch::Sender<Option<PtyResult>>,
) {
    let mut error: Option<String> = None;
    while let Some(message) = reader.next().await {
        match message {
            // Binary frames are terminal output, verbatim.
            Ok(Message::Binary(bytes)) => {
                // A send fails only when the receiver was dropped; output
                // is then deliberately discarded while control messages
                // keep flowing.
                let _ = output.send(bytes.to_vec()).await;
            }
            // Text frames are tried as JSON control messages first; only
            // non-control text is terminal output.
            Ok(Message::Text(text)) => {
                if let Ok(control) = serde_json::from_str::<ControlMessage>(&text) {
                    if control.kind == "control" {
                        match control.status.as_deref() {
                            Some("connected") => {
                                let _ = connection.send(ConnectionState::Connected);
                            }
                            Some("exited") => {
                                // An instantly-exiting PTY still means the
                                // connection was established; Exited also
                                // keeps is_connected() honest for a dead
                                // session between "exited" and the close.
                                let _ = connection.send(ConnectionState::Exited);
                                let _ = exit.send(Some(PtyResult {
                                    exit_code: control.exit_code,
                                    error: control.exit_reason,
                                }));
                            }
                            Some("error") => {
                                let message = control
                                    .error
                                    .unwrap_or_else(|| "Unknown connection error".to_string());
                                error = Some(message.clone());
                                let _ = connection.send(ConnectionState::Failed(message));
                            }
                            _ => {}
                        }
                        continue;
                    }
                }
                let _ = output.send(text.as_bytes().to_vec()).await;
            }
            Ok(Message::Close(frame)) => {
                if exit.borrow().is_none() {
                    // Fallback for daemons predating the exit-control
                    // message: the close reason may carry the outcome as
                    // JSON, and a normal close without one means exit 0.
                    let mut result = PtyResult::default();
                    let mut normal_close = false;
                    if let Some(frame) = frame {
                        if let Ok(reason) = serde_json::from_str::<CloseReason>(&frame.reason) {
                            result.exit_code = reason.exit_code;
                            result.error = reason.error.or(reason.exit_reason);
                        }
                        normal_close =
                            frame.code == tungstenite::protocol::frame::coding::CloseCode::Normal;
                    }
                    if result.exit_code.is_none() && normal_close {
                        result.exit_code = Some(0);
                    }
                    if result.error.is_none() {
                        result.error = error.clone();
                    }
                    let _ = exit.send(Some(result));
                }
                break;
            }
            Ok(_) => {}
            Err(e) => {
                let message = format!("PTY read error: {e}");
                error = Some(message.clone());
                let _ = connection.send(ConnectionState::Failed(message));
                break;
            }
        }
    }
    if exit.borrow().is_none() {
        if let Some(message) = error {
            let _ = exit.send(Some(PtyResult {
                exit_code: None,
                error: Some(message),
            }));
        }
    }
    // Dropping `output` closes the stream for `recv`; dropping the watch
    // senders wakes any waiters that never saw an exit.
}

/// Base64url without padding (`+`→`-`, `/`→`_`, no `=`), as the daemon's
/// subprotocol token format requires.
fn base64url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map(u32::from);
        let b2 = chunk.get(2).copied().map(u32::from);
        let triple = (b0 << 16) | (b1.unwrap_or(0) << 8) | b2.unwrap_or(0);
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        if b1.is_some() {
            out.push(ALPHABET[(triple >> 6) as usize & 63] as char);
        }
        if b2.is_some() {
            out.push(ALPHABET[triple as usize & 63] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Accept a WebSocket, negotiating the first offered subprotocol the
    /// way the real daemon does — a client offering subprotocols must be
    /// answered with one, or strict clients (tungstenite, browsers)
    /// reject the handshake.
    async fn accept_with_subprotocol(
        stream: tokio::net::TcpStream,
    ) -> WebSocketStream<tokio::net::TcpStream> {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        tokio_tungstenite::accept_hdr_async(stream, |request: &Request, mut response: Response| {
            if let Some(offered) = request.headers().get("Sec-WebSocket-Protocol") {
                let first = offered
                    .to_str()
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
                    .trim()
                    .to_string();
                response
                    .headers_mut()
                    .insert("Sec-WebSocket-Protocol", first.parse().unwrap());
            }
            Ok(response)
        })
        .await
        .unwrap()
    }

    fn test_config(base_path: String) -> ToolboxConfig {
        ToolboxConfig {
            base_path,
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: None,
            api_key: None,
        }
    }

    #[tokio::test]
    async fn test_pty_handshake_io_and_exit() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_with_subprotocol(stream).await;
            ws.send(Message::Text(
                r#"{"type":"control","status":"connected"}"#.into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Binary(b"shell$ ".to_vec().into()))
                .await
                .unwrap();
            let input = loop {
                match ws.next().await {
                    Some(Ok(Message::Binary(bytes))) => break bytes,
                    Some(Ok(_)) => continue,
                    other => panic!("expected input frame, got {other:?}"),
                }
            };
            assert_eq!(input.as_ref(), b"ls\n");
            ws.send(Message::Binary(b"file-a\n".to_vec().into()))
                .await
                .unwrap();
            ws.send(Message::Text(
                r#"{"type":"control","status":"exited","exitCode":0}"#.into(),
            ))
            .await
            .unwrap();
            ws.close(None).await.unwrap();
        });

        let mut handle = PtyHandle::connect(
            test_config(format!("http://{addr}")),
            "pty-1".to_string(),
            "/process/pty/pty-1/connect",
            None,
        )
        .await
        .unwrap();

        assert!(handle.is_connected());
        assert_eq!(handle.recv().await.unwrap(), b"shell$ ");
        handle.send_input("ls\n").await.unwrap();
        assert_eq!(handle.recv().await.unwrap(), b"file-a\n");

        let result = handle.wait().await.unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(handle.exit_code(), Some(0));
        assert!(!handle.is_connected());
        assert!(handle.recv().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_pty_instantly_exiting_session_still_connects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_with_subprotocol(stream).await;
            // No "connected" first: the PTY exits immediately. The
            // handshake must not hang or reject.
            ws.send(Message::Text(
                r#"{"type":"control","status":"exited","exitCode":7,"exitReason":"done"}"#.into(),
            ))
            .await
            .unwrap();
            ws.close(None).await.unwrap();
        });

        let handle = PtyHandle::connect(
            test_config(format!("http://{addr}")),
            "pty-2".to_string(),
            "/process/pty/pty-2/connect",
            None,
        )
        .await
        .unwrap();

        let result = handle.wait().await.unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.error.as_deref(), Some("done"));
        assert!(!handle.is_connected());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_pty_normal_close_without_exit_control_means_exit_zero() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_with_subprotocol(stream).await;
            ws.send(Message::Text(
                r#"{"type":"control","status":"connected"}"#.into(),
            ))
            .await
            .unwrap();
            // A daemon predating exit-control: only a normal close frame.
            ws.close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: "".into(),
            }))
            .await
            .unwrap();
        });

        let handle = PtyHandle::connect(
            test_config(format!("http://{addr}")),
            "pty-3".to_string(),
            "/process/pty/pty-3/connect",
            None,
        )
        .await
        .unwrap();

        let result = handle.wait().await.unwrap();
        assert_eq!(result.exit_code, Some(0));
        server.await.unwrap();
    }

    #[test]
    fn test_base64url_no_pad() {
        assert_eq!(base64url_no_pad(b""), "");
        assert_eq!(base64url_no_pad(b"f"), "Zg");
        assert_eq!(base64url_no_pad(b"fo"), "Zm8");
        assert_eq!(base64url_no_pad(b"foo"), "Zm9v");
        // Bytes that hit the url-safe alphabet substitutions.
        assert_eq!(base64url_no_pad(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn test_control_message_parses_camel_case() {
        let control: ControlMessage = serde_json::from_str(
            r#"{"type":"control","status":"exited","exitCode":3,"exitReason":"boom"}"#,
        )
        .unwrap();
        assert_eq!(control.kind, "control");
        assert_eq!(control.status.as_deref(), Some("exited"));
        assert_eq!(control.exit_code, Some(3));
        assert_eq!(control.exit_reason.as_deref(), Some("boom"));
    }
}
