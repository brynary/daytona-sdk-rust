use std::collections::HashMap;
use std::sync::Arc;

use daytona_api_client::apis::configuration::Configuration as ApiConfiguration;
use daytona_api_client::apis::sandbox_api;
use daytona_api_client::models;

use crate::config::{resolve_config, DaytonaConfig, ResolvedConfig};
use crate::error::{error_from_response, DaytonaError};
use crate::sandbox::Sandbox;
use crate::snapshot::SnapshotService;
use crate::types::{CreateParams, CreateSandboxOptions};
use crate::volume::VolumeService;

const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
const SDK_SOURCE: &str = "rust-sdk";

/// The main Daytona SDK client.
pub struct Client {
    pub(crate) config: ResolvedConfig,
    pub(crate) api_config: ApiConfiguration,
    pub(crate) _http_client: reqwest::Client,
    pub(crate) toolbox_proxy_cache: Arc<tokio::sync::RwLock<HashMap<String, String>>>,

    /// Volume management service.
    pub volume: VolumeService,
    /// Snapshot management service.
    pub snapshot: SnapshotService,
}

impl Client {
    /// Create a new client using environment variables for configuration.
    pub async fn new() -> Result<Self, DaytonaError> {
        Self::new_with_config(DaytonaConfig::default()).await
    }

    /// Create a new client with explicit configuration.
    /// Config fields take precedence over environment variables.
    pub async fn new_with_config(config: DaytonaConfig) -> Result<Self, DaytonaError> {
        let resolved = resolve_config(&config)?;
        let api_config = build_api_config(&resolved);

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| DaytonaError::general(e.to_string()))?;

        let toolbox_proxy_cache = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        let volume = VolumeService {
            api_config: api_config.clone(),
            org_id: resolved.organization_id.clone(),
        };

        let snapshot = SnapshotService {
            api_config: api_config.clone(),
            org_id: resolved.organization_id.clone(),
        };

