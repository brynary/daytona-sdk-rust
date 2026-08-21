use std::future::Future;
use std::sync::Once;

use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::{process_api, urlencode};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_tungstenite::tungstenite;

use crate::client::{convert_toolbox_error, TOOLBOX_SDK_VERSION};
use crate::error::DaytonaError;
use crate::types::CodeLanguage;
use crate::types::ExecuteCommandOptions;
use crate::types::ExecuteResponse;

const STDOUT_PREFIX_BYTES: &[u8] = &[0x01, 0x01, 0x01];
const STDERR_PREFIX_BYTES: &[u8] = &[0x02, 0x02, 0x02];
const MAX_PREFIX_LEN: usize = 3;
static RUSTLS_PROVIDER: Once = Once::new();

/// Result of executing a session command.
#[derive(Debug, Clone)]
pub struct SessionExecuteResult {
    /// The command identifier.
    pub cmd_id: String,
    /// Exit code, present if the command completed synchronously.
    pub exit_code: Option<i32>,
    /// Combined output, present if the server returned it.
    pub output: Option<String>,
    /// Standard output, present if the command completed synchronously.
    pub stdout: Option<String>,
    /// Standard error, present if the command completed synchronously.
    pub stderr: Option<String>,
}

/// Logs for a command executed inside a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionCommandLogsResult {
    /// Combined command output.
    pub output: String,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Whether stdout/stderr were separated by the server or demuxed locally.
    pub streams_separated: bool,
}

/// Service for executing commands and managing sessions in a sandbox.
pub struct ProcessService {
    pub(crate) config: ToolboxConfig,
}

impl ProcessService {
    /// Execute a command in the sandbox.
    pub async fn execute_command(
        &self,
        command: &str,
        options: ExecuteCommandOptions,
    ) -> Result<ExecuteResponse, DaytonaError> {
        let timeout_secs = options.timeout.map(|d| d.as_secs() as i32);

        let exec_body = daytona_toolbox_client::models::ExecuteRequest {
            command: command.to_string(),
            cwd: options.cwd,
            envs: options.env.filter(|env| !env.is_empty()),
            timeout: timeout_secs,
        };

        let result = process_api::execute_command(&self.config, exec_body)
            .await
            .map_err(convert_toolbox_error)?;

        Ok(ExecuteResponse {
            exit_code: result.exit_code.unwrap_or(0) as i32,
            result: result.result,
            artifacts: Vec::new(),
        })
    }

    /// Execute code in a language-specific way.
    ///
    /// This wraps [`ProcessService::execute_command`] with a language-specific
    /// command prefix, matching the TypeScript SDK's `codeRun` method.
    ///
    /// # Arguments
    /// * `code` - The source code to execute
    /// * `language` - The programming language to use
    /// * `options` - Optional execution settings (cwd, env, timeout)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use daytona_sdk::types::{CodeLanguage, ExecuteCommandOptions};
    /// # async fn example(process: &daytona_sdk::ProcessService) -> Result<(), daytona_sdk::DaytonaError> {
    /// let result = process.code_run(
    ///     "print('Hello, World!')",
    ///     CodeLanguage::Python,
    ///     ExecuteCommandOptions::default(),
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn code_run(
        &self,
        code: &str,
        language: CodeLanguage,
        options: ExecuteCommandOptions,
    ) -> Result<ExecuteResponse, DaytonaError> {
        let command = match language {
            CodeLanguage::Python => {
                format!("python3 -c {}", shell_escape(code))
            }
            CodeLanguage::Javascript => {
                format!("node -e {}", shell_escape(code))
            }
            CodeLanguage::Typescript => {
                format!("npx ts-node -e {}", shell_escape(code))
            }
        };
        self.execute_command(&command, options).await
    }

