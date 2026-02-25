use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Code language for the code interpreter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeLanguage {
    #[default]
    Python,
    Javascript,
    Typescript,
}

impl std::fmt::Display for CodeLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::Javascript => write!(f, "javascript"),
            Self::Typescript => write!(f, "typescript"),
        }
    }
}

/// Resource allocation for a sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<i32>,
}

/// Volume mount configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub volume_id: String,
    pub mount_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
}

/// Base parameters shared by all sandbox creation methods.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxBaseParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeLanguage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_stop_interval: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_archive_interval: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_delete_interval: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<VolumeMount>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_block_all: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_allow_list: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
}

/// Parameters for creating a sandbox from a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotParams {
    #[serde(flatten)]
    pub base: SandboxBaseParams,
    pub snapshot: String,
}

/// Parameters for creating a sandbox from a Docker image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageParams {
    #[serde(flatten)]
    pub base: SandboxBaseParams,
    pub image: ImageSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
}

/// The image source for creating a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
    /// A simple image name string (e.g., "ubuntu:22.04").
    Name(String),
    /// A DockerImage builder result.
    Custom(crate::image::DockerImage),
}

/// Discriminated union of sandbox creation parameters.
#[derive(Debug, Clone)]
pub enum CreateParams {
    Snapshot(SnapshotParams),
    Image(ImageParams),
}

/// Options for creating a sandbox.
#[derive(Debug, Clone, Default)]
pub struct CreateSandboxOptions {
    pub timeout: Option<std::time::Duration>,
    pub wait_for_start: bool,
    pub log_sender: Option<tokio::sync::mpsc::UnboundedSender<String>>,
}

/// Options for executing a command.
#[derive(Debug, Clone, Default)]
pub struct ExecuteCommandOptions {
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub timeout: Option<std::time::Duration>,
}

/// Result of executing a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    pub exit_code: i32,
    pub result: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

/// Information about a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_time: Option<String>,
}

/// A file to upload.
#[derive(Debug, Clone)]
pub struct FileUpload {
    pub path: String,
    pub content: Vec<u8>,
}

/// Git status of a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub current_branch: String,
    #[serde(default)]
    pub files: Vec<FileStatus>,
    #[serde(default)]
    pub ahead: i32,
    #[serde(default)]
    pub behind: i32,
}

/// Status of a single file in git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    pub name: String,
    pub staging: char,
    pub worktree: char,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
}

/// Git commit response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitResponse {
    pub sha: String,
}

/// PTY terminal size.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

/// PTY session info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Screenshot region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Screenshot options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScreenshotOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<ScreenshotRegion>,
}

/// Screenshot response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResponse {
    pub base64_image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Preview link for a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewLink {
    pub url: String,
    pub token: String,
}

/// Execution output message (from code interpreter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub content: String,
}

/// Volume information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Snapshot information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub general: Option<SnapshotGeneral>,
}

/// Snapshot general metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotGeneral {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
}

/// Parameters for creating a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSnapshotParams {
    pub name: String,
    pub image: ImageSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_validation: Option<bool>,
}

/// Options for git clone.
#[derive(Debug, Clone, Default)]
pub struct GitCloneOptions {
    pub branch: Option<String>,
    pub commit_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Options for git commit.
#[derive(Debug, Clone, Default)]
pub struct GitCommitOptions {
    pub allow_empty: Option<bool>,
}

/// Options for git push.
#[derive(Debug, Clone, Default)]
pub struct GitPushOptions {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Options for git pull.
#[derive(Debug, Clone, Default)]
pub struct GitPullOptions {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// Options for git delete branch.
#[derive(Debug, Clone, Default)]
pub struct GitDeleteBranchOptions {
    pub force: Option<bool>,
}

/// Options for creating a folder.
#[derive(Debug, Clone, Default)]
pub struct CreateFolderOptions {
    pub mode: Option<String>,
}

/// Options for setting file permissions.
#[derive(Debug, Clone, Default)]
pub struct SetFilePermissionsOptions {
    pub mode: Option<String>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// Options for running code.
#[derive(Debug, Clone, Default)]
pub struct RunCodeOptions {
    pub context_id: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub timeout: Option<std::time::Duration>,
}

/// Code interpreter context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterpreterContext {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Options for creating a PTY session.
#[derive(Debug, Clone, Default)]
pub struct PtySessionOptions {
    pub size: Option<PtySize>,
    pub env: Option<HashMap<String, String>>,
}

/// Display information from computer use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub width: i32,
    pub height: i32,
}

/// Recording info from computer use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<String>,
}

/// Window information from computer use display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

/// Options for pip install in DockerImage builder.
#[derive(Debug, Clone, Default)]
pub struct PipInstallOptions {
    pub find_links: Option<String>,
    pub index_url: Option<String>,
    pub extra_index_urls: Option<Vec<String>>,
    pub pre: Option<bool>,
    pub extra_options: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_language_default() {
        assert_eq!(CodeLanguage::default(), CodeLanguage::Python);
    }

    #[test]
    fn test_code_language_display() {
        assert_eq!(CodeLanguage::Python.to_string(), "python");
        assert_eq!(CodeLanguage::Javascript.to_string(), "javascript");
        assert_eq!(CodeLanguage::Typescript.to_string(), "typescript");
    }

    #[test]
    fn test_sandbox_base_params_defaults() {
        let params = SandboxBaseParams::default();
        assert!(params.name.is_none());
        assert!(params.user.is_none());
        assert!(params.language.is_none());
        assert!(params.env_vars.is_none());
        assert!(params.labels.is_none());
    }

    #[test]
    fn test_resources_default() {
        let r = Resources::default();
        assert!(r.cpu.is_none());
        assert!(r.gpu.is_none());
        assert!(r.memory.is_none());
        assert!(r.disk.is_none());
    }

    #[test]
    fn test_execute_command_options_default() {
        let opts = ExecuteCommandOptions::default();
        assert!(opts.cwd.is_none());
        assert!(opts.env.is_none());
        assert!(opts.timeout.is_none());
    }
}