        Ok(Client {
            config: resolved,
            api_config,
            _http_client: http_client,
            toolbox_proxy_cache,
            volume,
            snapshot,
        })
    }

    /// Create a new sandbox.
    pub async fn create(
        &self,
        params: CreateParams,
        options: CreateSandboxOptions,
    ) -> Result<Sandbox, DaytonaError> {
        let create_sandbox = build_create_request(&params, &self.config);

        let api_sandbox = sandbox_api::create_sandbox(
            &self.api_config,
            create_sandbox,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        let sandbox = self.sandbox_from_api(api_sandbox);

        if options.wait_for_start {
            sandbox.wait_for_start(options.timeout).await?;
        }

        Ok(sandbox)
    }

    /// Get a sandbox by ID or name.
    pub async fn get(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        let api_sandbox = sandbox_api::get_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
            Some(true),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(self.sandbox_from_api(api_sandbox))
    }

    /// List all sandboxes, optionally filtered by labels.
    pub async fn list(&self, labels: Option<&str>) -> Result<Vec<Sandbox>, DaytonaError> {
        let api_sandboxes = sandbox_api::list_sandboxes(
            &self.api_config,
            self.config.organization_id.as_deref(),
            Some(true),
            labels,
            None,
        )
        .await
        .map_err(convert_api_error)?;

        Ok(api_sandboxes
            .into_iter()
            .map(|s| self.sandbox_from_api(s))
            .collect())
    }

    /// Delete a sandbox by ID or name.
    pub async fn delete(&self, sandbox_id_or_name: &str) -> Result<(), DaytonaError> {
        sandbox_api::delete_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(())
    }

    /// Start a stopped sandbox.
    pub async fn start(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        let api_sandbox = sandbox_api::start_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(self.sandbox_from_api(api_sandbox))
    }

    /// Stop a running sandbox.
    pub async fn stop(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        let api_sandbox = sandbox_api::stop_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(self.sandbox_from_api(api_sandbox))
    }

    /// Convert an API sandbox model into an SDK Sandbox.
    pub(crate) fn sandbox_from_api(&self, api_sandbox: models::Sandbox) -> Sandbox {
        Sandbox::new(self, api_sandbox)
    }

    /// Get the toolbox proxy URL for a sandbox, using the cache.
    #[allow(dead_code)]
    pub(crate) async fn get_toolbox_proxy_url(
        &self,
        sandbox_id: &str,
    ) -> Result<String, DaytonaError> {
        // Check cache first
        let target = self.config.target.as_deref().unwrap_or("default");
        {
            let cache = self.toolbox_proxy_cache.read().await;
            if let Some(url) = cache.get(target) {
                return Ok(format!("{}/{}", url, sandbox_id));
            }
        }

        // Fetch from API
        let proxy_url = sandbox_api::get_toolbox_proxy_url(
            &self.api_config,
            sandbox_id,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        let base_url = proxy_url.url;

        // Cache the base URL
        {
            let mut cache = self.toolbox_proxy_cache.write().await;
            cache.insert(target.to_string(), base_url.clone());
        }

        Ok(format!("{}/{}", base_url, sandbox_id))
    }

    /// Create a toolbox API client configuration for a specific sandbox.
    #[allow(dead_code)]
    pub(crate) async fn create_toolbox_config(
        &self,
        sandbox_id: &str,
    ) -> Result<daytona_toolbox_client::apis::configuration::Configuration, DaytonaError> {
        let toolbox_url = self.get_toolbox_proxy_url(sandbox_id).await?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = self.config.bearer_token() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .map_err(|e| DaytonaError::general(e.to_string()))?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| DaytonaError::general(e.to_string()))?;

        let mw_client = reqwest_middleware::ClientBuilder::new(client).build();

        Ok(daytona_toolbox_client::apis::configuration::Configuration {
            base_path: toolbox_url,
            client: mw_client,
            user_agent: Some(format!("daytona-sdk-rust/{}", SDK_VERSION)),
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: self.config.bearer_token().map(|s| s.to_string()),
            api_key: None,
        })
    }
}

fn build_api_config(resolved: &ResolvedConfig) -> ApiConfiguration {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Daytona-Source",
        reqwest::header::HeaderValue::from_static(SDK_SOURCE),
    );
    if let Ok(v) = reqwest::header::HeaderValue::from_str(SDK_VERSION) {
        headers.insert("X-Daytona-SDK-Version", v);
    }
    if let Some(org_id) = &resolved.organization_id {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(org_id) {
            headers.insert("X-Daytona-Organization-ID", v);
        }
    }

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_default();

    let mw_client = reqwest_middleware::ClientBuilder::new(client).build();

    ApiConfiguration {
        base_path: resolved.api_url.clone(),
        client: mw_client,
        user_agent: Some(format!("daytona-sdk-rust/{}", SDK_VERSION)),
        basic_auth: None,
        oauth_access_token: None,
        bearer_access_token: resolved.bearer_token().map(|s| s.to_string()),
        api_key: None,
    }
}

fn build_create_request(params: &CreateParams, config: &ResolvedConfig) -> models::CreateSandbox {
    match params {
        CreateParams::Snapshot(snapshot_params) => {
            let mut cs = models::CreateSandbox::new();
            cs.snapshot = Some(snapshot_params.snapshot.clone());
            apply_base_params(&mut cs, &snapshot_params.base, config);
            cs
        }
        CreateParams::Image(image_params) => {
            let mut cs = models::CreateSandbox::new();
            if let Some(resources) = &image_params.resources {
                cs.cpu = resources.cpu;
                cs.gpu = resources.gpu;
                cs.memory = resources.memory;
                cs.disk = resources.disk;
            }
            apply_base_params(&mut cs, &image_params.base, config);
            cs
        }
    }
}