    /// Create a new session.
    pub async fn create_session(&self, session_id: &str) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::CreateSessionRequest {
            session_id: session_id.to_string(),
        };
        process_api::create_session(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Delete a session.
    pub async fn delete_session(&self, session_id: &str) -> Result<(), DaytonaError> {
        process_api::delete_session(&self.config, session_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Execute a command in an existing session.
    ///
    /// When `run_async` is true, the command returns immediately without waiting
    /// for completion. Use [`ProcessService::get_session_command`] to check status.
    ///
    /// Set `suppress_input_echo` to true to suppress the input echo in the output.
    pub async fn execute_session_command(
        &self,
        session_id: &str,
        command: &str,
        run_async: bool,
        suppress_input_echo: bool,
    ) -> Result<SessionExecuteResult, DaytonaError> {
        let req = daytona_toolbox_client::models::SessionExecuteRequest {
            command: command.to_string(),
            r#async: None,
            run_async: Some(run_async),
            suppress_input_echo: Some(suppress_input_echo),
        };

        let result = process_api::session_execute_command(&self.config, session_id, req)
            .await
            .map_err(convert_toolbox_error)?;

        Ok(SessionExecuteResult {
            cmd_id: result.cmd_id,
            exit_code: result.exit_code,
            output: result.output,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }

    /// List sessions.
    pub async fn list_sessions(
        &self,
    ) -> Result<Vec<daytona_toolbox_client::models::Session>, DaytonaError> {
        let sessions = process_api::list_sessions(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(sessions)
    }

    /// Get a session.
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<daytona_toolbox_client::models::Session, DaytonaError> {
        let session = process_api::get_session(&self.config, session_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(session)
    }

    /// Get session command result.
    pub async fn get_session_command(
        &self,
        session_id: &str,
        command_id: &str,
    ) -> Result<daytona_toolbox_client::models::Command, DaytonaError> {
        let cmd = process_api::get_session_command(&self.config, session_id, command_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(cmd)
    }

    /// Get session command logs.
    pub async fn get_session_command_logs(
        &self,
        session_id: &str,
        command_id: &str,
    ) -> Result<SessionCommandLogsResult, DaytonaError> {
        let uri = format!(
            "{}/process/session/{}/command/{}/logs",
            self.config.base_path,
            urlencode(session_id),
            urlencode(command_id)
        );
        let mut req_builder = self.config.client.get(&uri);
        if let Some(user_agent) = &self.config.user_agent {
            req_builder = req_builder.header(reqwest::header::USER_AGENT, user_agent.clone());
        }
        req_builder =
            req_builder.header(reqwest::header::ACCEPT, "application/json, text/plain, */*");

        let resp = req_builder
            .send()
            .await
            .map_err(|e| DaytonaError::general(format!("failed to fetch command logs: {e}")))?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp
            .bytes()
            .await
            .map_err(|e| DaytonaError::general(format!("failed to read command logs: {e}")))?;

        if !status.is_success() {
            let body_text = String::from_utf8_lossy(&body);
            return Err(DaytonaError::api(
                status.as_u16(),
                format!("failed to fetch command logs: {body_text}"),
            ));
        }

        Ok(parse_session_command_logs(&body, content_type.as_deref()))
    }

    /// Stream session command logs through separate stdout/stderr callbacks.
    pub async fn get_session_command_logs_stream<FOut, FErr, FutOut, FutErr>(
        &self,
        session_id: &str,
        command_id: &str,
        mut on_stdout: FOut,
        mut on_stderr: FErr,
    ) -> Result<(), DaytonaError>
    where
        FOut: FnMut(String) -> FutOut + Send,
        FErr: FnMut(String) -> FutErr + Send,
        FutOut: Future<Output = Result<(), DaytonaError>> + Send,
        FutErr: Future<Output = Result<(), DaytonaError>> + Send,
    {
        ensure_rustls_crypto_provider();

        let path = format!(
            "/process/session/{}/command/{}/logs?follow=true",
            urlencode(session_id),
            urlencode(command_id)
        );
        let ws_url = build_ws_url(&self.config.base_path, &path)?;
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
            .header("X-Daytona-Source", "rust-sdk")
            .header("X-Daytona-SDK-Version", TOOLBOX_SDK_VERSION)
            .header("X-Daytona-Split-Output", "true");

        if let Some(token) = &self.config.bearer_access_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(user_agent) = &self.config.user_agent {
            request = request.header(reqwest::header::USER_AGENT.as_str(), user_agent);
        }

        let request = request.body(()).map_err(|e| {
            DaytonaError::general(format!("failed to build log stream request: {e}"))
        })?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| DaytonaError::general(format!("failed to connect to log stream: {e}")))?;
        let (_write, mut read) = ws_stream.split();
        let mut demux = StdDemux::default();

        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(tungstenite::Error::Protocol(
                    tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                )) => break,
                Err(tungstenite::Error::ConnectionClosed) => break,
                Err(e) => {
                    return Err(DaytonaError::general(format!("log stream read error: {e}")));
                }
            };

            let chunks = match msg {
                tungstenite::Message::Text(text) => demux.push(text.as_bytes()),
                tungstenite::Message::Binary(bytes) => demux.push(bytes.as_ref()),
                tungstenite::Message::Close(_) => break,
                _ => Vec::new(),
            };
            for chunk in chunks {
                match chunk.stream {
                    StreamKind::Stdout => on_stdout(chunk.text).await?,
                    StreamKind::Stderr => on_stderr(chunk.text).await?,
                }
            }
        }

        for chunk in demux.finish() {
            match chunk.stream {
                StreamKind::Stdout => on_stdout(chunk.text).await?,
                StreamKind::Stderr => on_stderr(chunk.text).await?,
            }
        }

        Ok(())
    }

    /// Create a PTY session.
    ///
    /// The `id` parameter allows specifying a custom session identifier, matching
    /// Go/TypeScript SDK behavior. Pass an empty string to let the server generate one.
    pub async fn create_pty_session(
        &self,
        id: &str,
        options: crate::types::PtySessionOptions,
    ) -> Result<String, DaytonaError> {
        let req = daytona_toolbox_client::models::PtyCreateRequest {
            cols: options.size.as_ref().map(|s| s.cols as i32),
            rows: options.size.as_ref().map(|s| s.rows as i32),
            cwd: None,
            envs: options.env,
            id: if id.is_empty() {
                None
            } else {
                Some(id.to_string())
            },
            lazy_start: None,
        };
        let resp = process_api::create_pty_session(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp.session_id)
    }

    /// List PTY sessions.
    pub async fn list_pty_sessions(
        &self,
    ) -> Result<daytona_toolbox_client::models::PtyListResponse, DaytonaError> {
        let sessions = process_api::list_pty_sessions(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(sessions)
    }

    /// Get PTY session info.
    pub async fn get_pty_session(
        &self,
        session_id: &str,
    ) -> Result<daytona_toolbox_client::models::PtySessionInfo, DaytonaError> {
        let info = process_api::get_pty_session(&self.config, session_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(info)
    }

    /// Delete (kill) a PTY session.
    pub async fn kill_pty_session(&self, session_id: &str) -> Result<(), DaytonaError> {
        process_api::delete_pty_session(&self.config, session_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Resize a PTY session.
    pub async fn resize_pty_session(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<daytona_toolbox_client::models::PtySessionInfo, DaytonaError> {
        let req = daytona_toolbox_client::models::PtyResizeRequest {
            cols: cols as i32,
            rows: rows as i32,
        };
        let info = process_api::resize_pty_session(&self.config, session_id, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(info)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DemuxChunk {
    stream: StreamKind,
    text: String,
}

#[derive(Default)]
struct StdDemux {
    buffer: Vec<u8>,
    current_kind: Option<StreamKind>,
    saw_marker: bool,
}

impl StdDemux {
    fn push(&mut self, bytes: &[u8]) -> Vec<DemuxChunk> {
        if bytes.is_empty() {
            return Vec::new();
        }

        self.buffer.extend_from_slice(bytes);
        let mut chunks = Vec::new();

        loop {
            let safe_len = safe_demux_len(&self.buffer);
            if safe_len == 0 {
                break;
            }

            let safe_region = &self.buffer[..safe_len];
            let stdout_idx = find_subslice(safe_region, STDOUT_PREFIX_BYTES);
            let stderr_idx = find_subslice(safe_region, STDERR_PREFIX_BYTES);
            let next = match (stdout_idx, stderr_idx) {
                (Some(out), Some(err)) if out <= err => {
                    Some((out, StreamKind::Stdout, STDOUT_PREFIX_BYTES.len()))
                }
                (Some(out), None) => Some((out, StreamKind::Stdout, STDOUT_PREFIX_BYTES.len())),
                (_, Some(err)) => Some((err, StreamKind::Stderr, STDERR_PREFIX_BYTES.len())),
                (None, None) => None,
            };

            match next {
                Some((idx, next_kind, marker_len)) => {
                    if idx > 0 {
                        emit_demux_chunk(self.current_kind, &self.buffer[..idx], &mut chunks);
                    }
                    self.buffer.drain(..idx + marker_len);
                    self.current_kind = Some(next_kind);
                    self.saw_marker = true;
                }
                None => {
                    emit_demux_chunk(self.current_kind, &self.buffer[..safe_len], &mut chunks);
                    self.buffer.drain(..safe_len);
                    break;
                }
            }
        }

        chunks
    }

    fn finish(mut self) -> Vec<DemuxChunk> {
        let mut chunks = Vec::new();
        emit_demux_chunk(self.current_kind, &self.buffer, &mut chunks);
        self.buffer.clear();
        chunks
    }
}

#[derive(Debug, Deserialize)]
struct SessionCommandLogsBody {
    output: Option<String>,
    stdout: Option<String>,
    stderr: Option<String>,
}

fn parse_session_command_logs(body: &[u8], content_type: Option<&str>) -> SessionCommandLogsResult {
    if content_type
        .map(|ct| ct.contains("application/json"))
        .unwrap_or(false)
    {
        if let Ok(json) = serde_json::from_slice::<SessionCommandLogsBody>(body) {
            return SessionCommandLogsResult {
                output: json.output.unwrap_or_default(),
                stdout: json.stdout.unwrap_or_default(),
                stderr: json.stderr.unwrap_or_default(),
                streams_separated: true,
            };
        }

        if let Ok(json_string) = serde_json::from_slice::<String>(body) {
            return parse_plain_or_muxed_logs(json_string.as_bytes());
        }
    }

    if let Ok(json) = serde_json::from_slice::<SessionCommandLogsBody>(body) {
        return SessionCommandLogsResult {
            output: json.output.unwrap_or_default(),
            stdout: json.stdout.unwrap_or_default(),
            stderr: json.stderr.unwrap_or_default(),
            streams_separated: true,
        };
    }

    parse_plain_or_muxed_logs(body)
}

fn parse_plain_or_muxed_logs(body: &[u8]) -> SessionCommandLogsResult {
    let mut demux = StdDemux::default();
    let mut chunks = demux.push(body);
    let saw_marker = demux.saw_marker;
    chunks.extend(demux.finish());

    if !saw_marker {
        let output = String::from_utf8_lossy(body).to_string();
        return SessionCommandLogsResult {
            output: output.clone(),
            stdout: output,
            stderr: String::new(),
            streams_separated: false,
        };
    }

    let mut result = SessionCommandLogsResult {
        streams_separated: true,
        ..Default::default()
    };
    for chunk in chunks {
        result.output.push_str(&chunk.text);
        match chunk.stream {
            StreamKind::Stdout => result.stdout.push_str(&chunk.text),
            StreamKind::Stderr => result.stderr.push_str(&chunk.text),
        }
    }
    result
}

fn emit_demux_chunk(kind: Option<StreamKind>, bytes: &[u8], chunks: &mut Vec<DemuxChunk>) {
    if bytes.is_empty() {
        return;
    }
    if let Some(stream) = kind {
        chunks.push(DemuxChunk {
            stream,
            text: String::from_utf8_lossy(bytes).to_string(),
        });
    }
}

fn safe_demux_len(buffer: &[u8]) -> usize {
    let mut keep = 0;
    let max_keep = MAX_PREFIX_LEN - 1;
    for len in 1..=max_keep.min(buffer.len()) {
        let suffix = &buffer[buffer.len() - len..];
        if STDOUT_PREFIX_BYTES.starts_with(suffix) || STDERR_PREFIX_BYTES.starts_with(suffix) {
            keep = keep.max(len);
        }
    }
    buffer.len() - keep
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Convert an HTTP(S) base URL to a WebSocket URL with the given path.
fn build_ws_url(base_url: &str, path: &str) -> Result<String, DaytonaError> {
    let full = format!("{base_url}{path}");
    let url =
        url::Url::parse(&full).map_err(|e| DaytonaError::general(format!("invalid URL: {e}")))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => {
            return Err(DaytonaError::general(format!(
                "unsupported scheme: {other}"
            )));
        }
    };
    let mut ws_url = url.clone();
    ws_url
        .set_scheme(scheme)
        .map_err(|_| DaytonaError::general("failed to set WebSocket scheme"))?;
    Ok(ws_url.to_string())
}

/// Extract the host (with port) from a URL string for the Host header.
fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| {
                if let Some(port) = u.port() {
                    format!("{h}:{port}")
                } else {
                    h.to_string()
                }
            })
        })
        .unwrap_or_default()
}

fn ensure_rustls_crypto_provider() {
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Shell-escape a string for use in a command, wrapping in single quotes.
fn shell_escape(s: &str) -> String {
    // Replace single quotes with '\'' (end quote, escaped quote, start quote)
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn process_service(mock_server: &MockServer) -> ProcessService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        ProcessService { config }
    }

    #[tokio::test]
    async fn test_execute_command() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "command": "echo hello world",
                "exitCode": 0,
                "result": "hello world\n"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let result = svc
            .execute_command("echo hello world", ExecuteCommandOptions::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.result, "hello world\n");
    }

    #[tokio::test]
    async fn test_execute_command_with_cwd() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "command": "pwd",
                "exitCode": 0,
                "result": "/tmp\n"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let opts = ExecuteCommandOptions {
            cwd: Some("/tmp".to_string()),
            ..Default::default()
        };
        let result = svc.execute_command("pwd", opts).await.unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn test_execute_command_with_environment() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/execute"))
            .and(body_json(serde_json::json!({
                "command": "printenv MODE",
                "envs": {"MODE": "test"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "command": "printenv MODE",
                "exitCode": 0,
                "result": "test\n"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let result = svc
            .execute_command(
                "printenv MODE",
                ExecuteCommandOptions {
                    env: Some(std::collections::HashMap::from([(
                        "MODE".to_string(),
                        "test".to_string(),
                    )])),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.result, "test\n");
    }

    #[tokio::test]
    async fn test_create_and_delete_session() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/session"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/process/session/sess-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        svc.create_session("sess-1").await.unwrap();
        svc.delete_session("sess-1").await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_session_command() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/session/sess-1/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "cmdId": "cmd-abc",
                "command": "ls -la"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let result = svc
            .execute_session_command("sess-1", "ls -la", false, false)
            .await
            .unwrap();
        assert_eq!(result.cmd_id, "cmd-abc");
    }

    #[tokio::test]
    async fn test_execute_session_command_sync_result() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/session/sess-1/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "cmdId": "cmd-sync",
                "exitCode": 0,
                "stdout": "hello\n",
                "stderr": ""
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let result = svc
            .execute_session_command("sess-1", "echo hello", false, true)
            .await
            .unwrap();
        assert_eq!(result.cmd_id, "cmd-sync");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.output, None);
        assert_eq!(result.stdout.as_deref(), Some("hello\n"));
    }

    #[tokio::test]
    async fn test_get_session_command_logs_json() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/session/sess-1/command/cmd-1/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({
                        "output": "out\nerr\n",
                        "stdout": "out\n",
                        "stderr": "err\n"
                    })),
            )
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let logs = svc
            .get_session_command_logs("sess-1", "cmd-1")
            .await
            .unwrap();
        assert_eq!(logs.output, "out\nerr\n");
        assert_eq!(logs.stdout, "out\n");
        assert_eq!(logs.stderr, "err\n");
        assert!(logs.streams_separated);
    }

    #[tokio::test]
    async fn test_get_session_command_logs_defaults_missing_json_fields() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/session/sess-1/command/cmd-1/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(serde_json::json!({})),
            )
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let logs = svc
            .get_session_command_logs("sess-1", "cmd-1")
            .await
            .unwrap();
        assert_eq!(
            logs,
            SessionCommandLogsResult {
                output: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                streams_separated: true,
            }
        );
    }

    #[tokio::test]
    async fn test_get_session_command_logs_plain_text_fallback() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/session/sess-1/command/cmd-1/logs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("line 1\nline 2\n"),
            )
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let logs = svc
            .get_session_command_logs("sess-1", "cmd-1")
            .await
            .unwrap();
        assert_eq!(logs.output, "line 1\nline 2\n");
        assert_eq!(logs.stdout, "line 1\nline 2\n");
        assert_eq!(logs.stderr, "");
        assert!(!logs.streams_separated);
    }

    #[test]
    fn test_parse_muxed_session_command_logs() {
        let body = [
            STDOUT_PREFIX_BYTES,
            b"stdout line 1\n",
            STDERR_PREFIX_BYTES,
            b"stderr line 1\n",
            STDOUT_PREFIX_BYTES,
            b"stdout line 2\n",
        ]
        .concat();

        let logs = parse_plain_or_muxed_logs(&body);

        assert_eq!(logs.output, "stdout line 1\nstderr line 1\nstdout line 2\n");
        assert_eq!(logs.stdout, "stdout line 1\nstdout line 2\n");
        assert_eq!(logs.stderr, "stderr line 1\n");
        assert!(logs.streams_separated);
    }

    #[test]
    fn test_demux_handles_split_markers() {
        let mut demux = StdDemux::default();
        assert!(demux.push(&[0x01]).is_empty());
        assert!(demux.push(&[0x01]).is_empty());
        assert_eq!(
            demux.push(&[0x01, b'h', b'i']),
            vec![DemuxChunk {
                stream: StreamKind::Stdout,
                text: "hi".to_string(),
            }]
        );
        assert_eq!(demux.finish(), Vec::<DemuxChunk>::new());
    }

    #[tokio::test]
    async fn test_get_session_command_logs_stream_demuxes_websocket() {
        use futures_util::SinkExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(tungstenite::Message::Binary(
                [STDOUT_PREFIX_BYTES, b"hello "].concat().into(),
            ))
            .await
            .unwrap();
            ws.send(tungstenite::Message::Binary(b"world\n".to_vec().into()))
                .await
                .unwrap();
            ws.send(tungstenite::Message::Binary(
                [STDERR_PREFIX_BYTES, b"err\n"].concat().into(),
            ))
            .await
            .unwrap();
            ws.close(None).await.unwrap();
        });

        let config = ToolboxConfig {
            base_path: format!("http://{addr}"),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: None,
            api_key: None,
        };
        let svc = ProcessService { config };
        let stdout = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
        let stderr = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));

