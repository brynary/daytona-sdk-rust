use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::lsp_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;

/// Service for Language Server Protocol (LSP) operations within a sandbox.
///
/// Provides IDE-like features such as code completion, symbol search, and
/// document analysis. The service manages a language server instance for a
/// specific language and project path.
///
/// Matches Go SDK `LspServerService` and TypeScript SDK `LspServer`.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example(sandbox: &daytona_sdk::Sandbox) -> Result<(), daytona_sdk::DaytonaError> {
/// let lsp = sandbox.lsp("python", "/home/daytona/project").await?;
/// lsp.start().await?;
/// lsp.did_open("/home/daytona/project/main.py").await?;
/// let symbols = lsp.document_symbols("/home/daytona/project/main.py").await?;
/// lsp.did_close("/home/daytona/project/main.py").await?;
/// lsp.stop().await?;
/// # Ok(())
/// # }
/// ```
pub struct LspService {
    pub(crate) config: ToolboxConfig,
    pub(crate) language_id: String,
    pub(crate) project_path: String,
}

impl LspService {
    /// Start the language server.
    ///
    /// Must be called before using other LSP operations. Call [`LspService::stop`]
    /// when finished to release resources.
    pub async fn start(&self) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::LspServerRequest {
            language_id: self.language_id.clone(),
            path_to_project: self.project_path.clone(),
        };
        lsp_api::start(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Stop the language server and release resources.
    pub async fn stop(&self) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::LspServerRequest {
            language_id: self.language_id.clone(),
            path_to_project: self.project_path.clone(),
        };
        lsp_api::stop(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Notify the language server that a file was opened.
    ///
    /// Should be called before requesting completions or symbols for a file.
    /// The path is automatically converted to a `file://` URI if needed
    /// (matching Go SDK behavior).
    pub async fn did_open(&self, path: &str) -> Result<(), DaytonaError> {
        let uri = ensure_file_uri(path);
        let req = daytona_toolbox_client::models::LspDocumentRequest {
            language_id: self.language_id.clone(),
            path_to_project: self.project_path.clone(),
            uri,
        };
        lsp_api::did_open(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Notify the language server that a file was closed.
    ///
    /// Call this when done working with a file to release server resources.
    pub async fn did_close(&self, path: &str) -> Result<(), DaytonaError> {
        let uri = ensure_file_uri(path);
        let req = daytona_toolbox_client::models::LspDocumentRequest {
            language_id: self.language_id.clone(),
            path_to_project: self.project_path.clone(),
            uri,
        };
        lsp_api::did_close(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Get all symbols (functions, classes, variables) in a document.
    pub async fn document_symbols(
        &self,
        path: &str,
    ) -> Result<Vec<daytona_toolbox_client::models::LspSymbol>, DaytonaError> {
        let uri = ensure_file_uri(path);
        let symbols = lsp_api::document_symbols(
            &self.config,
            &self.language_id,
            &self.project_path,
            &uri,
        )
        .await
        .map_err(convert_toolbox_error)?;
        Ok(symbols)
    }

    /// Search for symbols across the entire workspace.
    ///
    /// Use this to find symbols (functions, classes, etc.) by name across all
    /// files in the project.
    pub async fn workspace_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<daytona_toolbox_client::models::LspSymbol>, DaytonaError> {
        let symbols = lsp_api::workspace_symbols(
            &self.config,
            query,
            &self.language_id,
            &self.project_path,
        )
        .await
        .map_err(convert_toolbox_error)?;
        Ok(symbols)
    }

    /// Get code completion suggestions at a position.
    ///
    /// The file should be opened with [`LspService::did_open`] before
    /// requesting completions.
    pub async fn completions(
        &self,
        path: &str,
        line: i32,
        character: i32,
    ) -> Result<daytona_toolbox_client::models::CompletionList, DaytonaError> {
        let uri = ensure_file_uri(path);
        let req = daytona_toolbox_client::models::LspCompletionParams {
            language_id: self.language_id.clone(),
            path_to_project: self.project_path.clone(),
            position: Box::new(daytona_toolbox_client::models::LspPosition {
                line,
                character,
            }),
            uri,
            context: None,
        };
        let completions = lsp_api::completions(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(completions)
    }
}

/// Ensure a path has a `file://` prefix, matching Go/TS SDK behavior.
fn ensure_file_uri(path: &str) -> String {
    if path.starts_with("file://") {
        path.to_string()
    } else {
        format!("file://{}", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn lsp_service(mock_server: &MockServer) -> LspService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        LspService {
            config,
            language_id: "python".to_string(),
            project_path: "/home/daytona/project".to_string(),
        }
    }

    #[tokio::test]
    async fn test_lsp_start_stop() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/lsp/start"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/lsp/stop"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = lsp_service(&mock_server).await;
        svc.start().await.unwrap();
        svc.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_lsp_did_open_close() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/lsp/did-open"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/lsp/did-close"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = lsp_service(&mock_server).await;
        svc.did_open("/home/daytona/project/main.py").await.unwrap();
        svc.did_close("/home/daytona/project/main.py")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_lsp_document_symbols() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/lsp/document-symbols"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "main", "kind": 12, "location": {"uri": "file:///main.py", "range": {"start": {"line": 0, "character": 0}, "end": {"line": 10, "character": 0}}}}
            ])))
            .mount(&mock_server)
            .await;

        let svc = lsp_service(&mock_server).await;
        let symbols = svc
            .document_symbols("/home/daytona/project/main.py")
            .await
            .unwrap();
        assert_eq!(symbols.len(), 1);
    }

    #[tokio::test]
    async fn test_lsp_workspace_symbols() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/lsp/workspacesymbols"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "MyClass", "kind": 5, "location": {"uri": "file:///main.py", "range": {"start": {"line": 5, "character": 0}, "end": {"line": 20, "character": 0}}}}
            ])))
            .mount(&mock_server)
            .await;

        let svc = lsp_service(&mock_server).await;
        let symbols = svc.workspace_symbols("MyClass").await.unwrap();
        assert_eq!(symbols.len(), 1);
    }

    #[tokio::test]
    async fn test_lsp_completions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/lsp/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "isIncomplete": false,
                "items": [
                    {"label": "print", "kind": 3, "detail": "built-in function"}
                ]
            })))
            .mount(&mock_server)
            .await;

        let svc = lsp_service(&mock_server).await;
        let completions = svc
            .completions("/home/daytona/project/main.py", 10, 5)
            .await
            .unwrap();
        assert!(!completions.items.is_empty());
    }

    #[test]
    fn test_ensure_file_uri() {
        assert_eq!(
            ensure_file_uri("/home/user/file.py"),
            "file:///home/user/file.py"
        );
        assert_eq!(
            ensure_file_uri("file:///already/prefixed"),
            "file:///already/prefixed"
        );
    }
}