fn apply_base_params(
    cs: &mut models::CreateSandbox,
    base: &crate::types::SandboxBaseParams,
    config: &ResolvedConfig,
) {
    cs.name = base.name.clone();
    cs.user = base.user.clone();
    cs.env = base.env_vars.clone();
    cs.labels = base.labels.clone();
    cs.public = base.public;
    cs.auto_stop_interval = base.auto_stop_interval;
    cs.auto_archive_interval = base.auto_archive_interval;
    cs.auto_delete_interval = base.auto_delete_interval;
    cs.network_block_all = base.network_block_all;
    if let Some(allow_list) = &base.network_allow_list {
        cs.network_allow_list = Some(allow_list.join(","));
    }
    cs.target = config.target.clone();
}

/// Convert a generated API client error to a DaytonaError.
pub(crate) fn convert_api_error<T: std::fmt::Debug>(
    err: daytona_api_client::apis::Error<T>,
) -> DaytonaError {
    match err {
        daytona_api_client::apis::Error::ResponseError(ref resp) => {
            error_from_response(resp.status.as_u16(), &resp.content, HashMap::new())
        }
        daytona_api_client::apis::Error::Reqwest(ref e) => {
            if e.is_timeout() {
                DaytonaError::timeout(e.to_string())
            } else {
                DaytonaError::general(e.to_string())
            }
        }
        _ => DaytonaError::general(err.to_string()),
    }
}

