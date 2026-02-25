use std::collections::HashMap;
use std::time::Duration;

use daytona_api_client::apis::sandbox_api;
use daytona_api_client::models;

use crate::client::{convert_api_error, Client};
use crate::code_interpreter::CodeInterpreterService;
use crate::computer_use::ComputerUseService;
use crate::error::DaytonaError;
use crate::filesystem::FileSystemService;
use crate::git::GitService;
use crate::process::ProcessService;
use crate::types::PreviewLink;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A Daytona sandbox with lifecycle and service access.
#[derive(Debug)]
pub struct Sandbox {
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub target: String,
    pub auto_archive_interval: Option<f64>,
    pub auto_delete_interval: Option<f64>,
    pub network_block_all: bool,
    pub network_allow_list: Option<String>,

    pub(crate) api_config: daytona_api_client::apis::configuration::Configuration,
    pub(crate) org_id: Option<String>,
    pub(crate) client: ClientRef,
}

/// Reference to the parent client, to create toolbox configurations.
#[derive(Debug)]
pub(crate) enum ClientRef {
    Borrowed {
        toolbox_proxy_cache: std::sync::Arc<tokio::sync::RwLock<HashMap<String, String>>>,
        config: crate::config::ResolvedConfig,
    },
}

impl Sandbox {
    pub(crate) fn new(client: &Client, api_sandbox: models::Sandbox) -> Self {
        let state = api_sandbox.state.map(|s| format!("{:?}", s));

        Sandbox {
            id: api_sandbox.id,
            name: api_sandbox.name,
            state,
            target: api_sandbox.target,
            auto_archive_interval: api_sandbox.auto_archive_interval,
            auto_delete_interval: api_sandbox.auto_delete_interval,
            network_block_all: api_sandbox.network_block_all,
            network_allow_list: api_sandbox.network_allow_list,
            api_config: client.api_config.clone(),
            org_id: client.config.organization_id.clone(),
            client: ClientRef::Borrowed {
                toolbox_proxy_cache: client.toolbox_proxy_cache.clone(),
                config: client.config.clone(),
            },
        }
    }

