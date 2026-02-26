use daytona_api_client::apis::configuration::Configuration as ApiConfig;
use daytona_api_client::apis::snapshots_api;
use daytona_api_client::models;

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
                create_req.build_info = Some(Box::new(
                    models::CreateBuildInfo::new(docker_image.dockerfile()),
                ));
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

    /// Delete a snapshot.
    pub async fn delete(&self, snapshot_id_or_name: &str) -> Result<(), DaytonaError> {
        snapshots_api::remove_snapshot(
            &self.api_config,
            snapshot_id_or_name,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;
        Ok(())
    }
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
                    {"id": "snap-1", "name": "ubuntu-22.04", "state": "active", "general": true, "cpu": 2.0, "gpu": 0.0, "mem": 4.0, "disk": 20.0, "size": null, "entrypoint": null, "errorReason": null, "lastUsedAt": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01"},
                    {"id": "snap-2", "name": "python-3.11", "state": "active", "general": true, "cpu": 2.0, "gpu": 0.0, "mem": 4.0, "disk": 20.0, "size": null, "entrypoint": null, "errorReason": null, "lastUsedAt": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01"}
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
                "updatedAt": "2024-01-01"
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
    async fn test_delete_snapshot() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/snapshots/snap-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = snapshot_service(&mock_server).await;
        svc.delete("snap-1").await.unwrap();
    }
}