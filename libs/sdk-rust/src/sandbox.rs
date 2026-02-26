use std::collections::HashMap;
use std::time::Duration;

use daytona_api_client::apis::sandbox_api;
use daytona_api_client::models;
use daytona_api_client::models::SandboxState;

use crate::client::{convert_api_error, convert_toolbox_error, Client};
use crate::code_interpreter::CodeInterpreterService;
use crate::computer_use::ComputerUseService;
use crate::error::DaytonaError;
use crate::filesystem::FileSystemService;
use crate::git::GitService;
use crate::lsp::LspService;
use crate::process::ProcessService;
use crate::types::{PreviewLink, Resources};

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Compute the remaining timeout after some elapsed time, matching Go/TS behavior.
///
/// Returns `None` (no timeout) if `total` is zero, or `Some(remaining)` where
/// `remaining = max(1ms, total - elapsed)`.  The 1ms floor mirrors the TS SDK's
/// `Math.max(0.001, timeout - elapsed)` which ensures at least one poll iteration.
fn remaining_timeout(
    total: Duration,
    start: tokio::time::Instant,
) -> Option<Duration> {
    if total.is_zero() {
        return None;
    }
    let elapsed = start.elapsed();
    let remaining = total.saturating_sub(elapsed);
    if remaining.is_zero() {
        Some(Duration::from_millis(1))
    } else {
        Some(remaining)
    }
}

type ToolboxConfig = daytona_toolbox_client::apis::configuration::Configuration;

/// A Daytona sandbox with lifecycle and service access.
#[derive(Debug)]
pub struct Sandbox {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub snapshot: Option<String>,
    pub user: String,
    pub env: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub public: bool,
    pub target: String,
    pub cpu: f64,
    pub gpu: f64,
    pub memory: f64,
    pub disk: f64,
    pub state: Option<SandboxState>,
    pub error_reason: Option<String>,
    pub recoverable: Option<bool>,
    pub backup_state: Option<models::sandbox::BackupState>,
    pub backup_created_at: Option<String>,
    pub auto_stop_interval: Option<f64>,
    pub auto_archive_interval: Option<f64>,
    pub auto_delete_interval: Option<f64>,
    pub volumes: Option<Vec<models::SandboxVolume>>,
    pub build_info: Option<Box<models::BuildInfo>>,
    pub network_block_all: bool,
    pub network_allow_list: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,

