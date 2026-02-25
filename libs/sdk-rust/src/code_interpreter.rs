use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::interpreter_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;

/// Service for code interpreter operations in a sandbox.
pub struct CodeInterpreterService {
    pub(crate) config: ToolboxConfig,
}

impl CodeInterpreterService {
    /// Create a new interpreter context.
    pub async fn create_context(
        &self,
        language: &str,
    ) -> Result<daytona_toolbox_client::models::InterpreterContext, DaytonaError> {
        let req = daytona_toolbox_client::models::CreateContextRequest {
            language: Some(language.to_string()),
            cwd: None,
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
        let ctx = svc.create_context("python").await.unwrap();
        assert_eq!(ctx.id, "ctx-123");
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
}
