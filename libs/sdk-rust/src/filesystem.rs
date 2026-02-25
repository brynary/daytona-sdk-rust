use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::file_system_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;
use crate::types::SetFilePermissionsOptions;

/// Service for file system operations within a sandbox.
pub struct FileSystemService {
    pub(crate) config: ToolboxConfig,
}

impl FileSystemService {
    /// Create a folder at the specified path.
    pub async fn create_folder(&self, folder_path: &str, mode: &str) -> Result<(), DaytonaError> {
        file_system_api::create_folder(&self.config, folder_path, mode)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// List files in a directory.
    pub async fn list_files(
        &self,
        dir_path: &str,
    ) -> Result<Vec<daytona_toolbox_client::models::FileInfo>, DaytonaError> {
        let files = file_system_api::list_files(&self.config, Some(dir_path))
            .await
            .map_err(convert_toolbox_error)?;
        Ok(files)
    }

    /// Delete a file or directory.
    pub async fn delete_file(&self, file_path: &str) -> Result<(), DaytonaError> {
        file_system_api::delete_file(&self.config, file_path, None)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Upload a file to the sandbox from a local path.
    pub async fn upload_file(
        &self,
        remote_path: &str,
        local_path: std::path::PathBuf,
    ) -> Result<(), DaytonaError> {
        file_system_api::upload_file(&self.config, remote_path, local_path)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Download a file from the sandbox as a raw response.
    pub async fn download_file(&self, file_path: &str) -> Result<reqwest::Response, DaytonaError> {
        let resp = file_system_api::download_file(&self.config, file_path)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(resp)
    }

    /// Get information about a file.
    pub async fn get_file_info(
        &self,
        file_path: &str,
    ) -> Result<daytona_toolbox_client::models::FileInfo, DaytonaError> {
        let info = file_system_api::get_file_info(&self.config, file_path)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(info)
    }

    /// Move a file from source to destination.
    pub async fn move_files(&self, source: &str, destination: &str) -> Result<(), DaytonaError> {
        file_system_api::move_file(&self.config, source, destination)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Search for text within files.
    pub async fn search_files(
        &self,
        dir_path: &str,
        pattern: &str,
    ) -> Result<daytona_toolbox_client::models::SearchFilesResponse, DaytonaError> {
        let result = file_system_api::search_files(&self.config, dir_path, pattern)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(result)
    }

    /// Find files matching a pattern.
    pub async fn find_files(
        &self,
        dir_path: &str,
        pattern: &str,
    ) -> Result<Vec<daytona_toolbox_client::models::Match>, DaytonaError> {
        let matches = file_system_api::find_in_files(&self.config, dir_path, pattern)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(matches)
    }

    /// Replace text in files.
    pub async fn replace_in_files(
        &self,
        files: Vec<String>,
        pattern: &str,
        new_value: &str,
    ) -> Result<Vec<daytona_toolbox_client::models::ReplaceResult>, DaytonaError> {
        let req = daytona_toolbox_client::models::ReplaceRequest {
            files,
            new_value: new_value.to_string(),
            pattern: pattern.to_string(),
        };
        let results = file_system_api::replace_in_files(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(results)
    }

    /// Set file permissions.
    pub async fn set_file_permissions(
        &self,
        file_path: &str,
        options: SetFilePermissionsOptions,
    ) -> Result<(), DaytonaError> {
        file_system_api::set_file_permissions(
            &self.config,
            file_path,
            options.owner.as_deref(),
            options.group.as_deref(),
            options.mode.as_deref(),
        )
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

    async fn fs_service(mock_server: &MockServer) -> FileSystemService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        FileSystemService { config }
    }

    #[tokio::test]
    async fn test_create_folder() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/files/folder"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        svc.create_folder("/home/daytona/project", "0755")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_list_files() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "file1.txt", "isDir": false, "size": 100, "modTime": "", "mode": "0644", "owner": "user", "group": "user", "permissions": "rw-r--r--"},
                {"name": "subdir", "isDir": true, "size": 0, "modTime": "", "mode": "0755", "owner": "user", "group": "user", "permissions": "rwxr-xr-x"}
            ])))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        let files = svc.list_files("/home/daytona").await.unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_file() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/files"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        svc.delete_file("/home/daytona/file.txt").await.unwrap();
    }

    #[tokio::test]
    async fn test_move_files() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/files/move"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        svc.move_files("/old/path", "/new/path").await.unwrap();
    }

    #[tokio::test]
    async fn test_search_files() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/files/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": ["test.rs"]
            })))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        let result = svc.search_files("/home", "fn main").await.unwrap();
    }

    #[tokio::test]
    async fn test_set_file_permissions() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/files/permissions"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = fs_service(&mock_server).await;
        let opts = SetFilePermissionsOptions {
            mode: Some("0644".to_string()),
            owner: Some("daytona".to_string()),
            ..Default::default()
        };
        svc.set_file_permissions("/home/daytona/file.txt", opts)
            .await
            .unwrap();
    }
}