    pub(crate) api_config: daytona_api_client::apis::configuration::Configuration,
    pub(crate) org_id: Option<String>,
    pub(crate) client: ClientRef,
    /// Cached toolbox config, created lazily on first service access.
    /// Matches Go/TS behavior of creating the toolbox client once per sandbox.
    pub(crate) toolbox_config_cache: tokio::sync::OnceCell<ToolboxConfig>,
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
        Sandbox {
            id: api_sandbox.id,
            organization_id: api_sandbox.organization_id,
            name: api_sandbox.name,
            snapshot: api_sandbox.snapshot,
            user: api_sandbox.user,
            env: api_sandbox.env,
            labels: api_sandbox.labels,
            public: api_sandbox.public,
            target: api_sandbox.target,
            cpu: api_sandbox.cpu,
            gpu: api_sandbox.gpu,
            memory: api_sandbox.memory,
            disk: api_sandbox.disk,
            state: api_sandbox.state,
            error_reason: api_sandbox.error_reason,
            recoverable: api_sandbox.recoverable,
            backup_state: api_sandbox.backup_state,
            backup_created_at: api_sandbox.backup_created_at,
            auto_stop_interval: api_sandbox.auto_stop_interval,
            auto_archive_interval: api_sandbox.auto_archive_interval,
            auto_delete_interval: api_sandbox.auto_delete_interval,
            volumes: api_sandbox.volumes,
            build_info: api_sandbox.build_info,
            network_block_all: api_sandbox.network_block_all,
            network_allow_list: api_sandbox.network_allow_list,
            created_at: api_sandbox.created_at,
            updated_at: api_sandbox.updated_at,
            api_config: client.api_config.clone(),
            org_id: client.config.organization_id.clone(),
            client: ClientRef::Borrowed {
                toolbox_proxy_cache: client.toolbox_proxy_cache.clone(),
                config: client.config.clone(),
            },
            toolbox_config_cache: tokio::sync::OnceCell::new(),
        }
    }

    /// Start the sandbox and wait for it to reach the started state.
    ///
    /// Updates the sandbox's local state from the API after the operation completes,
    /// matching Go/TypeScript SDK behavior.
    ///
    /// Uses a default timeout of 60 seconds. For a custom timeout, use
    /// [`Sandbox::start_with_timeout`].
    pub async fn start(&mut self) -> Result<(), DaytonaError> {
        self.start_with_timeout(Duration::from_secs(60)).await
    }

    /// Start the sandbox with a custom timeout.
    ///
    /// The method blocks until the sandbox reaches the "started" state or the
    /// timeout is exceeded. Pass `Duration::ZERO` for no timeout.
    /// Updates the sandbox's local state from the API after completion.
    ///
    /// The timeout covers the entire operation (API call + wait), matching
    /// Go (context-based) and TypeScript (elapsed-time subtraction) behavior.
    pub async fn start_with_timeout(&mut self, timeout: Duration) -> Result<(), DaytonaError> {
        let start_time = tokio::time::Instant::now();
        let api_sandbox =
            sandbox_api::start_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;
        // Update local state immediately from the API response (matching TS behavior)
        self.update_from_api(api_sandbox);
        let timeout_opt = remaining_timeout(timeout, start_time);
        self.wait_for_state_mut(SandboxState::Started, timeout_opt)
            .await
    }

    /// Stop the sandbox and wait for it to reach the stopped state.
    ///
    /// Updates the sandbox's local state from the API after the operation completes,
    /// matching Go/TypeScript SDK behavior.
    ///
    /// Uses a default timeout of 60 seconds. For a custom timeout, use
    /// [`Sandbox::stop_with_timeout`].
    pub async fn stop(&mut self) -> Result<(), DaytonaError> {
        self.stop_with_timeout(Duration::from_secs(60)).await
    }

    /// Stop the sandbox with a custom timeout.
    ///
    /// The method blocks until the sandbox reaches the "stopped" state or the
    /// timeout is exceeded. Pass `Duration::ZERO` for no timeout.
    /// Updates the sandbox's local state from the API after completion.
    ///
    /// The timeout covers the entire operation (API call + wait), matching
    /// Go (context-based) and TypeScript (elapsed-time subtraction) behavior.
    pub async fn stop_with_timeout(&mut self, timeout: Duration) -> Result<(), DaytonaError> {
        let start_time = tokio::time::Instant::now();
        let api_sandbox =
            sandbox_api::stop_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;
        // Update local state immediately from the API response (matching TS behavior)
        self.update_from_api(api_sandbox);
        let timeout_opt = remaining_timeout(timeout, start_time);
        self.wait_for_state_mut(SandboxState::Stopped, timeout_opt)
            .await
    }

    /// Delete the sandbox.
    pub async fn delete(&self) -> Result<(), DaytonaError> {
        sandbox_api::delete_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Archive the sandbox.
    pub async fn archive(&mut self) -> Result<(), DaytonaError> {
        sandbox_api::archive_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        self.refresh_data().await
    }

    /// Recover the sandbox from a recoverable error and wait for it to start.
    ///
    /// Updates the sandbox's local state from the API after the operation completes.
    ///
    /// Uses a default timeout of 60 seconds. Pass a custom timeout via
    /// [`Sandbox::recover_with_timeout`].
    pub async fn recover(&mut self) -> Result<(), DaytonaError> {
        self.recover_with_timeout(Duration::from_secs(60)).await
    }

    /// Recover the sandbox from a recoverable error with a custom timeout.
    ///
    /// The method blocks until the sandbox reaches the "started" state or the
    /// timeout is exceeded. Pass `Duration::ZERO` for no timeout.
    /// Updates the sandbox's local state from the API after completion.
    ///
    /// The timeout covers the entire operation (API call + wait), matching
    /// Go (context-based) and TypeScript (elapsed-time subtraction) behavior.
    pub async fn recover_with_timeout(&mut self, timeout: Duration) -> Result<(), DaytonaError> {
        let start_time = tokio::time::Instant::now();
        let api_sandbox =
            sandbox_api::recover_sandbox(&self.api_config, &self.id, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;
        // Update local state immediately from the API response (matching TS behavior)
        self.update_from_api(api_sandbox);
        let timeout_opt = remaining_timeout(timeout, start_time);
        self.wait_for_state_mut(SandboxState::Started, timeout_opt)
            .await
    }

    /// Refresh the sandbox's last activity timestamp.
    ///
    /// This resets the timer for automated lifecycle management actions (auto-stop,
    /// auto-archive, auto-delete). Useful for keeping long-running sessions alive
    /// while there is still user activity.
    pub async fn refresh_activity(&self) -> Result<(), DaytonaError> {
        sandbox_api::update_last_activity(&self.api_config, &self.id, self.org_id.as_deref())
            .await
            .map_err(convert_api_error)?;
        Ok(())
    }

    /// Wait for the sandbox to reach the "started" state.
    pub async fn wait_for_start(&self, timeout: Option<Duration>) -> Result<(), DaytonaError> {
        self.wait_for_state(SandboxState::Started, timeout).await
    }

    /// Wait for the sandbox to reach the "stopped" state.
    ///
    /// Also treats `Destroyed` as stopped to handle ephemeral sandboxes that are
    /// automatically deleted upon stopping (matching TypeScript SDK behavior).
    pub async fn wait_for_stop(&self, timeout: Option<Duration>) -> Result<(), DaytonaError> {
        self.wait_for_state(SandboxState::Stopped, timeout).await
    }

    /// Resize the sandbox resources with a default timeout of 60 seconds.
    ///
    /// Waits for the resize to complete before returning.
    /// Updates the sandbox's local state from the API after the operation completes,
    /// matching Go/TypeScript SDK behavior.
    /// For a custom timeout, use [`Sandbox::resize_with_timeout`].
    pub async fn resize(&mut self, resources: &Resources) -> Result<(), DaytonaError> {
        self.resize_with_timeout(resources, Duration::from_secs(60))
            .await
    }

    /// Resize the sandbox resources with a custom timeout.
    ///
    /// Waits for the resize to complete or the timeout to expire.
    /// Pass `Duration::ZERO` for no timeout.
    /// Updates the sandbox's local state from the API after the resize completes
    /// (matching Go SDK behavior where `WaitForResize` calls `RefreshData` in
    /// its polling loop).
    ///
    /// The timeout covers the entire operation (API call + wait + refresh),
    /// matching Go (context-based) and TypeScript (elapsed-time subtraction) behavior.
    pub async fn resize_with_timeout(
        &mut self,
        resources: &Resources,
        timeout: Duration,
    ) -> Result<(), DaytonaError> {
        let start_time = tokio::time::Instant::now();

        let mut resize = models::ResizeSandbox {
            cpu: None,
            memory: None,
            disk: None,
        };
        if let Some(cpu) = resources.cpu {
            if cpu > 0 {
                resize.cpu = Some(cpu);
            }
        }
        if let Some(memory) = resources.memory {
            if memory > 0 {
                resize.memory = Some(memory);
            }
        }
        if let Some(disk) = resources.disk {
            if disk > 0 {
                resize.disk = Some(disk);
            }
        }

        let api_sandbox =
            sandbox_api::resize_sandbox(&self.api_config, &self.id, resize, self.org_id.as_deref())
                .await
                .map_err(convert_api_error)?;

        // Update sandbox data from the resize response (matching Go SDK)
        self.update_from_api(api_sandbox);

        let timeout_opt = remaining_timeout(timeout, start_time);
        self.wait_for_resize(timeout_opt).await?;

        // Refresh sandbox data after resize completes so local fields are up to date
        // (matching Go SDK where WaitForResize calls RefreshData in polling loop)
        self.refresh_data().await
    }

    /// Wait for a sandbox resize operation to complete.
    pub async fn wait_for_resize(&self, timeout: Option<Duration>) -> Result<(), DaytonaError> {
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(DaytonaError::timeout(format!(
                        "sandbox resize did not complete within {:?}",
                        timeout.unwrap(),
                    )));
                }
            }

            let api_sandbox =
                sandbox_api::get_sandbox(&self.api_config, &self.id, self.org_id.as_deref(), None)
                    .await
                    .map_err(convert_api_error)?;

            if let Some(state) = &api_sandbox.state {
                if matches!(state, SandboxState::Error | SandboxState::BuildFailed) {
                    return Err(DaytonaError::general(format!(
                        "sandbox {} resize failed: {}",
                        self.id,
                        api_sandbox
                            .error_reason
                            .unwrap_or_else(|| "unknown error".to_string()),
                    )));
                }
                if !matches!(state, SandboxState::Resizing) {
                    return Ok(());
                }
            }

            tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
        }
    }

    /// Set labels on the sandbox.
    ///
    /// Replaces all existing labels. Returns the new labels from the API response.
    /// Calls `refresh_data` afterwards to ensure all sandbox fields are in sync
    /// (matching Go SDK behavior).
    pub async fn set_labels(
        &mut self,
        labels: HashMap<String, String>,
    ) -> Result<HashMap<String, String>, DaytonaError> {
        let sandbox_labels = models::SandboxLabels { labels };
        let response = sandbox_api::replace_labels(
            &self.api_config,
            &self.id,
            sandbox_labels,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;
        let new_labels = response.labels;
        self.labels = new_labels.clone();

        // Refresh all sandbox data to keep fields in sync (matching Go SDK)
        self.refresh_data().await?;

        Ok(new_labels)
    }

    /// Get a preview link for a port on this sandbox.
    ///
    /// The port is specified as an integer, matching Go/TypeScript SDK behavior
    /// where `GetPreviewLink(ctx, port int)` / `getPreviewLink(port: number)` accept
    /// integer port numbers.
    pub async fn get_preview_link(&self, port: u16) -> Result<PreviewLink, DaytonaError> {
        let preview = sandbox_api::get_port_preview_url(
            &self.api_config,
            &self.id,
            port as f64,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(PreviewLink {
            url: preview.url,
            token: preview.token,
        })
    }

    /// Get a signed preview URL for a port on this sandbox.
    ///
    /// The signed URL can be shared and includes an authentication token.
    /// The optional `expires_in_seconds` parameter controls how long the URL remains valid
    /// (defaults to 60 seconds on the server if not provided).
    ///
    /// Matches the TypeScript SDK `getSignedPreviewUrl` method.
    pub async fn get_signed_preview_url(
        &self,
        port: i32,
        expires_in_seconds: Option<i32>,
    ) -> Result<models::SignedPortPreviewUrl, DaytonaError> {
        let result = sandbox_api::get_signed_port_preview_url(
            &self.api_config,
            &self.id,
            port,
            self.org_id.as_deref(),
            expires_in_seconds,
        )
        .await
        .map_err(convert_api_error)?;

        Ok(result)
    }

    /// Expire a signed preview URL for a port on this sandbox.
    ///
    /// Invalidates a previously issued signed preview URL token.
    ///
    /// Matches the TypeScript SDK `expireSignedPreviewUrl` method.
    pub async fn expire_signed_preview_url(
        &self,
        port: i32,
        token: &str,
    ) -> Result<(), DaytonaError> {
        sandbox_api::expire_signed_port_preview_url(
            &self.api_config,
            &self.id,
            port,
            token,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(())
    }

    /// Create SSH access credentials for the sandbox.
    ///
    /// The optional `expires_in_minutes` parameter controls how long the SSH access
    /// remains valid. Returns an [`SshAccessDto`](models::SshAccessDto) containing
    /// the token and SSH command.
    ///
    /// Matches the TypeScript SDK `createSshAccess` method.
    pub async fn create_ssh_access(
        &self,
        expires_in_minutes: Option<f64>,
    ) -> Result<models::SshAccessDto, DaytonaError> {
        let result = sandbox_api::create_ssh_access(
            &self.api_config,
            &self.id,
            self.org_id.as_deref(),
            expires_in_minutes,
        )
        .await
        .map_err(convert_api_error)?;

        Ok(result)
    }

    /// Revoke SSH access for the sandbox.
    ///
    /// Invalidates the specified SSH access token.
    ///
    /// Matches the TypeScript SDK `revokeSshAccess` method.
    pub async fn revoke_ssh_access(&self, token: &str) -> Result<(), DaytonaError> {
        sandbox_api::revoke_ssh_access(
            &self.api_config,
            &self.id,
            self.org_id.as_deref(),
            Some(token),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(())
    }

    /// Validate an SSH access token for the sandbox.
    ///
    /// Returns validation information about the token.
    ///
    /// Matches the TypeScript SDK `validateSshAccess` method.
    pub async fn validate_ssh_access(
        &self,
        token: &str,
    ) -> Result<models::SshAccessValidationDto, DaytonaError> {
        let result = sandbox_api::validate_ssh_access(
            &self.api_config,
            token,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        Ok(result)
    }

    /// Get the user's home directory path in the sandbox.
    pub async fn get_user_home_dir(&self) -> Result<String, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        let resp =
            daytona_toolbox_client::apis::info_api::get_user_home_dir(toolbox_config)
                .await
                .map_err(convert_toolbox_error)?;
        Ok(resp.dir)
    }

    /// Get the user's home directory path in the sandbox.
    ///
    /// **Deprecated**: Use [`Sandbox::get_user_home_dir`] instead. This method
    /// is provided for compatibility with the TypeScript SDK's `getUserRootDir`.
    #[deprecated(note = "Use get_user_home_dir instead")]
    pub async fn get_user_root_dir(&self) -> Result<String, DaytonaError> {
        self.get_user_home_dir().await
    }

    /// Get the current working directory in the sandbox.
    pub async fn get_working_dir(&self) -> Result<String, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        let resp =
            daytona_toolbox_client::apis::info_api::get_work_dir(toolbox_config)
                .await
                .map_err(convert_toolbox_error)?;
        Ok(resp.dir)
    }

    /// Set the auto-archive interval in minutes.
    ///
    /// The sandbox will be automatically archived after being stopped for this
    /// many minutes. Set to 0 to disable auto-archiving.
    pub async fn set_auto_archive_interval(
        &mut self,
        interval_minutes: i32,
    ) -> Result<(), DaytonaError> {
        if interval_minutes < 0 {
            return Err(DaytonaError::general(
                "autoArchiveInterval must be a non-negative integer",
            ));
        }

        sandbox_api::set_auto_archive_interval(
            &self.api_config,
            &self.id,
            interval_minutes as f64,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        self.auto_archive_interval = Some(interval_minutes as f64);
        Ok(())
    }

    /// Set the auto-stop interval in minutes.
    ///
    /// The sandbox will automatically stop after being idle (no new events) for
    /// the specified interval. Set to 0 to disable auto-stop.
    pub async fn set_autostop_interval(
        &mut self,
        interval_minutes: i32,
    ) -> Result<(), DaytonaError> {
        if interval_minutes < 0 {
            return Err(DaytonaError::general(
                "autoStopInterval must be a non-negative integer",
            ));
        }

        sandbox_api::set_autostop_interval(
            &self.api_config,
            &self.id,
            interval_minutes as f64,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        self.auto_stop_interval = Some(interval_minutes as f64);
        Ok(())
    }

    /// Set the auto-delete interval in minutes.
    ///
    /// The sandbox will be automatically deleted after being stopped for this
    /// many minutes. Set to -1 to disable auto-deletion. Set to 0 to delete
    /// immediately upon stopping.
    pub async fn set_auto_delete_interval(
        &mut self,
        interval_minutes: i32,
    ) -> Result<(), DaytonaError> {
        sandbox_api::set_auto_delete_interval(
            &self.api_config,
            &self.id,
            interval_minutes as f64,
            self.org_id.as_deref(),
        )
        .await
        .map_err(convert_api_error)?;

        self.auto_delete_interval = Some(interval_minutes as f64);
        Ok(())
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

        self.update_from_api(api_sandbox);

        Ok(())
    }

    /// Get the FileSystemService for this sandbox.
    ///
    /// The underlying toolbox configuration is cached after the first call,
    /// matching Go/TypeScript SDK behavior where services share a single client.
    pub async fn filesystem(&self) -> Result<FileSystemService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(FileSystemService {
            config: toolbox_config.clone(),
        })
    }

    /// Shorthand alias for [`Sandbox::filesystem`], matching TypeScript SDK's `sandbox.fs`.
    pub async fn fs(&self) -> Result<FileSystemService, DaytonaError> {
        self.filesystem().await
    }

    /// Get the GitService for this sandbox.
    pub async fn git(&self) -> Result<GitService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(GitService {
            config: toolbox_config.clone(),
        })
    }

    /// Get the ProcessService for this sandbox.
    pub async fn process(&self) -> Result<ProcessService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(ProcessService {
            config: toolbox_config.clone(),
        })
    }

    /// Get the CodeInterpreterService for this sandbox.
    pub async fn code_interpreter(&self) -> Result<CodeInterpreterService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(CodeInterpreterService {
            config: toolbox_config.clone(),
        })
    }

    /// Get the ComputerUseService for this sandbox.
    pub async fn computer_use(&self) -> Result<ComputerUseService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(ComputerUseService {
            config: toolbox_config.clone(),
        })
    }

    /// Get an LspService for this sandbox with the given language and project path.
    ///
    /// The LSP service provides IDE-like features: code completion, symbol search,
    /// and document analysis. This matches Go SDK's `Sandbox.Lsp(languageID, projectPath)`
    /// and TypeScript SDK's `Sandbox.createLspServer(languageId, pathToProject)`.
    ///
    /// # Arguments
    /// * `language_id` - The language identifier (e.g., `"python"`, `"go"`, `"typescript"`)
    /// * `project_path` - The root path of the project for LSP analysis
    pub async fn lsp(
        &self,
        language_id: &str,
        project_path: &str,
    ) -> Result<LspService, DaytonaError> {
        let toolbox_config = self.get_or_create_toolbox_config().await?;
        Ok(LspService {
            config: toolbox_config.clone(),
            language_id: language_id.to_string(),
            project_path: project_path.to_string(),
        })
    }

    /// Returns true if `current` matches `target`, treating `Destroyed` as
    /// equivalent to `Stopped` to handle ephemeral sandboxes (matches TS SDK).
    fn state_matches(current: &SandboxState, target: &SandboxState) -> bool {
        if current == target {
            return true;
        }
        // Treat Destroyed as Stopped for ephemeral sandboxes that auto-delete on stop
        *target == SandboxState::Stopped && *current == SandboxState::Destroyed
    }

    /// Wait for the sandbox to reach a target state (immutable version).
    ///
    /// When the target is `Stopped`, `Destroyed` is also treated as a match
    /// to handle ephemeral sandboxes that auto-delete upon stopping
    /// (matching TypeScript SDK behavior).
    ///
    /// Error/BuildFailed states cause an immediate error for any target state
    /// (matching TypeScript SDK behavior where both `waitUntilStarted` and
    /// `waitUntilStopped` check for error states).
    ///
    /// When waiting for `Stopped` and the sandbox returns NotFound (404), it is
    /// treated as `Destroyed` (matching TypeScript SDK's `refreshDataSafe` pattern
    /// for ephemeral sandboxes that auto-delete upon stopping).
    async fn wait_for_state(
        &self,
        target_state: SandboxState,
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

            let api_sandbox = match sandbox_api::get_sandbox(
                &self.api_config,
                &self.id,
                self.org_id.as_deref(),
                None,
            )
            .await
            {
                Ok(s) => s,
                Err(err) => {
                    let daytona_err = convert_api_error(err);
                    // When waiting for Stopped, treat NotFound as Destroyed
                    // (matches TS refreshDataSafe pattern for ephemeral sandboxes)
                    if target_state == SandboxState::Stopped
                        && matches!(daytona_err, DaytonaError::NotFound { .. })
                    {
                        return Ok(());
                    }
                    return Err(daytona_err);
                }
            };

            if let Some(state) = &api_sandbox.state {
                if Self::state_matches(state, &target_state) {
                    return Ok(());
                }
                // Check for error states for all targets (matching TS behavior
                // where both waitUntilStarted and waitUntilStopped check for errors)
                if matches!(state, SandboxState::Error | SandboxState::BuildFailed) {
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

    /// Wait for the sandbox to reach a target state, updating local fields on success.
    ///
    /// This is used by `start`, `stop`, and `recover` to ensure the sandbox
    /// object reflects the latest state from the API (matching Go/TS behavior).
    /// When the target is `Stopped`, `Destroyed` is also treated as a match.
    ///
    /// Error/BuildFailed states cause an immediate error for any target state
    /// (matching TypeScript SDK behavior).
    ///
    /// When waiting for `Stopped` and the sandbox returns NotFound (404), it is
    /// treated as `Destroyed` (matching TypeScript SDK's `refreshDataSafe` pattern).
    async fn wait_for_state_mut(
        &mut self,
        target_state: SandboxState,
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

            let api_sandbox = match sandbox_api::get_sandbox(
                &self.api_config,
                &self.id,
                self.org_id.as_deref(),
                None,
            )
            .await
            {
                Ok(s) => s,
                Err(err) => {
                    let daytona_err = convert_api_error(err);
                    // When waiting for Stopped, treat NotFound as Destroyed
                    // (matches TS refreshDataSafe pattern for ephemeral sandboxes)
                    if target_state == SandboxState::Stopped
                        && matches!(daytona_err, DaytonaError::NotFound { .. })
                    {
                        self.state = Some(SandboxState::Destroyed);
                        return Ok(());
                    }
                    return Err(daytona_err);
                }
            };

            if let Some(state) = &api_sandbox.state {
                if Self::state_matches(state, &target_state) {
                    // Update local fields from the API response
                    self.update_from_api(api_sandbox);
                    return Ok(());
                }
                // Check for error states for all targets (matching TS behavior)
                if matches!(state, SandboxState::Error | SandboxState::BuildFailed) {
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

    /// Update local sandbox fields from an API response.
    fn update_from_api(&mut self, api_sandbox: models::Sandbox) {
        self.name = api_sandbox.name;
        self.snapshot = api_sandbox.snapshot;
        self.user = api_sandbox.user;
        self.env = api_sandbox.env;
        self.labels = api_sandbox.labels;
        self.public = api_sandbox.public;
        self.state = api_sandbox.state;
        self.target = api_sandbox.target;
        self.cpu = api_sandbox.cpu;
        self.gpu = api_sandbox.gpu;
        self.memory = api_sandbox.memory;
        self.disk = api_sandbox.disk;
        self.error_reason = api_sandbox.error_reason;
        self.recoverable = api_sandbox.recoverable;
        self.backup_state = api_sandbox.backup_state;
        self.backup_created_at = api_sandbox.backup_created_at;
        self.auto_stop_interval = api_sandbox.auto_stop_interval;
        self.auto_archive_interval = api_sandbox.auto_archive_interval;
        self.auto_delete_interval = api_sandbox.auto_delete_interval;
        self.volumes = api_sandbox.volumes;
        self.build_info = api_sandbox.build_info;
        self.network_block_all = api_sandbox.network_block_all;
        self.network_allow_list = api_sandbox.network_allow_list;
        self.created_at = api_sandbox.created_at;
        self.updated_at = api_sandbox.updated_at;
    }

    /// Get or create the cached toolbox config.
    /// The config is created on first call and reused thereafter,
    /// matching Go/TypeScript SDK behavior.
    async fn get_or_create_toolbox_config(&self) -> Result<&ToolboxConfig, DaytonaError> {
        self.toolbox_config_cache
            .get_or_try_init(|| self.create_toolbox_config())
            .await
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
                // Cache by the sandbox's own target (region), matching Go SDK behavior.
                // This ensures sandboxes in different regions use distinct proxy URLs.
                let region = &self.target;

                // Check cache
                let base_url = {
                    let cache = toolbox_proxy_cache.read().await;
                    cache.get(region).cloned()
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
                        cache.insert(region.to_string(), base.clone());
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
                // Add SDK identification headers (matching Go SDK's createToolboxClient)
                headers.insert(
                    "X-Daytona-Source",
                    reqwest::header::HeaderValue::from_static("rust-sdk"),
                );
                if let Ok(v) = reqwest::header::HeaderValue::from_str(env!("CARGO_PKG_VERSION")) {
                    headers.insert("X-Daytona-SDK-Version", v);
                }
                // Add organization header when using JWT (matching Go SDK)
                if let Some(org_id) = &config.organization_id {
                    if let Ok(v) = reqwest::header::HeaderValue::from_str(org_id) {
                        headers.insert("X-Daytona-Organization-ID", v);
                    }
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
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopped")).unwrap());
        sandbox.start().await.unwrap();
        assert_eq!(sandbox.state, Some(SandboxState::Started));
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

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopped")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state, Some(SandboxState::Stopped));
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

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "archived")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
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

        // refresh_data is called after set_labels (matching Go SDK)
        // The GET response should reflect the updated labels
        let mut refreshed = sandbox_json("sb-1", "started");
        refreshed["labels"] = serde_json::json!({"env": "prod", "team": "backend"});
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&refreshed))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        labels.insert("team".to_string(), "backend".to_string());

        sandbox.set_labels(labels).await.unwrap();
        assert_eq!(sandbox.labels.get("env").unwrap(), "prod");
        assert_eq!(sandbox.labels.get("team").unwrap(), "backend");
    }

    #[tokio::test]
    async fn test_sandbox_set_labels_returns_labels() {
        let mock_server = MockServer::start().await;

        let labels_response = serde_json::json!({
            "labels": {"project": "api"}
        });

        Mock::given(method("PUT"))
            .and(path("/sandbox/sb-1/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&labels_response))
            .mount(&mock_server)
            .await;

        // refresh_data is called after set_labels (matching Go SDK)
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        let mut labels = HashMap::new();
        labels.insert("project".to_string(), "api".to_string());

        let returned = sandbox.set_labels(labels).await.unwrap();
        assert_eq!(returned.get("project").unwrap(), "api");
    }

    #[tokio::test]
    async fn test_sandbox_new_fields() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server).await;

        let json = serde_json::json!({
            "id": "sb-1",
            "organizationId": "org-1",
            "name": "test",
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
            "state": "error",
            "recoverable": true,
            "backupState": "Completed",
            "backupCreatedAt": "2024-01-01T00:00:00Z"
        });
        let sandbox = client.sandbox_from_api(serde_json::from_value(json).unwrap());
        assert_eq!(sandbox.recoverable, Some(true));
        assert_eq!(
            sandbox.backup_state,
            Some(daytona_api_client::models::sandbox::BackupState::Completed)
        );
        assert_eq!(
            sandbox.backup_created_at.as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
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

        // Use wait_for_start (immutable) for the timeout test
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

        // GET mock for wait_for_resize polling
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        let resources = crate::types::Resources {
            cpu: Some(4),
            memory: Some(8),
            disk: Some(40),
            ..Default::default()
        };
        sandbox.resize(&resources).await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_set_auto_archive_interval() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/autoarchive/30"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        sandbox.set_auto_archive_interval(30).await.unwrap();
        assert_eq!(sandbox.auto_archive_interval, Some(30.0));
    }

    #[tokio::test]
    async fn test_sandbox_set_autostop_interval() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/autostop/15"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        sandbox.set_autostop_interval(15).await.unwrap();
        assert_eq!(sandbox.auto_stop_interval, Some(15.0));
    }

    #[tokio::test]
    async fn test_sandbox_set_auto_delete_interval() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/autodelete/60"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());

        sandbox.set_auto_delete_interval(60).await.unwrap();
        assert_eq!(sandbox.auto_delete_interval, Some(60.0));
    }

    #[tokio::test]
    async fn test_sandbox_recover() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/recover"))
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
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "error")).unwrap());
        sandbox.recover().await.unwrap();
        assert_eq!(sandbox.state, Some(SandboxState::Started));
    }

    #[tokio::test]
    async fn test_sandbox_refresh_activity() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/last-activity"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.refresh_activity().await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_state_is_enum() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server).await;

        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        assert_eq!(sandbox.state, Some(SandboxState::Started));

        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopped")).unwrap());
        assert_eq!(sandbox.state, Some(SandboxState::Stopped));

        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "error")).unwrap());
        assert_eq!(sandbox.state, Some(SandboxState::Error));
    }

    #[tokio::test]
    async fn test_sandbox_get_signed_preview_url() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1/ports/3000/signed-preview-url"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sandboxId": "sb-1",
                "port": 3000,
                "token": "signed-token-abc",
                "url": "https://preview.daytona.io/sb-1/3000?token=signed-token-abc"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        let result = sandbox.get_signed_preview_url(3000, None).await.unwrap();
        assert_eq!(result.token, "signed-token-abc");
        assert_eq!(result.port, 3000);
    }

    #[tokio::test]
    async fn test_sandbox_expire_signed_preview_url() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/ports/3000/signed-preview-url/my-token/expire"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox
            .expire_signed_preview_url(3000, "my-token")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_create_ssh_access() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/ssh-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ssh-123",
                "sandboxId": "sb-1",
                "token": "ssh-token-abc",
                "expiresAt": "2024-12-31T23:59:59Z",
                "createdAt": "2024-01-01T00:00:00Z",
                "updatedAt": "2024-01-01T00:00:00Z",
                "sshCommand": "ssh -o StrictHostKeyChecking=no daytona@proxy.daytona.io"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        let result = sandbox.create_ssh_access(Some(60.0)).await.unwrap();
        assert_eq!(result.token, "ssh-token-abc");
        assert_eq!(result.sandbox_id, "sb-1");
    }

    #[tokio::test]
    async fn test_sandbox_revoke_ssh_access() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/sandbox/sb-1/ssh-access"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "started")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.revoke_ssh_access("ssh-token-abc").await.unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_wait_for_stop_treats_destroyed_as_stopped() {
        let mock_server = MockServer::start().await;

        // Return "destroyed" state — ephemeral sandboxes auto-delete on stop
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "destroyed")),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopping")).unwrap());

        // wait_for_stop should succeed even though state is "destroyed" (not "stopped")
        sandbox
            .wait_for_stop(Some(Duration::from_secs(5)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_validate_ssh_access() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sandbox/ssh-access/validate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sandboxId": "sb-1",
                "valid": true,
                "token": "ssh-token-abc"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        let result = sandbox.validate_ssh_access("ssh-token-abc").await.unwrap();
        assert_eq!(result.sandbox_id, "sb-1");
    }

    #[tokio::test]
    async fn test_sandbox_wait_for_stop_errors_on_error_state() {
        let mock_server = MockServer::start().await;

        // Return "error" state — sandbox entered error while stopping
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json({
                let mut j = sandbox_json("sb-1", "error");
                j["errorReason"] = serde_json::json!("disk full");
                j
            }))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopping")).unwrap());

        // wait_for_stop should now fail immediately with error state
        // (matching TS waitUntilStopped behavior)
        let err = sandbox
            .wait_for_stop(Some(Duration::from_secs(5)))
            .await
            .unwrap_err();
        assert!(err.message().contains("error state"));
        assert!(err.message().contains("disk full"));
    }

    #[tokio::test]
    async fn test_sandbox_wait_for_stop_handles_not_found_as_destroyed() {
        let mock_server = MockServer::start().await;

        // Return 404 — sandbox was deleted (ephemeral auto-delete)
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(serde_json::json!({"message": "sandbox not found"})),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "stopping")).unwrap());

        // wait_for_stop should succeed when sandbox returns NotFound
        // (matching TS refreshDataSafe pattern for ephemeral sandboxes)
        sandbox
            .wait_for_stop(Some(Duration::from_secs(5)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_sandbox_stop_updates_state_from_response() {
        let mock_server = MockServer::start().await;

        // stop_sandbox returns the sandbox in "stopping" state
        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/stop"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopping")),
            )
            .mount(&mock_server)
            .await;

        // GET returns "stopped" for the wait loop
        Mock::given(method("GET"))
            .and(path("/sandbox/sb-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json("sb-1", "stopped")))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server).await;
        let mut sandbox = client
            .sandbox_from_api(serde_json::from_value(sandbox_json("sb-1", "started")).unwrap());
        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state, Some(SandboxState::Stopped));
    }
}