    /// Start the sandbox.
    pub async fn start(&self) -> Result<(), DaytonaError> {
        sandbox_api::start_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Stop the sandbox.
    pub async fn stop(&self) -> Result<(), DaytonaError> {
        sandbox_api::stop_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Delete the sandbox.
    pub async fn delete(&self) -> Result<(), DaytonaError> {
        sandbox_api::delete_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Archive the sandbox.
    pub async fn archive(&self) -> Result<(), DaytonaError> {
        sandbox_api::archive_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Wait for the sandbox to reach the "started" state.
    pub async fn wait_for_start(&self, timeout: Option<Duration>) -> Result<(), DaytonaError> {
        self.wait_for_state("Started", timeout).await
    }

    /// Wait for the sandbox to reach the "stopped" state.
    pub async fn wait_for_stop(&self, timeout: Option<Duration>) -> Result<(), DaytonaError> {
        self.wait_for_state("Stopped", timeout).await
    }

    /// Resize the sandbox resources.
    pub async fn resize(
        &self,
        cpu: Option<i32>,
        memory: Option<i32>,
        disk: Option<i32>,
    ) -> Result<(), DaytonaError> {
        let resize = models::ResizeSandbox { cpu, memory, disk };
        sandbox_api::resize_sandbox(&self.api_config, &self.id, resize, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Set labels on the sandbox.
    pub async fn set_labels(
        &self,
        labels: HashMap<String, String>,
    ) -> Result<HashMap<String, String>, DaytonaError> {
        let sandbox_labels = models::SandboxLabels { labels };
        let result = sandbox_api::replace_labels(
            &self.api_config,
            &self.id,
            sandbox_labels,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;
        Ok(result.labels)
    }

    /// Get a preview link for a port on this sandbox.
    pub async fn get_preview_link(&self, port: f64) -> Result<PreviewLink, DaytonaError> {
        let preview = sandbox_api::get_port_preview_url(
            &self.api_config,
            &self.id,
            port,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(PreviewLink {
            url: preview.url,
            token: preview.token,
        })
    }

    /// Refresh sandbox data from the API.
    pub async fn refresh_data(&mut self) -> Result<(), DaytonaError> {
        let api_sandbox = sandbox_api::get_sandbox(
            &self.api_config,
            &self.id,
            self.org_id.as_deref(),
            Some(true),
        )
        .await
        .map_err(convert_api_error)?;

        self.name = api_sandbox.name;
        self.state = api_sandbox.state.map(|s| format!("{:?}", s));
        self.target = api_sandbox.target;
        self.auto_archive_interval = api_sandbox.auto_archive_interval;
        self.auto_delete_interval = api_sandbox.auto_delete_interval;
        self.network_block_all = api_sandbox.network_block_all;
        self.network_allow_list = api_sandbox.network_allow_list;

        Ok(())
    }

    /// Get the FileSystemService for this sandbox.
    pub async fn filesystem(&self) -> Result<FileSystemService, DaytonaError> {
        let toolbox_config = self.create_toolbox_config().await?;
        Ok(FileSystemService {
            config: toolbox_config,
        })
    }

    /// Get the GitService for this sandbox.
    pub async fn git(&self) -> Result<GitService, DaytonaError> {
        let toolbox_config = self.create_toolbox_config().await?;
        Ok(GitService {
            config: toolbox_config,
        })
    }

    /// Get the ProcessService for this sandbox.
    pub async fn process(&self) -> Result<ProcessService, DaytonaError> {
        let toolbox_config = self.create_toolbox_config().await?;
        Ok(ProcessService {
            config: toolbox_config,
        })
    }

    /// Get the CodeInterpreterService for this sandbox.
    pub async fn code_interpreter(&self) -> Result<CodeInterpreterService, DaytonaError> {
        let toolbox_config = self.create_toolbox_config().await?;
        Ok(CodeInterpreterService {
            config: toolbox_config,
        })
    }

    /// Get the ComputerUseService for this sandbox.
    pub async fn computer_use(&self) -> Result<ComputerUseService, DaytonaError> {
        let toolbox_config = self.create_toolbox_config().await?;
        Ok(ComputerUseService {
            config: toolbox_config,
        })
    }

    async fn wait_for_state(
        &self,
        target_state: &str,
        timeout: Option<Duration>,
    ) -> Result<(), DaytonaError> {
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(DaytonaError::timeout(format!(
                        "timed out waiting for sandbox {} to reach state {}",
                        self.id, target_state,
                    )));
                }
            }

            let api_sandbox =
                sandbox_api::get_sandbox(&self.api_config, &self.id, self.org_id.as_deref(), None)
                    .await
                    .map_err(convert_api_error)?;

            if let Some(state) = &api_sandbox.state {
                let state_str = format!("{:?}", state);
                if state_str == target_state {
                    return Ok(());
                }
                // Check for error state
                if state_str.contains("Error") {
                    return Err(DaytonaError::general(format!(
                        "sandbox {} entered error state: {}",
                        self.id,
                        api_sandbox.error_reason.unwrap_or_default(),
                    )));
                }
            }

            tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
        }
    }

    async fn create_toolbox_config(
        &self,
    ) -> Result<daytona_toolbox_client::apis::configuration::Configuration, DaytonaError> {
        match &self.client {
            ClientRef::Borrowed {
                toolbox_proxy_cache,
                config,
            } => {
                let sandbox_id = &self.id;
                let target = config.target.as_deref().unwrap_or("default");

                // Check cache
                let base_url = {
                    let cache = toolbox_proxy_cache.read().await;
                    cache.get(target).cloned()
                };

                let toolbox_url = if let Some(base) = base_url {
                    format!("{}/{}", base, sandbox_id)
                } else {
                    // Fetch from API
                    let proxy_url = sandbox_api::get_toolbox_proxy_url(
                        &self.api_config,
                        sandbox_id,
                        self.org_id.as_deref(),
                    )
                    .await
                    .map_err(convert_api_error)?;

                    let base = proxy_url.url.clone();
                    {
                        let mut cache = toolbox_proxy_cache.write().await;
                        cache.insert(target.to_string(), base.clone());
                    }
                    format!("{}/{}", base, sandbox_id)
                };

                let mut headers = reqwest::header::HeaderMap::new();
                if let Some(token) = config.bearer_token() {
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                            .map_err(|e| DaytonaError::general(e.to_string()))?,
                    );
                }

                let client = reqwest::Client::builder()
                    .default_headers(headers)
                    .timeout(Duration::from_secs(60))
                    .build()
                    .map_err(|e| DaytonaError::general(e.to_string()))?;

                let mw_client = reqwest_middleware::ClientBuilder::new(client).build();

                Ok(daytona_toolbox_client::apis::configuration::Configuration {
                    base_path: toolbox_url,
                    client: mw_client,
                    user_agent: Some(format!("daytona-sdk-rust/{}", env!("CARGO_PKG_VERSION"))),
                    basic_auth: None,
                    oauth_access_token: None,
                    bearer_access_token: config.bearer_token().map(|s| s.to_string()),
                    api_key: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sandbox_json(id: &str, state: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "organizationId": "org-1",
            "name": format!("sandbox-{}", id),
            "user": "daytona",
            "env": {},
            "labels": {},
            "public": false,
            "networkBlockAll": false,
            "target": "us",
            "cpu": 2.0,
            "gpu": 0.0,
            "memory": 4.0,
            "disk": 20.0,
            "state": state
        })
    }

    async fn test_client(mock_server: &MockServer) -> Client {
        let config = crate::config::DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        Client::new_with_config(config).await.unwrap()
    }

    #[tokio::test]
    async fn test_sandbox_start() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/start"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "starting")),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopped")).unwrap());
        sandbox.start().await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_stop() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/stop"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopping")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_delete() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/sandbox/sb-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "destroyed")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.delete().await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_archive() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/archive"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "archiving")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.archive().await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_set_labels() {
        let mock_server = MockServer::start().await;

        let labels_response = serde_json::json!({
            "labels": {"env": "prod", "team": "backend"}
        });

        Mock::given(method("PUT"))
            .and(path("/sandbox/sb-1/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&labels_response))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        labels.insert("team".to_string(), "backend".to_string());

        let result = sandbox.set_labels(labels).await.unwrap();
        assert_eq!(result.get("env").unwrap(), "prod");
    }

    #[tokio::test]
    async fn test_sandbox_refresh_data() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopped")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        sandbox.refresh_data().await.unwrap();
        // After refresh, state should reflect the API response
        assert!(sandbox.state.is_some());
    }

    #[tokio::test]
    async fn test_sandbox_wait_for_start_timeout() {
        let mock_server = MockServer::start().await;

        // Always return "stopped" state so the wait times out
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopped")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopped")).unwrap());

        let err = sandbox
            .wait_for_start(Some(Duration::from_millis(100)))
            .await
            .unwrap_err();
        assert!(matches!(err, DaytonaError::Timeout { .. }));
    }

    #[tokio::test]
    async fn test_sandbox_resize() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/resize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        sandbox.resize(Some(4), Some(8), Some(40)).await.unwrap();
    }
}
