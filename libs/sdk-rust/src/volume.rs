use std::time::Duration;

use daytona_api_client::apis::configuration::Configuration as ApiConfig;
use daytona_api_client::apis::volumes_api;

use crate::client::convert_api_error;
use crate::error::DaytonaError;

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Service for managing volumes.
pub struct VolumeService {
    pub(crate) api_config: ApiConfig,
    pub(crate) org_id: Option<String>,
}

impl VolumeService {
    /// List all volumes.
    pub async fn list(&self) -> Result<Vec<daytona_api_client::models::VolumeDto>, DaytonaError> {
        let volumes = volumes_api::list_volumes(&self.api_config, self.org_id.as_deref(), None)
            .await
            .map_err(convert_api_error)?;
        Ok(volumes)
    }

    /// Get a volume by ID.
    pub async fn get(
        &self,
        volume_id: &str,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        let volume = volumes_api::get_volume(&self.api_config, volume_id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(volume)
    }

    /// Create a new volume.
    pub async fn create(
        &self,
        params: daytona_api_client::models::CreateVolume,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        let volume = volumes_api::create_volume(&self.api_config, params, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(volume)
    }

    /// Delete a volume.
    pub async fn delete(&self, volume_id: &str) -> Result<(), DaytonaError> {
        volumes_api::delete_volume(&self.api_config, volume_id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Wait for a volume to be ready.
    pub async fn wait_for_ready(
        &self,
        volume_id: &str,
        timeout: Option<Duration>,
    ) -> Result<(), DaytonaError> {
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(DaytonaError::timeout(format!(
                        "timed out waiting for volume {} to be ready",
                        volume_id,
                    )));
                }
            }

            let volume =
                volumes_api::get_volume(&self.api_config, volume_id, self.org_id.as_deref())
                    .await
                    .map_err(convert_api_error)?;

            let state_str = format!("{:?}", volume.state);
            if state_str.contains("Ready") || state_str.contains("ready") {
                return Ok(());
            }
            if state_str.contains("Error") || state_str.contains("error") {
                return Err(DaytonaError::general(format!(
                    "volume {} entered error state",
                    volume_id,
                )));
            }

            tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn volume_service(mock_server: &MockServer) -> VolumeService {
        let config = ApiConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        VolumeService {
            api_config: config,
            org_id: None,
        }
    }

    #[tokio::test]
    async fn test_list_volumes() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/volumes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "vol-1", "name": "data-volume", "organizationId": "org-1", "state": "ready", "errorReason": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01"},
                {"id": "vol-2", "name": "cache-volume", "organizationId": "org-1", "state": "ready", "errorReason": null, "createdAt": "2024-01-01", "updatedAt": "2024-01-01"}
            ])))
            .mount(&mock_server)
            .await;

        let svc = volume_service(&mock_server).await;
        let volumes = svc.list().await.unwrap();
        assert_eq!(volumes.len(), 2);
    }

    #[tokio::test]
    async fn test_get_volume() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/volumes/vol-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "vol-1",
                "name": "data-volume",
                "organizationId": "org-1",
                "state": "ready",
                "errorReason": null,
                "createdAt": "2024-01-01",
                "updatedAt": "2024-01-01"
            })))
            .mount(&mock_server)
            .await;

        let svc = volume_service(&mock_server).await;
        let volume = svc.get("vol-1").await.unwrap();
        assert_eq!(volume.id, "vol-1");
    }

    #[tokio::test]
    async fn test_delete_volume() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/volumes/vol-1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = volume_service(&mock_server).await;
        svc.delete("vol-1").await.unwrap();
    }

    #[tokio::test]
    async fn test_wait_for_ready_timeout() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/volumes/vol-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "vol-1",
                "name": "data-volume",
                "organizationId": "org-1",
                "state": "pending_create",
                "errorReason": null,
                "createdAt": "2024-01-01",
                "updatedAt": "2024-01-01"
            })))
            .mount(&mock_server)
            .await;

        let svc = volume_service(&mock_server).await;
        let err = svc
            .wait_for_ready("vol-1", Some(Duration::from_millis(100)))
            .await
            .unwrap_err();
        assert!(matches!(err, DaytonaError::Timeout { .. }));
    }
}
