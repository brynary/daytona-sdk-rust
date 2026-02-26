use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::process_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;
use crate::types::CodeLanguage;
use crate::types::ExecuteCommandOptions;
use crate::types::ExecuteResponse;

/// Result of executing a session command.
#[derive(Debug, Clone)]
pub struct SessionExecuteResult {
    /// The command identifier.
    pub cmd_id: String,
    /// Exit code, present if the command completed synchronously.
    pub exit_code: Option<i32>,
    /// Standard output, present if the command completed synchronously.
    pub stdout: Option<String>,
    /// Standard error, present if the command completed synchronously.
    pub stderr: Option<String>,
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
        follow: Option<bool>,
    ) -> Result<String, DaytonaError> {
        let logs =
            process_api::get_session_command_logs(&self.config, session_id, command_id, follow)
                .await
                .map_err(convert_toolbox_error)?;
        Ok(logs)
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
            id: if id.is_empty() { None } else { Some(id.to_string()) },
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

/// Shell-escape a string for use in a command, wrapping in single quotes.
fn shell_escape(s: &str) -> String {
    // Replace single quotes with '\'' (end quote, escaped quote, start quote)
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
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
        assert_eq!(result.stdout.as_deref(), Some("hello\n"));
    }

    #[tokio::test]
    async fn test_get_session_command_logs() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/process/session/sess-1/command/cmd-1/logs"))
            .respond_with(ResponseTemplate::new(200).set_body_string("line 1\nline 2\n"))
            .mount(&mock_server)
            .await;

        let svc = process_service(&mock_server).await;
        let logs = svc
            .get_session_command_logs("sess-1", "cmd-1", None)
            .await
            .unwrap();
        assert!(logs.contains("line 1"));
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
        let sessions = svc.list_pty_sessions().await.unwrap();
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