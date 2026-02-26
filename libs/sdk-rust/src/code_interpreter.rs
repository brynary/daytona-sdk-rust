use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::interpreter_api;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;

/// An execution error returned by the code interpreter.
#[derive(Debug, Clone)]
pub struct ExecutionError {
    pub name: String,
    pub value: String,
    pub traceback: Option<String>,
}

/// The final result of a code execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub error: Option<ExecutionError>,
}

/// A single output message from the code interpreter.
#[derive(Debug, Clone)]
pub struct OutputMessage {
    /// "stdout", "stderr", or "error"
    pub msg_type: String,
    pub text: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub traceback: Option<String>,
}

/// Service for code interpreter operations in a sandbox.
pub struct CodeInterpreterService {
    pub(crate) config: ToolboxConfig,
}

impl CodeInterpreterService {
    /// Execute code in the interpreter and collect the result.
    ///
    /// This connects via WebSocket to the interpreter execute endpoint,
    /// sends the code, and collects all output until the connection closes.
    ///
    /// Matches the Go/TypeScript SDK `RunCode` behavior but returns the
    /// collected result synchronously rather than using channels (idiomatic Rust
    /// uses `await` instead of channel-based streaming).
    ///
    /// # Arguments
    /// * `code` - The source code to execute
    /// * `options` - Optional execution settings (context, env, timeout)
    pub async fn run_code(
        &self,
        code: &str,
        options: crate::types::RunCodeOptions,
    ) -> Result<ExecutionResult, DaytonaError> {
        // Build WebSocket URL from the toolbox base URL
        let base_url = &self.config.base_path;
        let ws_url = build_ws_url(base_url, "/process/interpreter/execute")?;

        // Build connection request with auth headers (matching Go SDK's buildHeaders)
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
            .header("X-Daytona-SDK-Version", env!("CARGO_PKG_VERSION"));

        if let Some(token) = &self.config.bearer_access_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let request = request
            .body(())
            .map_err(|e| DaytonaError::general(format!("failed to build WebSocket request: {}", e)))?;

        let (ws_stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| {
                DaytonaError::general(format!("failed to connect to code interpreter: {}", e))
            })?;

        let (mut write, mut read) = ws_stream.split();

        // Build the execute request payload (matches Go/TS: no language field)
        let mut exec_req = serde_json::json!({
            "code": code,
        });
        if let Some(ctx_id) = &options.context_id {
            exec_req["contextId"] = serde_json::Value::String(ctx_id.clone());
        }
        if let Some(env) = &options.env {
            exec_req["envs"] = serde_json::to_value(env)
                .map_err(|e| DaytonaError::general(e.to_string()))?;
        }
        if let Some(timeout) = &options.timeout {
            exec_req["timeout"] = serde_json::Value::Number(
                serde_json::Number::from(timeout.as_secs() as u64),
            );
        }

        write
            .send(tungstenite::Message::Text(exec_req.to_string().into()))
            .await
            .map_err(|e| {
                DaytonaError::general(format!("failed to send execute request: {}", e))
            })?;

        // Collect output
        let mut result = ExecutionResult::default();
        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();

        while let Some(msg) = read.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(tungstenite::Error::Protocol(
                    tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                )) => break,
                Err(e) => {
                    // Normal close
                    if matches!(&e, tungstenite::Error::ConnectionClosed) {
                        break;
                    }
                    result.error = Some(ExecutionError {
                        name: "ConnectionError".to_string(),
                        value: format!("WebSocket error: {}", e),
                        traceback: None,
                    });
                    break;
                }
            };

