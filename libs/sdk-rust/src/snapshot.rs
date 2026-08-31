use std::future::Future;

use daytona_api_client::apis::configuration::Configuration as ApiConfig;
use daytona_api_client::apis::snapshots_api;
use daytona_api_client::models;
use futures_util::StreamExt;

use crate::client::convert_api_error;
use crate::error::DaytonaError;
use crate::types::{CreateSnapshotParams, ImageSource};

/// Service for managing snapshots.
pub struct SnapshotService {
    pub(crate) api_config: ApiConfig,
    pub(crate) org_id: Option<String>,
}

impl SnapshotService {
    /// List all snapshots with optional pagination.
    ///
    /// Matches Go/TypeScript SDK behavior of accepting `page` and `limit` parameters.
    pub async fn list(
        &self,
        page: Option<i32>,
        limit: Option<i32>,
    ) -> Result<daytona_api_client::models::PaginatedSnapshots, DaytonaError> {
        let snapshots = snapshots_api::get_all_snapshots(
            &self.api_config,
            self.org_id.as_deref(),
            page.map(|p| p as f64),
            limit.map(|l| l as f64),
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(convert_api_error)?;
        Ok(snapshots)
    }

    /// Get a snapshot by ID or name.
    pub async fn get(
        &self,
        snapshot_id_or_name: &str,
    ) -> Result<daytona_api_client::models::SnapshotDto, DaytonaError> {
        let snapshot = snapshots_api::get_snapshot(
            &self.api_config,
            snapshot_id_or_name,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;
        Ok(snapshot)
    }

    /// Create a new snapshot from SDK-level params.
    ///
    /// Converts `CreateSnapshotParams` into the API model, handling both string
    /// image names and custom `DockerImage` builders. This is the primary create
    /// method, matching Go/TypeScript SDK behavior where `Create` accepts
    /// SDK-level params.
    pub async fn create(
        &self,
        params: &CreateSnapshotParams,
    ) -> Result<daytona_api_client::models::SnapshotDto, DaytonaError> {
        let mut create_req = models::CreateSnapshot::new(params.name.clone());

        // Handle image: string → image_name, DockerImage → build_info
        match &params.image {
            ImageSource::Name(name) => {
                create_req.image_name = Some(name.clone());
            }
            ImageSource::Custom(docker_image) => {
                create_req.build_info = Some(Box::new(models::CreateBuildInfo::new(
                    docker_image.dockerfile(),
                )));
            }
        }

        // Handle resources — only set fields when > 0 (matching Go/TypeScript SDK behavior)
        if let Some(resources) = &params.resources {
            if let Some(cpu) = resources.cpu {
                if cpu > 0 {
                    create_req.cpu = Some(cpu);
                }
            }
            if let Some(gpu) = resources.gpu {
                if gpu > 0 {
                    create_req.gpu = Some(gpu);
                }
            }
            if let Some(memory) = resources.memory {
                if memory > 0 {
                    create_req.memory = Some(memory);
                }
            }
            if let Some(disk) = resources.disk {
                if disk > 0 {
                    create_req.disk = Some(disk);
                }
            }
        }

        // Handle entrypoint
        if let Some(entrypoint) = &params.entrypoint {
            create_req.entrypoint = Some(entrypoint.clone());
        }

        self.create_raw(create_req).await
    }

    /// Create a new snapshot from raw API params.
    ///
    /// For most use cases, prefer [`SnapshotService::create`] which accepts
    /// SDK-level `CreateSnapshotParams`. This method is for advanced cases
    /// where you need full control over the API request.
    pub async fn create_raw(
        &self,
        params: daytona_api_client::models::CreateSnapshot,
    ) -> Result<daytona_api_client::models::SnapshotDto, DaytonaError> {
        let snapshot =
            snapshots_api::create_snapshot(&self.api_config, params, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;
        Ok(snapshot)
    }

    /// Stream build logs for a snapshot.
    ///
    /// The control plane returns a short-lived log URL. `follow` keeps
    /// the response open until the build ends; without it, the current
    /// log history is returned and the stream closes.
    pub async fn stream_build_logs<F, Fut>(
        &self,
        snapshot_id: &str,
        follow: bool,
        mut on_chunk: F,
    ) -> Result<(), DaytonaError>
    where
        F: FnMut(Vec<u8>) -> Fut + Send,
        Fut: Future<Output = Result<(), DaytonaError>> + Send,
    {
        let logs = snapshots_api::get_snapshot_build_logs_url(
            &self.api_config,
            snapshot_id,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;
        let mut url = url::Url::parse(&logs.url)
            .map_err(|error| DaytonaError::general(format!("invalid snapshot log URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("follow", if follow { "true" } else { "false" });

        let response = self
            .api_config
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| {
                DaytonaError::general(format!("failed to fetch snapshot build logs: {error}"))
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(DaytonaError::api(
                status.as_u16(),
                format!("failed to fetch snapshot build logs: {body}"),
            ));
        }

        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|error| {
                DaytonaError::general(format!("snapshot build log stream failed: {error}"))
            })?;
            on_chunk(chunk.to_vec()).await?;
        }
        Ok(())
    }

    /// Delete a snapshot by ID or name.
    ///
    /// The delete endpoint is ID-only; names are resolved through
    /// [`SnapshotService::get`] first, using the same UUID-shape shortcut
    /// as [`SnapshotService::activate`].
    pub async fn delete(&self, snapshot_id_or_name: &str) -> Result<(), DaytonaError> {
        if is_uuid(snapshot_id_or_name) {
            match self.delete_by_id(snapshot_id_or_name).await {
                // A snapshot *name* may itself be UUID-shaped; only a
                // NotFound falls through to name resolution.
                Err(DaytonaError::NotFound { .. }) => {}
                other => return other,
            }
        }
        let resolved = self.get(snapshot_id_or_name).await?;
        self.delete_by_id(&resolved.id).await
    }

    async fn delete_by_id(&self, snapshot_id: &str) -> Result<(), DaytonaError> {
        snapshots_api::remove_snapshot(&self.api_config, snapshot_id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Activate an inactive snapshot by ID or name, returning the updated
    /// snapshot.
    ///
    /// Matches TS `SnapshotService.activate` / Go `Activate`: the endpoint
    /// is ID-only, so a UUID-shaped identifier is tried directly (falling
    /// back to name resolution only on NotFound, since names may be
    /// UUID-shaped), and any other identifier resolves through
    /// [`SnapshotService::get`] first — keeping the common case at one
    /// round trip.
    pub async fn activate(
        &self,
        snapshot_id_or_name: &str,
    ) -> Result<daytona_api_client::models::SnapshotDto, DaytonaError> {
        if is_uuid(snapshot_id_or_name) {
            match self.activate_by_id(snapshot_id_or_name).await {
                Err(DaytonaError::NotFound { .. }) => {}
                other => return other,
            }
        }
        let resolved = self.get(snapshot_id_or_name).await?;
        self.activate_by_id(&resolved.id).await
    }

    async fn activate_by_id(
        &self,
        snapshot_id: &str,
    ) -> Result<daytona_api_client::models::SnapshotDto, DaytonaError> {
        snapshots_api::activate_snapshot(&self.api_config, snapshot_id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)
    }
}

/// RFC 4122 UUID shape — versions 1-5 with variant 8/9/a/b, or the nil
/// UUID — matching the reference SDKs' UUID_REGEX used to route ID-only
/// operations.
fn is_uuid(value: &str) -> bool {
    if value == "00000000-0000-0000-0000-000000000000" {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    matches!(bytes[14].to_ascii_lowercase(), b'1'..=b'5')
        && matches!(bytes[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn snapshot_service(mock_server: &MockServer) -> SnapshotService {
        let config = ApiConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        SnapshotService {
            api_config: config,
            org_id: None,
        }
    }

    #[tokio::test]
    async fn test_list_snapshots() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/snapshots"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "snap-1", "name": "ubuntu-22.04", "state": "active", "general": true, "cpu": 2.0, "gpu": 0.0, "mem": 4.0, "disk": 20.0, "size": null, "entrypoint": null, "errorReason": null, "lastUsedAt": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01", "sourceSandboxId": null},
                    {"id": "snap-2", "name": "python-3.11", "state": "active", "general": true, "cpu": 2.0, "gpu": 0.0, "mem": 4.0, "disk": 20.0, "size": null, "entrypoint": null, "errorReason": null, "lastUsedAt": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01", "sourceSandboxId": null}
                ],
                "total": 2.0,
                "page": 1.0,
                "totalPages": 1.0
            })))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let snapshots = svc.list(None, None).await.unwrap();
        assert_eq!(snapshots.items.len(), 2);
    }

    #[tokio::test]
    async fn test_get_snapshot() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/snapshots/snap-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "snap-1",
                "name": "ubuntu-22.04",
                "state": "active",
                "general": true,
                "cpu": 2.0,
                "gpu": 0.0,
                "mem": 4.0,
                "disk": 20.0,
                "size": null,
                "entrypoint": null,
                "errorReason": null,
                "lastUsedAt": null,
                "createdAt": "2024-01-01",
                "updatedAt": "2024-01-01",
                "sourceSandboxId": null
            })))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let snapshot = svc.get("snap-1").await.unwrap();
        assert_eq!(snapshot.id, "snap-1");
    }

    #[tokio::test]
    async fn test_get_snapshot_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/snapshots/nonexistent"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({"message": "snapshot not found"})),
            )
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let err = svc.get("nonexistent").await.unwrap_err();
        assert!(matches!(err, DaytonaError::NotFound { .. }));
    }

    #[tokio::test]
    async fn test_stream_build_logs() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/snapshots/snap-1/build-logs-url"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "url": format!("{}/build-logs", mock_server.uri())
            })))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/build-logs"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"step one\nstep two\n"))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let chunks = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let received = std::sync::Arc::clone(&chunks);
        svc.stream_build_logs("snap-1", true, move |chunk| {
            let received = std::sync::Arc::clone(&received);
            async move {
                received.lock().await.extend(chunk);
                Ok(())
            }
        })
        .await
        .unwrap();

        assert_eq!(&*chunks.lock().await, b"step one\nstep two\n");
    }

    fn snapshot_json(id: &str, name: &str, state: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": name, "state": state, "general": true,
            "cpu": 2.0, "gpu": 0.0, "mem": 4.0, "disk": 20.0,
            "size": null, "entrypoint": null, "errorReason": null,
            "lastUsedAt": null, "createdAt": "2024-01-01",
            "updatedAt": "2024-01-01", "sourceSandboxId": null
        })
    }

    const SNAP_UUID: &str = "0195b0a1-7a3d-4bcd-8f12-3456789abcde";

    #[tokio::test]
    async fn test_delete_snapshot_resolves_name_first() {
        let mock_server = MockServer::start().await;

        // "snap-1" is not UUID-shaped, so delete resolves it through GET
        // before calling the ID-only delete endpoint.
        Mock::given(method("GET"))
            .and(path("/snapshots/snap-1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(snapshot_json(SNAP_UUID, "snap-1", "active")),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("DELETE"))
            .and(path(format!("/snapshots/{SNAP_UUID}")))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        svc.delete("snap-1").await.unwrap();
    }

    #[tokio::test]
    async fn test_activate_snapshot_by_uuid_is_one_round_trip() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path(format!("/snapshots/{SNAP_UUID}/activate")))
            .respond_with(ResponseTemplate::new(200).set_body_json(snapshot_json(
                SNAP_UUID,
                "cached-env",
                "active",
            )))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let snapshot = svc.activate(SNAP_UUID).await.unwrap();
        assert_eq!(snapshot.id, SNAP_UUID);
        assert_eq!(
            snapshot.state,
            daytona_api_client::models::SnapshotState::Active
        );
    }

    #[tokio::test]
    async fn test_activate_snapshot_resolves_uuid_shaped_name_on_not_found() {
        let mock_server = MockServer::start().await;
        // A *name* that happens to be UUID-shaped: the direct ID attempt
        // 404s, then resolution through GET finds the real ID.
        let uuid_shaped_name = "11111111-2222-3333-8444-555555555555";

        Mock::given(method("POST"))
            .and(path(format!("/snapshots/{uuid_shaped_name}/activate")))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({"message": "snapshot not found"})),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/snapshots/{uuid_shaped_name}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(snapshot_json(
                SNAP_UUID,
                uuid_shaped_name,
                "inactive",
            )))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/snapshots/{SNAP_UUID}/activate")))
            .respond_with(ResponseTemplate::new(200).set_body_json(snapshot_json(
                SNAP_UUID,
                uuid_shaped_name,
                "active",
            )))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        let snapshot = svc.activate(uuid_shaped_name).await.unwrap();
        assert_eq!(snapshot.id, SNAP_UUID);
    }

    #[test]
    fn test_is_uuid_matches_the_reference_regex() {
        assert!(is_uuid("0195b0a1-7a3d-4bcd-8f12-3456789abcde"));
        assert!(is_uuid("0195B0A1-7A3D-4BCD-8F12-3456789ABCDE"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000"));
        // Version 3 is a name-based UUID: verify the version nibble range.
        assert!(is_uuid("11111111-2222-3333-8444-555555555555"));
        assert!(!is_uuid("snap-1"));
        assert!(!is_uuid("0195b0a1-7a3d-0bcd-8f12-3456789abcde")); // version 0
        assert!(!is_uuid("0195b0a1-7a3d-4bcd-0f12-3456789abcde")); // bad variant
        assert!(!is_uuid("0195b0a17a3d4bcd8f123456789abcde")); // no dashes
    }
}
