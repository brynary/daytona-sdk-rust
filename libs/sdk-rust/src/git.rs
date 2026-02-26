use daytona_toolbox_client::apis::configuration::Configuration as ToolboxConfig;
use daytona_toolbox_client::apis::git_api;

use crate::client::convert_toolbox_error;
use crate::error::DaytonaError;
use crate::types::{
    GitCloneOptions, GitCommitOptions, GitDeleteBranchOptions, GitPullOptions, GitPushOptions,
};

/// Service for Git operations within a sandbox.
pub struct GitService {
    pub(crate) config: ToolboxConfig,
}

impl GitService {
    /// Clone a Git repository.
    pub async fn clone(
        &self,
        url: &str,
        target_path: &str,
        options: GitCloneOptions,
    ) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitCloneRequest {
            url: url.to_string(),
            path: target_path.to_string(),
            branch: options.branch,
            commit_id: options.commit_id,
            username: options.username,
            password: options.password,
        };
        git_api::clone_repository(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Get the git status for a repository.
    pub async fn status(
        &self,
        repo_path: &str,
    ) -> Result<daytona_toolbox_client::models::GitStatus, DaytonaError> {
        let status = git_api::get_status(&self.config, repo_path)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(status)
    }

    /// Stage files for commit.
    pub async fn add(&self, repo_path: &str, files: Vec<String>) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitAddRequest {
            files,
            path: repo_path.to_string(),
        };
        git_api::add_files(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Commit staged changes.
    pub async fn commit(
        &self,
        repo_path: &str,
        message: &str,
        author: &str,
        email: &str,
        options: GitCommitOptions,
    ) -> Result<daytona_toolbox_client::models::GitCommitResponse, DaytonaError> {
        let req = daytona_toolbox_client::models::GitCommitRequest {
            message: message.to_string(),
            path: repo_path.to_string(),
            author: author.to_string(),
            email: email.to_string(),
            allow_empty: options.allow_empty,
        };
        let result = git_api::commit_changes(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(result)
    }

    /// Push changes to remote.
    pub async fn push(&self, repo_path: &str, options: GitPushOptions) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitRepoRequest {
            path: repo_path.to_string(),
            username: options.username,
            password: options.password,
        };
        git_api::push_changes(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Pull changes from remote.
    pub async fn pull(&self, repo_path: &str, options: GitPullOptions) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitRepoRequest {
            path: repo_path.to_string(),
            username: options.username,
            password: options.password,
        };
        git_api::pull_changes(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// List branches.
    pub async fn branches(
        &self,
        repo_path: &str,
    ) -> Result<daytona_toolbox_client::models::ListBranchResponse, DaytonaError> {
        let branches = git_api::list_branches(&self.config, repo_path)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(branches)
    }

    /// Checkout a branch.
    pub async fn checkout(&self, repo_path: &str, branch: &str) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitCheckoutRequest {
            path: repo_path.to_string(),
            branch: branch.to_string(),
        };
        git_api::checkout_branch(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Create a new branch.
    pub async fn create_branch(&self, repo_path: &str, branch: &str) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitBranchRequest {
            path: repo_path.to_string(),
            name: branch.to_string(),
        };
        git_api::create_branch(&self.config, req)
            .await
            .map_err(convert_toolbox_error)?;
        Ok(())
    }

    /// Delete a branch.
    pub async fn delete_branch(
        &self,
        repo_path: &str,
        branch: &str,
        _options: GitDeleteBranchOptions,
    ) -> Result<(), DaytonaError> {
        let req = daytona_toolbox_client::models::GitPeriodGitDeleteBranchRequest {
            path: repo_path.to_string(),
            name: branch.to_string(),
        };
        git_api::delete_branch(&self.config, req)
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

    async fn git_service(mock_server: &MockServer) -> GitService {
        let config = ToolboxConfig {
            base_path: mock_server.uri(),
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            user_agent: None,
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: Some("test-token".to_string()),
            api_key: None,
        };
        GitService { config }
    }

    #[tokio::test]
    async fn test_git_clone() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/clone"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.clone(
            "https://github.com/example/repo.git",
            "/home/daytona/repo",
            GitCloneOptions::default(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_git_clone_with_branch() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/clone"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.clone(
            "https://github.com/example/repo.git",
            "/home/daytona/repo",
            GitCloneOptions {
                branch: Some("develop".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_git_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/git/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "currentBranch": "main",
                "fileStatus": [],
                "ahead": 0,
                "behind": 0
            })))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        let status = svc.status("/home/daytona/repo").await.unwrap();
        assert_eq!(status.current_branch, "main");
    }

    #[tokio::test]
    async fn test_git_add() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/add"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.add("/home/daytona/repo", vec!["file.txt".to_string()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_git_commit() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/commit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hash": "abc123def"
            })))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        let result = svc
            .commit(
                "/home/daytona/repo",
                "Initial commit",
                "Test User",
                "test@example.com",
                GitCommitOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.hash, "abc123def");
    }

    #[tokio::test]
    async fn test_git_push() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/push"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.push("/home/daytona/repo", GitPushOptions::default())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_git_pull() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/pull"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.pull("/home/daytona/repo", GitPullOptions::default())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_git_branches() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/git/branches"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "branches": ["main", "develop"]
            })))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        let _result = svc.branches("/home/daytona/repo").await.unwrap();
    }

    #[tokio::test]
    async fn test_git_checkout() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/checkout"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.checkout("/home/daytona/repo", "develop").await.unwrap();
    }

    #[tokio::test]
    async fn test_git_create_branch() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/git/branches"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.create_branch("/home/daytona/repo", "feature/new")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_git_delete_branch() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/git/branches"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let svc = git_service(&mock_server).await;
        svc.delete_branch(
            "/home/daytona/repo",
            "old-branch",
            GitDeleteBranchOptions::default(),
        )
        .await
        .unwrap();
    }
}
