use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::process_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;
use crate::types::ExecuteCommandOptions;
use crate::types::ExecuteResponse;

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
    pub async fn execute_session_command(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<String, DaytonaError> {
        let req = daytona_toolbox_client::models::SessionExecuteRequest {
            command: command.to_string(),
            r#async: Some(false),
            run_async: None,
            suppress_input_echo: None,
        };

        let result = process_api::session_execute_command(&self.config, session_id, req)
            .await
            .map_err(convert_toolbox_error)?;

        Ok(result.cmd_id)
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
    pub async fn create_pty_session(
        &self,
        options: crate::types::PtySessionOptions,
    ) -> Result<String, DaytonaError> {
        let req = daytona_toolbox_client::models::PtyCreateRequest {
            cols: options.size.as_ref().map(|s| s.cols as i32),
            rows: options.size.as_ref().map(|s| s.rows as i32),
            cwd: None,
            envs: options.env,
            id: None,
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
        let cmd_id = svc
            .execute_session_command("sess-1", "ls -la")
            .await
            .unwrap();
        assert_eq!(cmd_id, "cmd-abc");
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
            .create_pty_session(crate::types::PtySessionOptions::default())
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
        let session_id = svc.create_pty_session(opts).await.unwrap();
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
}
