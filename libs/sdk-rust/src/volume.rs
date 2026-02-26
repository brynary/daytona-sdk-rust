use std::time::Duration;

use daytona_api_client::apis::configuration::Configuration as ApiConfig;
use daytona_api_client::apis::volumes_api;
use daytona_api_client::models::VolumeState;

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

    /// Get a volume by name.
    ///
    /// This matches the Go SDK's `Get` behavior which uses the volume name as
    /// the lookup key rather than the internal ID.
    pub async fn get_by_name(
        &self,
        name: &str,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        let volume =
            volumes_api::get_volume_by_name(&self.api_config, name, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;
        Ok(volume)
    }

    /// Create a new volume.
    ///
    /// Takes the volume name as a string, matching Go SDK behavior.
    /// The volume starts in "pending" state and transitions to "ready" when available.
    /// Use [`VolumeService::wait_for_ready`] to wait for the volume to become ready.
    pub async fn create(
        &self,
        name: &str,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        let params = daytona_api_client::models::CreateVolume::new(name.to_string());
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
    ///
    /// Polls the volume state until it reaches `Ready`, enters an `Error` state,
    /// or the timeout expires. Returns the updated volume data when ready.
    ///
    /// Accepts either a volume ID or name. For name-based lookup matching the
    /// Go SDK's `WaitForReady(ctx, volume, timeout)`, use [`VolumeService::wait_for_ready_by_name`].
    pub async fn wait_for_ready(
        &self,
        volume_id: &str,
        timeout: Option<Duration>,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        self.poll_volume_state(
            |svc| Box::pin(svc.get(volume_id)),
            volume_id,
            timeout,
        )
        .await
    }

    /// Wait for a volume to be ready, polling by name.
    ///
    /// This matches the Go SDK's `WaitForReady(ctx, volume, timeout)` behavior
    /// which looks up the volume by name on each poll iteration.
    pub async fn wait_for_ready_by_name(
        &self,
        volume_name: &str,
        timeout: Option<Duration>,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError> {
        self.poll_volume_state(
            |svc| Box::pin(svc.get_by_name(volume_name)),
            volume_name,
            timeout,
        )
        .await
    }

    /// Internal helper that polls volume state using a configurable fetch function.
    async fn poll_volume_state<'a, F>(
        &'a self,
        fetch: F,
        display_id: &str,
        timeout: Option<Duration>,
    ) -> Result<daytona_api_client::models::VolumeDto, DaytonaError>
    where
        F: Fn(&'a Self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<daytona_api_client::models::VolumeDto, DaytonaError>> + Send + 'a>>,
    {
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(DaytonaError::timeout(format!(
                        "timed out waiting for volume {} to be ready",
                        display_id,
                    )));
                }
            }

            let volume = fetch(self).await?;

            match volume.state {
                VolumeState::Ready => return Ok(volume),
                VolumeState::Error => {
                    let reason = volume
                        .error_reason
                        .unwrap_or_else(|| "unknown error".to_string());
                    return Err(DaytonaError::general(format!(
                        "volume {} entered error state: {}",
                        display_id, reason,
                    )));
                }
                _ => {}
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
    async fn test_get_volume_by_name() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/volumes/by-name/data-volume"))
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
        let volume = svc.get_by_name("data-volume").await.unwrap();
        assert_eq!(volume.id, "vol-1");
        assert_eq!(volume.name, "data-volume");
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

    #[tokio::test]
    async fn test_wait_for_ready_returns_volume() {
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
        let volume = svc
            .wait_for_ready("vol-1", Some(Duration::from_secs(5)))
            .await
            .unwrap();
        assert_eq!(volume.id, "vol-1");
        assert_eq!(volume.name, "data-volume");
    }

    #[tokio::test]
    async fn test_wait_for_ready_by_name() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/volumes/by-name/data-volume"))
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
        let volume = svc
            .wait_for_ready_by_name("data-volume", Some(Duration::from_secs(5)))
            .await
            .unwrap();
        assert_eq!(volume.id, "vol-1");
        assert_eq!(volume.name, "data-volume");
    }
}