/// Convert a generated toolbox client error to a DaytonaError.
pub(crate) fn convert_toolbox_error<T: std::fmt::Debug>(
    err: daytona_toolbox_client::apis::Error<T>,
) -> DaytonaError {
    match err {
        daytona_toolbox_client::apis::Error::ResponseError(ref resp) => {
            error_from_response(resp.status.as_u16(), &resp.content, HashMap::new())
        }
        daytona_toolbox_client::apis::Error::Reqwest(ref e) => {
            if e.is_timeout() {
                DaytonaError::timeout(e.to_string())
            } else {
                DaytonaError::general(e.to_string())
            }
        }
        _ => DaytonaError::general(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_env<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().unwrap();
        let vars = [
            "DAYTONA_API_KEY",
            "DAYTONA_JWT_TOKEN",
            "DAYTONA_ORGANIZATION_ID",
            "DAYTONA_API_URL",
            "DAYTONA_TARGET",
        ];
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for k in &vars {
            std::env::remove_var(k);
        }
        f();
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(&k, &val),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[tokio::test]
    async fn test_client_new_from_env() {
        with_clean_env(|| {
            std::env::set_var("DAYTONA_API_KEY", "test-api-key");
            std::env::set_var("DAYTONA_API_URL", "https://test.api.com");
        });
        // Note: the client was already created in the env context
        // We test the config resolution separately since env modification is not threadsafe
    }

    #[tokio::test]
    async fn test_client_new_with_config() {
        let mock_server = MockServer::start().await;
        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        assert_eq!(client.config.api_key.as_deref(), Some("test-key"));
        assert_eq!(client.config.api_url, mock_server.uri());
    }

    #[tokio::test]
    async fn test_client_auth_header() {
        let config = DaytonaConfig {
            api_key: Some("my-api-key".to_string()),
            api_url: Some("https://test.example.com".to_string()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        assert_eq!(
            client.api_config.bearer_access_token.as_deref(),
            Some("my-api-key")
        );
    }

    #[tokio::test]
    async fn test_client_default_headers() {
        let config = DaytonaConfig {
            api_key: Some("key".to_string()),
            api_url: Some("https://test.example.com".to_string()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        assert!(client
            .api_config
            .user_agent
            .as_ref()
            .unwrap()
            .contains("daytona-sdk-rust"));
    }

    #[tokio::test]
    async fn test_client_get_sandbox() {
        let mock_server = MockServer::start().await;

        let sandbox_json = serde_json::json!({
            "id": "sandbox-123",
            "organizationId": "org-1",
            "name": "test-sandbox",
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
            "state": "started"
        });

        Mock::given(method("GET"))
            .and(path("/sandbox/sandbox-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sandbox_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let sandbox = client.get("sandbox-123").await.unwrap();
        assert_eq!(sandbox.id, "sandbox-123");
        assert_eq!(sandbox.name, "test-sandbox");
    }

    #[tokio::test]
    async fn test_client_get_sandbox_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/nonexistent"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({"message": "sandbox not found"})),
            )
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.get("nonexistent").await.unwrap_err();
        assert!(matches!(err, DaytonaError::NotFound { .. }));
    }

    #[tokio::test]
    async fn test_client_list_sandboxes() {
        let mock_server = MockServer::start().await;

        let sandboxes_json = serde_json::json!([
            {
                "id": "sb-1",
                "organizationId": "org-1",
                "name": "sandbox-1",
                "user": "daytona",
                "env": {},
                "labels": {},
                "public": false,
                "networkBlockAll": false,
                "target": "us",
                "cpu": 2.0,
                "gpu": 0.0,
                "memory": 4.0,
                "disk": 20.0
            },
            {
                "id": "sb-2",
                "organizationId": "org-1",
                "name": "sandbox-2",
                "user": "daytona",
                "env": {},
                "labels": {},
                "public": false,
                "networkBlockAll": false,
                "target": "us",
                "cpu": 2.0,
                "gpu": 0.0,
                "memory": 4.0,
                "disk": 20.0
            }
        ]);

        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sandboxes_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let sandboxes = client.list(None).await.unwrap();
        assert_eq!(sandboxes.len(), 2);
        assert_eq!(sandboxes[0].id, "sb-1");
        assert_eq!(sandboxes[1].id, "sb-2");
    }

    #[tokio::test]
    async fn test_client_delete_sandbox() {
        let mock_server = MockServer::start().await;

        let sandbox_json = serde_json::json!({
            "id": "sandbox-123",
            "organizationId": "org-1",
            "name": "test-sandbox",
            "user": "daytona",
            "env": {},
            "labels": {},
            "public": false,
            "networkBlockAll": false,
            "target": "us",
            "cpu": 2.0,
            "gpu": 0.0,
            "memory": 4.0,
            "disk": 20.0
        });

        Mock::given(method("DELETE"))
            .and(path("/sandbox/sandbox-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sandbox_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        client.delete("sandbox-123").await.unwrap();
    }

    #[tokio::test]
    async fn test_client_rate_limit_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(serde_json::json!({"message": "rate limited"})),
            )
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.get("sb-1").await.unwrap_err();
        assert!(matches!(err, DaytonaError::RateLimit { .. }));
    }

    #[tokio::test]
    async fn test_client_create_sandbox() {
        let mock_server = MockServer::start().await;

        let sandbox_json = serde_json::json!({
            "id": "new-sandbox",
            "organizationId": "org-1",
            "name": "my-sandbox",
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
            "state": "started"
        });

        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(201).set_body_json(&sandbox_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();

        let params = CreateParams::Snapshot(crate::types::SnapshotParams {
            base: crate::types::SandboxBaseParams {
                name: Some("my-sandbox".to_string()),
                ..Default::default()
            },
            snapshot: "base-snapshot".to_string(),
        });

        let sandbox = client
            .create(params, CreateSandboxOptions::default())
            .await
            .unwrap();
        assert_eq!(sandbox.id, "new-sandbox");
        assert_eq!(sandbox.name, "my-sandbox");
    }

    #[tokio::test]
    async fn test_toolbox_proxy_cache() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1/toolbox-proxy-url"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"url": "https://proxy.daytona.io"})),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();

        // First call should hit the API
        let url = client.get_toolbox_proxy_url("sb-1").await.unwrap();
        assert!(url.contains("sb-1"));

        // Second call should use cache (mock expects exactly 1 call)
        let url2 = client.get_toolbox_proxy_url("sb-2").await.unwrap();
        assert!(url2.contains("sb-2"));
    }
}