            match msg {
                tungstenite::Message::Text(text) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        let msg_type = json
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        match msg_type {
                            "stdout" => {
                                let content = json
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                stdout_buf.push_str(content);
                            }
                            "stderr" => {
                                let content = json
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                stderr_buf.push_str(content);
                            }
                            "error" => {
                                let name = json
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Error")
                                    .to_string();
                                let value = json
                                    .get("value")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let traceback = json
                                    .get("traceback")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                result.error = Some(ExecutionError {
                                    name,
                                    value,
                                    traceback,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                tungstenite::Message::Close(_) => break,
                _ => {}
            }
        }

        result.stdout = stdout_buf;
        result.stderr = stderr_buf;

        Ok(result)
    }

    /// Create a new interpreter context.
    ///
    /// An optional `cwd` sets the working directory for the context.
    /// Language is determined by the server (defaults to Python).
    pub async fn create_context(
        &self,
        cwd: Option<&str>,
    ) -> Result<daytona_toolbox_client::models::InterpreterContext, DaytonaError> {
        let req = daytona_toolbox_client::models::CreateContextRequest {
            language: None,
            cwd: cwd.map(|s| s.to_string()),
        };
        let context = interpreter_api::create_interpreter_context(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(context)
    }

    /// List interpreter contexts.
    pub async fn list_contexts(
        &self,
    ) -> Result<daytona_toolbox_client::models::ListContextsResponse, DaytonaError> {
        let contexts = interpreter_api::list_interpreter_contexts(&self.config)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(contexts)
    }

    /// Delete an interpreter context.
    pub async fn delete_context(&self, context_id: &str) -> Result<(), DaytonaError> {
        interpreter_api::delete_interpreter_context(&self.config, context_id)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }
}

/// Convert an HTTP(S) base URL to a WebSocket URL with the given path.
fn build_ws_url(base_url: &str, path: &str) -> Result<String, DaytonaError> {
    let full = format!("{}{}", base_url, path);
    let url = url::Url::parse(&full)
        .map_err(|e| DaytonaError::general(format!("invalid URL: {}", e)))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => {
            return Err(DaytonaError::general(format!(
                "unsupported scheme: {}",
                other
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
        .and_then(|u| u.host_str().map(|h| {
            if let Some(port) = u.port() {
                format!("{}:{}", h, port)
            } else {
                h.to_string()
            }
        }))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn interpreter_service(mock_server: &MockServer) -> CodeInterpreterService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        CodeInterpreterService { config }
    }

    #[tokio::test]
    async fn test_create_context() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/interpreter/context"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-123",
                "language": "python",
                "active": true,
                "createdAt": "2024-01-01T00:00:00Z",
                "cwd": "/home/daytona"
            })))
            .mount(&mock_server)
            .await;

        let svc = interpreter_service(&mock_server).await;
        let ctx = svc.create_context(None).await.unwrap();
        assert_eq!(ctx.id, "ctx-123");
    }

    #[tokio::test]
    async fn test_create_context_with_cwd() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/process/interpreter/context"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-456",
                "language": "python",
                "active": true,
                "createdAt": "2024-01-01T00:00:00Z",
                "cwd": "/home/daytona/project"
            })))
            .mount(&mock_server)
            .await;

        let svc = interpreter_service(&mock_server).await;
        let ctx = svc
            .create_context(Some("/home/daytona/project"))
            .await
            .unwrap();
        assert_eq!(ctx.id, "ctx-456");
    }

    #[tokio::test]
    async fn test_list_contexts() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/interpreter/context"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "contexts": [
                    {"id": "ctx-1", "language": "python", "active": true, "createdAt": "2024-01-01T00:00:00Z", "cwd": "/home"},
                    {"id": "ctx-2", "language": "javascript", "active": true, "createdAt": "2024-01-01T00:00:00Z", "cwd": "/home"}
                ]
            })))
            .mount(&mock_server)
            .await;

        let svc = interpreter_service(&mock_server).await;
        let response = svc.list_contexts().await.unwrap();
    }

    #[tokio::test]
    async fn test_delete_context() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/process/interpreter/context/ctx-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let svc = interpreter_service(&mock_server).await;
        svc.delete_context("ctx-123").await.unwrap();
    }

    #[test]
    fn test_build_ws_url_http() {
        let result = build_ws_url("http://localhost:8080", "/process/interpreter/execute").unwrap();
        assert_eq!(result, "ws://localhost:8080/process/interpreter/execute");
    }

    #[test]
    fn test_build_ws_url_https() {
        let result =
            build_ws_url("https://proxy.daytona.io/sb-1", "/process/interpreter/execute").unwrap();
        assert_eq!(
            result,
            "wss://proxy.daytona.io/sb-1/process/interpreter/execute"
        );
    }

    #[test]
    fn test_execution_result_default() {
        let result = ExecutionResult::default();
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_extract_host_with_port() {
        assert_eq!(extract_host("http://localhost:8080/path"), "localhost:8080");
    }

    #[test]
    fn test_extract_host_without_port() {
        assert_eq!(extract_host("https://proxy.daytona.io/path"), "proxy.daytona.io");
    }
}