        svc.get_session_command_logs_stream(
            "sess-1",
            "cmd-1",
            {
                let stdout = stdout.clone();
                move |chunk| {
                    let stdout = stdout.clone();
                    async move {
                        stdout.lock().await.push_str(&chunk);
                        Ok(())
                    }
                }
            },
            {
                let stderr = stderr.clone();
                move |chunk| {
                    let stderr = stderr.clone();
                    async move {
                        stderr.lock().await.push_str(&chunk);
                        Ok(())
                    }
                }
            },
        )
        .await
        .unwrap();

        server.await.unwrap();
        assert_eq!(&*stdout.lock().await, "hello world\n");
        assert_eq!(&*stderr.lock().await, "err\n");
    }

    #[tokio::test]
    async fn test_create_pty_session() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/pty"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sessionId": "pty-123"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let session_id = svc
            .create_pty_session("", crate::types::PtySessionOptions::default())
            .await
            .unwrap();
        assert_eq!(session_id, "pty-123");
    }

    #[tokio::test]
    async fn test_create_pty_session_with_size() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/pty"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sessionId": "pty-456"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let opts = crate::types::PtySessionOptions {
            size: Some(crate::types::PtySize { rows: 24, cols: 80 }),
            ..Default::default()
        };
        let session_id = svc.create_pty_session("pty-custom", opts).await.unwrap();
        assert_eq!(session_id, "pty-456");
    }

    #[tokio::test]
    async fn test_list_pty_sessions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/pty"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sessions": [
                    {"id": "pty-1", "active": true, "cols": 80, "rows": 24, "createdAt": "2024-01-01", "cwd": "/home", "envs": {}, "lazyStart": false}
                ]
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let _sessions = svc.list_pty_sessions().await.unwrap();
    }

    #[tokio::test]
    async fn test_get_pty_session() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/pty/pty-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pty-1",
                "active": true,
                "cols": 80,
                "rows": 24,
                "createdAt": "2024-01-01",
                "cwd": "/home",
                "envs": {},
                "lazyStart": false
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let info = svc.get_pty_session("pty-1").await.unwrap();
        assert_eq!(info.id, "pty-1");
        assert!(info.active);
    }

    #[tokio::test]
    async fn test_kill_pty_session() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/process/pty/pty-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        svc.kill_pty_session("pty-1").await.unwrap();
    }

    #[tokio::test]
    async fn test_resize_pty_session() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/pty/pty-1/resize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pty-1",
                "active": true,
                "cols": 120,
                "rows": 40,
                "createdAt": "2024-01-01",
                "cwd": "/home",
                "envs": {},
                "lazyStart": false
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let info = svc.resize_pty_session("pty-1", 120, 40).await.unwrap();
        assert_eq!(info.cols, 120);
        assert_eq!(info.rows, 40);
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"sessionId": "sess-1", "commands": []},
                {"sessionId": "sess-2", "commands": []}
            ])))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let sessions = svc.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn test_code_run_python() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "command": "python3 -c 'print(42)'",
                "exitCode": 0,
                "result": "42\n"
            })))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let result = svc
            .code_run(
                "print(42)",
                crate::types::CodeLanguage::Python,
                ExecuteCommandOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.result, "42\n");
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("a b c"), "'a b c'");
    }
}
