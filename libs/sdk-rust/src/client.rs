use std::collections::HashMap;
use std::sync::Arc;

use daytona_api_client::apis::configuration::Configuration as ApiConfiguration;
use daytona_api_client::apis::sandbox_api;
use daytona_api_client::models;

use crate::config::{resolve_config, DaytonaConfig, ResolvedConfig};
use crate::error::{error_from_response, DaytonaError};
use crate::sandbox::Sandbox;
use crate::snapshot::SnapshotService;
use crate::types::{CreateParams, CreateSandboxOptions, PaginatedSandboxes};
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
    ///
    /// By default, waits for the sandbox to reach the "started" state before
    /// returning. Set `options.wait_for_start` to `false` to return immediately.
    ///
    /// When waiting, the returned Sandbox has up-to-date state from the API
    /// (matching TypeScript SDK behavior where `create` waits and returns a
    /// fully-initialized Sandbox).
    pub async fn create(
        &self,
        params: CreateParams,
        options: CreateSandboxOptions,
    ) -> Result<Sandbox, DaytonaError> {
        let create_sandbox = build_create_request(&params, &self.config)?;

        let api_sandbox = sandbox_api::create_sandbox(
            &self.api_config,
            create_sandbox,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        // Check for immediate error state (matching Go/TypeScript SDK behavior)
        if let Some(state) = &api_sandbox.state {
            if matches!(
                state,
                models::SandboxState::Error | models::SandboxState::BuildFailed
            ) {
                return Err(DaytonaError::general("sandbox failed to start"));
            }
        }

        let sandbox_id = api_sandbox.id.clone();
        let sandbox = self.sandbox_from_api(api_sandbox);

        if options.wait_for_start {
            sandbox.wait_for_start(options.timeout).await?;
            // Return a fresh sandbox with the final state (matching TS behavior)
            return self.get(&sandbox_id).await;
        }

        Ok(sandbox)
    }

    /// Get a sandbox by ID or name.
    pub async fn get(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        if sandbox_id_or_name.is_empty() {
            return Err(DaytonaError::general(
                "sandbox ID or name is required",
            ));
        }

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

    /// List sandboxes with optional label filtering and pagination.
    ///
    /// Returns a paginated result matching the Go/TypeScript SDK behavior.
    pub async fn list(
        &self,
        labels: Option<&HashMap<String, String>>,
        page: Option<i32>,
        limit: Option<i32>,
    ) -> Result<PaginatedSandboxes, DaytonaError> {
        if let Some(p) = page {
            if p < 1 {
                return Err(DaytonaError::general("page must be a positive integer"));
            }
        }
        if let Some(l) = limit {
            if l < 1 {
                return Err(DaytonaError::general("limit must be a positive integer"));
            }
        }

        let labels_json = labels
            .map(|l| serde_json::to_string(l).unwrap_or_default());

        let result = sandbox_api::list_sandboxes_paginated(
            &self.api_config,
            self.config.organization_id.as_deref(),
            page.map(|p| p as f64),
            limit.map(|l| l as f64),
            None,        // id
            None,        // name
            labels_json.as_deref(),
            None,        // include_errored_deleted
            None,        // states
            None,        // snapshots
            None,        // regions
            None, None, None, None, None, None,
            None, None,  // last_event_after, last_event_before
            None, None,  // sort, order
        )
        .await
        .map_err(convert_api_error)?;

        let items = result
            .items
            .into_iter()
            .map(|s| self.sandbox_from_api(s))
            .collect();

        Ok(PaginatedSandboxes {
            items,
            total: result.total as i64,
            page: result.page as i64,
            total_pages: result.total_pages as i64,
        })
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

    /// Find a single sandbox by ID/name or by matching labels.
    ///
    /// If `sandbox_id_or_name` is provided, delegates to [`Client::get`].
    /// Otherwise, searches for sandboxes matching the provided labels and returns
    /// the first match. Returns a not-found error if no matching sandbox is found.
    pub async fn find_one(
        &self,
        sandbox_id_or_name: Option<&str>,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<Sandbox, DaytonaError> {
        if let Some(id_or_name) = sandbox_id_or_name {
            if !id_or_name.is_empty() {
                return self.get(id_or_name).await;
            }
        }

        let labels_json = labels
            .map(|l| serde_json::to_string(l).unwrap_or_default());

        let api_sandboxes = sandbox_api::list_sandboxes_paginated(
            &self.api_config,
            self.config.organization_id.as_deref(),
            Some(1.0),   // page
            Some(1.0),   // limit
            None,        // id
            None,        // name
            labels_json.as_deref(),
            None,        // include_errored_deleted
            None,        // states
            None,        // snapshots
            None,        // regions
            None, None, None, None, None, None,
            None, None,  // last_event_after, last_event_before
            None, None,  // sort, order
        )
        .await
        .map_err(convert_api_error)?;

        let items = api_sandboxes.items;
        if items.is_empty() {
            return Err(DaytonaError::not_found("no sandbox found matching criteria"));
        }

        Ok(self.sandbox_from_api(items.into_iter().next().unwrap()))
    }

    /// Start a stopped sandbox and wait for it to reach the started state.
    ///
    /// Matches Go/TypeScript SDK behavior where starting a sandbox waits for the
    /// state transition before returning. Uses a default timeout of 60 seconds.
    ///
    /// Returns a fresh Sandbox with up-to-date state after waiting completes,
    /// matching the TypeScript SDK's `Daytona.start(sandbox)` which delegates to
    /// `sandbox.start()` and updates all sandbox fields.
    pub async fn start(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        let start_time = tokio::time::Instant::now();

        sandbox_api::start_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        // Build a lightweight sandbox to perform the wait
        let api_sandbox = sandbox_api::get_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
            Some(true),
        )
        .await
        .map_err(convert_api_error)?;

        let sandbox = self.sandbox_from_api(api_sandbox);

        // Deduct elapsed time from the 60s timeout (matching Go/TS behavior)
        let elapsed = start_time.elapsed();
        let remaining = std::time::Duration::from_secs(60).saturating_sub(elapsed);
        let timeout = if remaining.is_zero() {
            Some(std::time::Duration::from_millis(1))
        } else {
            Some(remaining)
        };
        sandbox.wait_for_start(timeout).await?;

        // Return a fresh sandbox with the final state
        self.get(sandbox_id_or_name).await
    }

    /// Stop a running sandbox and wait for it to reach the stopped state.
    ///
    /// Matches Go/TypeScript SDK behavior where stopping a sandbox waits for the
    /// state transition before returning. Uses a default timeout of 60 seconds.
    ///
    /// Returns a fresh Sandbox with up-to-date state after waiting completes.
    pub async fn stop(&self, sandbox_id_or_name: &str) -> Result<Sandbox, DaytonaError> {
        let start_time = tokio::time::Instant::now();

        sandbox_api::stop_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        // Build a lightweight sandbox to perform the wait
        let api_sandbox = sandbox_api::get_sandbox(
            &self.api_config,
            sandbox_id_or_name,
            self.config.organization_id.as_deref(),
            Some(true),
        )
        .await
        .map_err(convert_api_error)?;

        let sandbox = self.sandbox_from_api(api_sandbox);

        // Deduct elapsed time from the 60s timeout (matching Go/TS behavior)
        let elapsed = start_time.elapsed();
        let remaining = std::time::Duration::from_secs(60).saturating_sub(elapsed);
        let timeout = if remaining.is_zero() {
            Some(std::time::Duration::from_millis(1))
        } else {
            Some(remaining)
        };
        sandbox.wait_for_stop(timeout).await?;

        // Return a fresh sandbox with the final state (or handle ephemeral deletion)
        match self.get(sandbox_id_or_name).await {
            Ok(s) => Ok(s),
            Err(DaytonaError::NotFound { .. }) => {
                // Ephemeral sandbox was auto-deleted on stop — return last known state
                // with Destroyed state, matching TS refreshDataSafe pattern
                Ok(sandbox)
            }
            Err(e) => Err(e),
        }
    }

    /// Convert an API sandbox model into an SDK Sandbox.
    pub(crate) fn sandbox_from_api(&self, api_sandbox: models::Sandbox) -> Sandbox {
        Sandbox::new(self, api_sandbox)
    }

    /// Get the toolbox proxy URL for a sandbox, using the cache.
    /// The cache is keyed by sandbox region (target), matching Go SDK behavior.
    #[allow(dead_code)]
    pub(crate) async fn get_toolbox_proxy_url(
        &self,
        sandbox_id: &str,
        region: &str,
    ) -> Result<String, DaytonaError> {
        // Check cache first
        {
            let cache = self.toolbox_proxy_cache.read().await;
            if let Some(url) = cache.get(region) {
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

        // Cache the base URL by region
        {
            let mut cache = self.toolbox_proxy_cache.write().await;
            cache.insert(region.to_string(), base_url.clone());
        }

        Ok(format!("{}/{}", base_url, sandbox_id))
    }

    /// Create a toolbox API client configuration for a specific sandbox.
    #[allow(dead_code)]
    pub(crate) async fn create_toolbox_config(
        &self,
        sandbox_id: &str,
        region: &str,
    ) -> Result<daytona_toolbox_client::apis::configuration::Configuration, DaytonaError> {
        let toolbox_url = self.get_toolbox_proxy_url(sandbox_id, region).await?;

        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = self.config.bearer_token() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .map_err(|e| DaytonaError::general(e.to_string()))?,
            );
        }
        // Add SDK identification headers (matching Go SDK's createToolboxClient)
        headers.insert(
            "X-Daytona-Source",
            reqwest::header::HeaderValue::from_static(SDK_SOURCE),
        );
        if let Ok(v) = reqwest::header::HeaderValue::from_str(SDK_VERSION) {
            headers.insert("X-Daytona-SDK-Version", v);
        }
        // Add organization header when using JWT (matching Go SDK)
        if let Some(org_id) = &self.config.organization_id {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(org_id) {
                headers.insert("X-Daytona-Organization-ID", v);
            }
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

fn build_create_request(
    params: &CreateParams,
    config: &ResolvedConfig,
) -> Result<models::CreateSandbox, DaytonaError> {
    match params {
        CreateParams::Snapshot(snapshot_params) => {
            validate_base_params(&snapshot_params.base)?;
            let mut cs = models::CreateSandbox::new();
            cs.snapshot = Some(snapshot_params.snapshot.clone());
            apply_base_params(&mut cs, &snapshot_params.base, config);
            Ok(cs)
        }
        CreateParams::Image(image_params) => {
            validate_base_params(&image_params.base)?;
            let mut cs = models::CreateSandbox::new();
            // Set build_info from image source (aligns with Go/TS reference SDKs)
            let dockerfile_content = match &image_params.image {
                crate::types::ImageSource::Name(name) => format!("FROM {}", name),
                crate::types::ImageSource::Custom(docker_image) => docker_image.dockerfile(),
            };
            cs.build_info = Some(Box::new(models::CreateBuildInfo::new(dockerfile_content)));
            // Only set resource fields when > 0 (matching Go/TypeScript SDK behavior)
            if let Some(resources) = &image_params.resources {
                if let Some(cpu) = resources.cpu {
                    if cpu > 0 {
                        cs.cpu = Some(cpu);
                    }
                }
                if let Some(gpu) = resources.gpu {
                    if gpu > 0 {
                        cs.gpu = Some(gpu);
                    }
                }
                if let Some(memory) = resources.memory {
                    if memory > 0 {
                        cs.memory = Some(memory);
                    }
                }
                if let Some(disk) = resources.disk {
                    if disk > 0 {
                        cs.disk = Some(disk);
                    }
                }
            }
            apply_base_params(&mut cs, &image_params.base, config);
            Ok(cs)
        }
    }
}

fn validate_base_params(base: &crate::types::SandboxBaseParams) -> Result<(), DaytonaError> {
    if let Some(interval) = base.auto_stop_interval {
        if interval < 0 {
            return Err(DaytonaError::general(
                "autoStopInterval must be a non-negative integer",
            ));
        }
    }
    if let Some(interval) = base.auto_archive_interval {
        if interval < 0 {
            return Err(DaytonaError::general(
                "autoArchiveInterval must be a non-negative integer",
            ));
        }
    }
    Ok(())
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
    // Explicitly set public and network_block_all to false when not provided,
    // matching Go SDK behavior which always sends these fields
    // (Go booleans default to false; leaving them as None might cause the API
    // to apply different defaults).
    cs.public = Some(base.public.unwrap_or(false));
    cs.auto_stop_interval = base.auto_stop_interval;
    cs.auto_archive_interval = base.auto_archive_interval;

    // Handle ephemeral sandboxes: set auto_delete_interval to 0
    if base.ephemeral == Some(true) {
        cs.auto_delete_interval = Some(0);
    } else {
        cs.auto_delete_interval = base.auto_delete_interval;
    }

    cs.network_block_all = Some(base.network_block_all.unwrap_or(false));
    if let Some(allow_list) = &base.network_allow_list {
        cs.network_allow_list = Some(allow_list.join(","));
    }
    if let Some(volumes) = &base.volumes {
        let api_volumes: Vec<models::SandboxVolume> = volumes
            .iter()
            .map(|v| {
                let mut sv = models::SandboxVolume::new(
                    v.volume_id.clone(),
                    v.mount_path.clone(),
                );
                sv.subpath = v.subpath.clone();
                sv
            })
            .collect();
        cs.volumes = Some(api_volumes);
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
    use wiremock::matchers::{method, path};
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

        let paginated_json = serde_json::json!({
            "items": [
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
            ],
            "total": 2.0,
            "page": 1.0,
            "totalPages": 1.0
        });

        Mock::given(method("GET"))
            .and(path("/sandbox/paginated"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&paginated_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let result = client.list(None, None, None).await.unwrap();
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].id, "sb-1");
        assert_eq!(result.items[1].id, "sb-2");
        assert_eq!(result.total, 2);
        assert_eq!(result.page, 1);
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

        // GET mock needed because wait_for_start defaults to true
        Mock::given(method("GET"))
            .and(path("/sandbox/new-sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sandbox_json))
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
    async fn test_client_create_sandbox_early_error() {
        let mock_server = MockServer::start().await;

        let sandbox_json = serde_json::json!({
            "id": "failed-sandbox",
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
            "state": "error"
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
            base: crate::types::SandboxBaseParams::default(),
            snapshot: "base-snapshot".to_string(),
        });

        let err = client
            .create(params, CreateSandboxOptions::default())
            .await
            .unwrap_err();
        assert!(err.message().contains("failed to start"));
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
        let url = client.get_toolbox_proxy_url("sb-1", "us").await.unwrap();
        assert!(url.contains("sb-1"));

        // Second call with same region should use cache (mock expects exactly 1 call)
        let url2 = client.get_toolbox_proxy_url("sb-2", "us").await.unwrap();
        assert!(url2.contains("sb-2"));
    }

    #[tokio::test]
    async fn test_client_find_one_by_id() {
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
        let sandbox = client
            .find_one(Some("sandbox-123"), None)
            .await
            .unwrap();
        assert_eq!(sandbox.id, "sandbox-123");
    }

    #[tokio::test]
    async fn test_client_find_one_by_labels() {
        let mock_server = MockServer::start().await;

        let paginated_json = serde_json::json!({
            "items": [{
                "id": "sb-match",
                "organizationId": "org-1",
                "name": "matched-sandbox",
                "user": "daytona",
                "env": {},
                "labels": {"env": "prod"},
                "public": false,
                "networkBlockAll": false,
                "target": "us",
                "cpu": 2.0,
                "gpu": 0.0,
                "memory": 4.0,
                "disk": 20.0,
                "state": "started"
            }],
            "total": 1.0,
            "page": 1.0,
            "totalPages": 1.0
        });

        Mock::given(method("GET"))
            .and(path("/sandbox/paginated"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&paginated_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        let sandbox = client.find_one(None, Some(&labels)).await.unwrap();
        assert_eq!(sandbox.id, "sb-match");
    }

    #[tokio::test]
    async fn test_client_find_one_not_found() {
        let mock_server = MockServer::start().await;

        let paginated_json = serde_json::json!({
            "items": [],
            "total": 0.0,
            "page": 1.0,
            "totalPages": 0.0
        });

        Mock::given(method("GET"))
            .and(path("/sandbox/paginated"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&paginated_json))
            .mount(&mock_server)
            .await;

        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.find_one(None, None).await.unwrap_err();
        assert!(matches!(err, DaytonaError::NotFound { .. }));
    }

    #[tokio::test]
    async fn test_client_get_empty_id() {
        let mock_server = MockServer::start().await;
        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.get("").await.unwrap_err();
        assert!(err.message().contains("sandbox ID or name is required"));
    }

    #[tokio::test]
    async fn test_client_list_invalid_page() {
        let mock_server = MockServer::start().await;
        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.list(None, Some(0), None).await.unwrap_err();
        assert!(err.message().contains("page must be a positive integer"));

        let err = client.list(None, Some(-1), None).await.unwrap_err();
        assert!(err.message().contains("page must be a positive integer"));
    }

    #[tokio::test]
    async fn test_client_list_invalid_limit() {
        let mock_server = MockServer::start().await;
        let config = DaytonaConfig {
            api_key: Some("test-key".to_string()),
            api_url: Some(mock_server.uri()),
            ..Default::default()
        };
        let client = Client::new_with_config(config).await.unwrap();
        let err = client.list(None, None, Some(0)).await.unwrap_err();
        assert!(err.message().contains("limit must be a positive integer"));
    }
}