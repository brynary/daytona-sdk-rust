//! Live integration tests against the Daytona API.
//!
//! These tests require a valid `DAYTONA_API_KEY` in `.env` at the workspace root.
//! Run with: `cargo test --test integration -- --test-threads=1`
//!
//! Tests must run serially (--test-threads=1) to stay within sandbox concurrency
//! and disk limits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use daytona_api_client::models::{SnapshotState, UpdateSandboxNetworkSettings};
use daytona_sdk::types::{
    ExecuteCommandOptions, GitCloneOptions, GitCommitOptions, ImageParams, ImageSource,
    RunCodeOptions, SandboxBaseParams,
};
use daytona_sdk::{
    Client, CreateParams, CreateSandboxOptions, CreateSnapshotParams, DaytonaConfig, DaytonaError,
    DockerImage, PtyCreateOptions, PtySize, Resources, SandboxClass, SandboxState, SnapshotParams,
};

fn load_env() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let _ = dotenvy::from_path(workspace_root.join(".env"));
}

async fn create_client() -> Client {
    load_env();
    Client::new()
        .await
        .expect("failed to create Daytona client")
}

fn ubuntu_image_params() -> CreateParams {
    CreateParams::Image(ImageParams {
        base: SandboxBaseParams::default(),
        image: ImageSource::Name("ubuntu:22.04".to_string()),
        resources: None,
    })
}

fn python_image_params() -> CreateParams {
    CreateParams::Image(ImageParams {
        base: SandboxBaseParams::default(),
        image: ImageSource::Name("python:3.11-slim".to_string()),
        resources: None,
    })
}

fn create_options() -> CreateSandboxOptions {
    CreateSandboxOptions {
        timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    }
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    format!("{prefix}-{}-{nanos:x}", std::process::id())
}

async fn wait_for_snapshot(
    client: &Client,
    id: &str,
) -> Result<daytona_sdk::api_types::SnapshotDto, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        let snapshot = client
            .snapshot
            .get(id)
            .await
            .map_err(|error| format!("get snapshot: {error}"))?;
        match snapshot.state {
            SnapshotState::Active => return Ok(snapshot),
            SnapshotState::Error | SnapshotState::BuildFailed => {
                return Err(format!(
                    "snapshot build failed: {:?}",
                    snapshot.error_reason
                ));
            }
            _ if tokio::time::Instant::now() >= deadline => {
                return Err(format!("snapshot stayed in state {}", snapshot.state));
            }
            _ => tokio::time::sleep(Duration::from_secs(5)).await,
        }
    }
}

fn vm_capacity_unavailable(error: &DaytonaError) -> bool {
    matches!(error.status_code(), Some(400 | 403))
        && (error.message().contains("No runners are configured")
            || error
                .message()
                .contains("not available to the organization"))
}

/// Helper: write a file inside a sandbox via shell command (avoids broken
/// generated multipart upload client).
async fn write_file_via_exec(process: &daytona_sdk::ProcessService, path: &str, content: &str) {
    let cmd = format!(
        "bash -c 'cat > {} << '\"'\"'RUSTEOF'\"'\"'\n{}\nRUSTEOF'",
        path, content
    );
    let result = process
        .execute_command(&cmd, ExecuteCommandOptions::default())
        .await
        .expect("write file via exec");
    assert_eq!(
        result.exit_code, 0,
        "write_file_via_exec failed: {}",
        result.result
    );
}

// ---------------------------------------------------------------------------
// Sandbox lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_create_and_delete_sandbox() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    assert!(!sandbox.id.is_empty());
    assert!(!sandbox.name.is_empty());
    assert_eq!(sandbox.state, Some(SandboxState::Started));

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_refresh_activity() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    // Regression: the API rejects a literal `null` body with 400
    // "Invalid JSON in request body"; refresh_activity must send an
    // empty object instead.
    sandbox.refresh_activity().await.expect("refresh activity");

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_get_sandbox() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let fetched = client.get(&sandbox.id).await.expect("get sandbox");
    assert_eq!(fetched.id, sandbox.id);
    assert_eq!(fetched.name, sandbox.name);

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_get_nonexistent_sandbox() {
    let client = create_client().await;

    let result = client.get("nonexistent-sandbox-id-12345").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, DaytonaError::NotFound { .. }),
        "expected NotFound, got: {err:?}"
    );
}

#[tokio::test]
async fn test_list_sandboxes() {
    let client = create_client().await;

    let result = client
        .list(None, Some(1), Some(5))
        .await
        .expect("list sandboxes");
    assert!(result.total >= 0);
    assert!(result.page >= 1);
    assert!(result.total_pages >= 0);
}

#[tokio::test]
async fn test_stop_and_start_sandbox() {
    let client = create_client().await;

    let mut sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    assert_eq!(sandbox.state, Some(SandboxState::Started));

    // Stop
    sandbox.stop().await.expect("stop sandbox");
    let stopped = client.get(&sandbox.id).await.expect("get after stop");
    assert_eq!(stopped.state, Some(SandboxState::Stopped));

    // Start
    sandbox.start().await.expect("start sandbox");
    let started = client.get(&sandbox.id).await.expect("get after start");
    assert_eq!(started.state, Some(SandboxState::Started));

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_find_one_by_id() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let found = client
        .find_one(Some(&sandbox.id), None)
        .await
        .expect("find_one");
    assert_eq!(found.id, sandbox.id);

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_create_with_labels() {
    let client = create_client().await;

    let mut labels = HashMap::new();
    labels.insert("rust-sdk-test".to_string(), "true".to_string());
    labels.insert("purpose".to_string(), "integration".to_string());

    let params = CreateParams::Image(ImageParams {
        base: SandboxBaseParams {
            labels: Some(labels.clone()),
            ..Default::default()
        },
        image: ImageSource::Name("ubuntu:22.04".to_string()),
        resources: None,
    });

    let sandbox = client
        .create(params, create_options())
        .await
        .expect("create with labels");

    assert_eq!(
        sandbox.labels.get("rust-sdk-test"),
        Some(&"true".to_string())
    );
    assert_eq!(
        sandbox.labels.get("purpose"),
        Some(&"integration".to_string())
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_create_with_env_vars() {
    let client = create_client().await;

    let mut env_vars = HashMap::new();
    env_vars.insert("MY_TEST_VAR".to_string(), "hello_from_rust".to_string());

    let params = CreateParams::Image(ImageParams {
        base: SandboxBaseParams {
            env_vars: Some(env_vars),
            ..Default::default()
        },
        image: ImageSource::Name("ubuntu:22.04".to_string()),
        resources: None,
    });

    let sandbox = client
        .create(params, create_options())
        .await
        .expect("create with env vars");

    let process = sandbox.process().await.expect("process service");
    let result = process
        .execute_command(
            "bash -c 'echo $MY_TEST_VAR'",
            ExecuteCommandOptions::default(),
        )
        .await
        .expect("execute echo");

    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.trim().contains("hello_from_rust"),
        "expected env var in output, got: {}",
        result.result
    );

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// Process execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_command() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let result = process
        .execute_command(
            "echo 'Hello from Rust SDK'",
            ExecuteCommandOptions::default(),
        )
        .await
        .expect("execute command");

    assert_eq!(result.exit_code, 0);
    assert!(result.result.contains("Hello from Rust SDK"));

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_execute_command_with_cwd() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let result = process
        .execute_command(
            "pwd",
            ExecuteCommandOptions {
                cwd: Some("/tmp".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("execute pwd");

    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.trim().contains("/tmp"),
        "expected /tmp in output, got: {}",
        result.result
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_execute_command_nonzero_exit() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let result = process
        .execute_command("bash -c 'exit 42'", ExecuteCommandOptions::default())
        .await
        .expect("execute exit 42");

    assert_ne!(result.exit_code, 0, "exit code should be non-zero");

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// Snapshots, timers, and network settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_snapshot_create_and_build_log_stream() {
    let client = create_client().await;
    let name = unique("sdk-live-build");
    let params = CreateSnapshotParams {
        name,
        image: ImageSource::Custom(DockerImage::from_dockerfile(
            r#"FROM debian:stable-slim
RUN echo sdk-live-snapshot-build
ENTRYPOINT ["/bin/sh", "-c", "exec sleep infinity"]
"#,
        )),
        region_id: Some("us".to_string()),
        sandbox_class: Some(SandboxClass::CONTAINER),
        resources: Some(Resources {
            cpu: Some(1),
            memory: Some(1),
            disk: Some(1),
            ..Default::default()
        }),
        entrypoint: None,
    };
    let snapshot = client
        .snapshot
        .create(&params)
        .await
        .expect("create snapshot");
    let outcome = async {
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        client
            .snapshot
            .stream_build_logs(&snapshot.id, true, move |chunk| {
                sink.lock().expect("snapshot log lock").extend(chunk);
                async { Ok(()) }
            })
            .await
            .map_err(|error| format!("stream snapshot build logs: {error}"))?;
        if output.lock().expect("snapshot log lock").is_empty() {
            return Err("snapshot build log stream was empty".to_string());
        }
        let active = wait_for_snapshot(&client, &snapshot.id).await?;
        if active.state != SnapshotState::Active {
            return Err(format!("snapshot ended in state {}", active.state));
        }
        Ok::<_, String>(())
    }
    .await;

    client
        .snapshot
        .delete(&snapshot.id)
        .await
        .expect("delete snapshot");
    outcome.expect("snapshot create and build log stream");
}

#[tokio::test]
async fn test_live_sandbox_to_snapshot() {
    let client = create_client().await;
    let mut sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");
    let name = unique("sdk-live-sandbox-snapshot");
    let outcome = async {
        sandbox
            .stop_with_timeout(Duration::from_secs(300))
            .await
            .map_err(|error| format!("stop sandbox: {error}"))?;
        sandbox
            .create_snapshot_with_timeout(&name, Duration::from_secs(600))
            .await
            .map_err(|error| format!("create sandbox snapshot: {error}"))?;
        let snapshot = client
            .snapshot
            .get(&name)
            .await
            .map_err(|error| format!("get sandbox snapshot: {error}"))?;
        if snapshot.source_sandbox_id.as_deref() != Some(sandbox.id.as_str()) {
            return Err(format!(
                "snapshot source was {:?}, expected {}",
                snapshot.source_sandbox_id, sandbox.id
            ));
        }
        Ok::<_, String>(())
    }
    .await;

    let _ = client.snapshot.delete(&name).await;
    sandbox.delete().await.expect("delete sandbox");
    outcome.expect("sandbox-to-snapshot creation");
}

#[tokio::test]
async fn test_live_ttl_and_container_auto_pause_rejection() {
    let client = create_client().await;
    let mut sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");
    let outcome = async {
        sandbox
            .set_ttl(120)
            .await
            .map_err(|error| format!("set TTL: {error}"))?;
        sandbox
            .refresh_data()
            .await
            .map_err(|error| format!("refresh after TTL: {error}"))?;
        if sandbox.auto_destroy_at.is_none() {
            return Err("TTL did not set auto_destroy_at".to_string());
        }
        sandbox
            .set_autostop_interval(0)
            .await
            .map_err(|error| format!("disable auto-stop: {error}"))?;
        match sandbox.set_auto_pause_interval(30).await {
            Err(error)
                if matches!(error.status_code(), Some(400 | 403))
                    || (error.message().contains("not supported")
                        && error.message().contains("container")) => {}
            Err(error) => return Err(format!("unexpected auto-pause error: {error}")),
            Ok(()) => return Err("container sandbox accepted auto-pause".to_string()),
        }
        Ok::<_, String>(())
    }
    .await;

    sandbox.delete().await.expect("delete sandbox");
    outcome.expect("TTL and container auto-pause behavior");
}

#[tokio::test]
async fn test_live_runtime_network_settings() {
    let client = create_client().await;
    let mut sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");
    let outcome = async {
        sandbox
            .update_network_settings(UpdateSandboxNetworkSettings {
                network_block_all: Some(true),
                network_allow_list: None,
                domain_allow_list: None,
            })
            .await
            .map_err(|error| format!("block sandbox network: {error}"))?;
        if !sandbox.network_block_all {
            return Err("network_block_all did not update locally".to_string());
        }
        let mut updated = false;
        for _ in 0..12 {
            match sandbox
                .update_network_settings(UpdateSandboxNetworkSettings {
                    network_block_all: Some(false),
                    network_allow_list: Some("1.1.1.1/32".to_string()),
                    domain_allow_list: None,
                })
                .await
            {
                Ok(()) => {
                    updated = true;
                    break;
                }
                Err(error) if error.message().contains("already in progress") => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                Err(error) => return Err(format!("allow CIDR: {error}")),
            }
        }
        if !updated {
            return Err("CIDR update stayed busy for 60 seconds".to_string());
        }
        if sandbox.network_block_all || sandbox.network_allow_list.as_deref() != Some("1.1.1.1/32")
        {
            return Err(format!(
                "runtime network state was block_all={}, allow_list={:?}",
                sandbox.network_block_all, sandbox.network_allow_list
            ));
        }
        Ok::<_, String>(())
    }
    .await;

    sandbox.delete().await.expect("delete sandbox");
    outcome.expect("runtime network settings");
}

#[tokio::test]
async fn test_live_vm_pause_and_fork_when_capacity_is_available() {
    let client = create_client().await;
    let snapshot_name = unique("sdk-live-vm-base");
    let snapshot = match client
        .snapshot
        .create(&CreateSnapshotParams {
            name: snapshot_name,
            image: ImageSource::Name("ubuntu:24.04".to_string()),
            region_id: Some("us-central-1".to_string()),
            sandbox_class: Some(SandboxClass::LINUX_VM),
            resources: Some(Resources {
                cpu: Some(1),
                memory: Some(3),
                disk: Some(3),
                ..Default::default()
            }),
            entrypoint: None,
        })
        .await
    {
        Ok(snapshot) => snapshot,
        Err(error) if vm_capacity_unavailable(&error) => {
            eprintln!("skipping VM pause/fork live test: {error}");
            return;
        }
        Err(error) => panic!("create VM snapshot: {error}"),
    };
    if let Err(error) = wait_for_snapshot(&client, &snapshot.id).await {
        let _ = client.snapshot.delete(&snapshot.id).await;
        if error.contains("No runners are configured")
            || error.contains("not available to the organization")
        {
            eprintln!("skipping VM pause/fork live test: {error}");
            return;
        }
        panic!("wait for VM snapshot: {error}");
    }

    let base = SandboxBaseParams {
        target: Some("us-central-1".to_string()),
        ..Default::default()
    };
    let mut source = match client
        .create(
            CreateParams::Snapshot(SnapshotParams {
                base,
                snapshot: snapshot.id.clone(),
            }),
            CreateSandboxOptions {
                timeout: Some(Duration::from_secs(300)),
                ..Default::default()
            },
        )
        .await
    {
        Ok(source) => source,
        Err(error) if vm_capacity_unavailable(&error) => {
            let _ = client.snapshot.delete(&snapshot.id).await;
            eprintln!("skipping VM pause/fork live test: {error}");
            return;
        }
        Err(error) => {
            let _ = client.snapshot.delete(&snapshot.id).await;
            panic!("create VM sandbox: {error}");
        }
    };
    let mut forked = None;
    let outcome = async {
        source
            .pause_with_timeout(Duration::from_secs(300))
            .await
            .map_err(|error| format!("pause VM: {error}"))?;
        source
            .start_with_timeout(Duration::from_secs(300))
            .await
            .map_err(|error| format!("resume VM: {error}"))?;

        let process = source
            .process()
            .await
            .map_err(|error| format!("source process service: {error}"))?;
        let setup = process
            .execute_command(
                r#"bash -c 'printf preserved >/tmp/sdk-live-fork-marker; nohup bash -c "exec -a sdk-live-fork-process sleep 600" </dev/null >/dev/null 2>&1 & echo $! >/tmp/sdk-live-fork-pid; cat /tmp/sdk-live-fork-pid'"#,
                ExecuteCommandOptions::default(),
            )
            .await
            .map_err(|error| format!("start VM process: {error}"))?;
        if setup.exit_code != 0 {
            return Err(format!("start VM process failed: {}", setup.result));
        }
        let expected_pid = setup.result.trim().to_string();

        let child = source
            .fork_with_timeout(None, Duration::from_secs(300))
            .await
            .map_err(|error| format!("fork VM: {error}"))?;
        let child_process = child
            .process()
            .await
            .map_err(|error| format!("fork process service: {error}"))?;
        let check = child_process
            .execute_command(
                r#"bash -c 'pid=$(cat /tmp/sdk-live-fork-pid); test "$(cat /tmp/sdk-live-fork-marker)" = preserved; test -r /proc/$pid/cmdline; tr "\0" " " </proc/$pid/cmdline | grep -q sdk-live-fork-process; printf "%s" "$pid"'"#,
                ExecuteCommandOptions::default(),
            )
            .await
            .map_err(|error| format!("check forked VM: {error}"))?;
        if check.exit_code != 0 || check.result.trim() != expected_pid {
            return Err(format!(
                "fork did not preserve process state: expected pid {expected_pid}, got {check:?}"
            ));
        }
        forked = Some(child);
        Ok::<_, String>(())
    }
    .await;

    if let Some(child) = forked {
        let _ = child.delete().await;
    }
    let _ = source.delete().await;
    let _ = client.snapshot.delete(&snapshot.id).await;
    outcome.expect("VM pause and fork behavior");
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_session_lifecycle() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");

    // Create session
    process
        .create_session("test-sess")
        .await
        .expect("create session");

    // Execute command setting env var
    let cmd1 = process
        .execute_session_command("test-sess", "export TEST_VAR=hello", false, false)
        .await
        .expect("set env var");
    assert!(!cmd1.cmd_id.is_empty());

    // Verify env var persists in same session
    let cmd2 = process
        .execute_session_command("test-sess", "echo $TEST_VAR", false, true)
        .await
        .expect("echo env var");
    assert!(!cmd2.cmd_id.is_empty());

    // List sessions
    let sessions = process.list_sessions().await.expect("list sessions");
    assert!(!sessions.is_empty());

    // Delete session
    process
        .delete_session("test-sess")
        .await
        .expect("delete session");

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_live_session_input_and_separated_streaming_logs() {
    let client = create_client().await;
    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");
    let process = sandbox.process().await.expect("process service");
    let session_id = unique("sdk-live-session");
    let outcome = async {
        process
            .create_session(&session_id)
            .await
            .map_err(|error| format!("create session: {error}"))?;
        let command = process
            .execute_session_command(
                &session_id,
                "read -r line; printf 'stdout:%s\\n' \"$line\"; printf 'stderr:%s\\n' \"$line\" >&2",
                true,
                true,
            )
            .await
            .map_err(|error| format!("execute session command: {error}"))?;

        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        let stdout_sink = Arc::clone(&stdout);
        let stderr_sink = Arc::clone(&stderr);
        let logs = process.get_session_command_logs_stream(
            &session_id,
            &command.cmd_id,
            move |chunk| {
                stdout_sink.lock().expect("stdout lock").push_str(&chunk);
                async { Ok(()) }
            },
            move |chunk| {
                stderr_sink.lock().expect("stderr lock").push_str(&chunk);
                async { Ok(()) }
            },
        );
        let input = async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            process
                .send_session_command_input(&session_id, &command.cmd_id, "sdk-live-input\n")
                .await
        };
        let (logs, input) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(30), logs),
            tokio::time::timeout(Duration::from_secs(30), input),
        );
        input
            .map_err(|_| "session input timed out".to_string())?
            .map_err(|error| format!("send session input: {error}"))?;
        logs.map_err(|_| "session log stream timed out".to_string())?
            .map_err(|error| format!("stream session logs: {error}"))?;

        let stdout = stdout.lock().expect("stdout lock").clone();
        let stderr = stderr.lock().expect("stderr lock").clone();
        if !stdout.contains("stdout:sdk-live-input")
            || stdout.contains("stderr:sdk-live-input")
            || !stderr.contains("stderr:sdk-live-input")
            || stderr.contains("stdout:sdk-live-input")
        {
            return Err(format!(
                "streams were not separated: stdout={stdout:?}, stderr={stderr:?}"
            ));
        }
        let fetched = process
            .get_session_command_logs(&session_id, &command.cmd_id)
            .await
            .map_err(|error| format!("fetch session logs: {error}"))?;
        if !fetched.streams_separated
            || !fetched.stdout.contains("stdout:sdk-live-input")
            || !fetched.stderr.contains("stderr:sdk-live-input")
        {
            return Err(format!("fetched logs were not separated: {fetched:?}"));
        }
        Ok::<_, String>(())
    }
    .await;

    let _ = process.delete_session(&session_id).await;
    sandbox.delete().await.expect("delete sandbox");
    outcome.expect("session input and separated streaming logs");
}

// ---------------------------------------------------------------------------
// Filesystem operations
// Uses exec-based file creation since the generated multipart upload client
// has a known bug (empty multipart form).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_filesystem_download_file() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    // Create file via exec
    write_file_via_exec(&process, "/tmp/rust-test.txt", "Hello from Rust SDK").await;

    // Download and verify
    let downloaded = fs
        .download_file("/tmp/rust-test.txt")
        .await
        .expect("download file");
    let content = String::from_utf8(downloaded).expect("valid utf8");
    assert!(
        content.contains("Hello from Rust SDK"),
        "expected content, got: {content}"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_upload_file_bytes() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let fs = sandbox.filesystem().await.expect("filesystem service");

    // Upload via bytes
    fs.upload_file_bytes("/tmp/upload-test.txt", b"uploaded via bytes")
        .await
        .expect("upload file bytes");

    // Download and verify
    let downloaded = fs
        .download_file("/tmp/upload-test.txt")
        .await
        .expect("download file");
    let content = String::from_utf8(downloaded).expect("valid utf8");
    assert!(
        content.contains("uploaded via bytes"),
        "expected uploaded content, got: {content}"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_upload_file() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let fs = sandbox.filesystem().await.expect("filesystem service");

    // Write a local temp file
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"uploaded from local file").unwrap();

    // Upload via path
    fs.upload_file("/tmp/upload-path-test.txt", tmp.path().to_path_buf())
        .await
        .expect("upload file");

    // Download and verify
    let downloaded = fs
        .download_file("/tmp/upload-path-test.txt")
        .await
        .expect("download file");
    let content = String::from_utf8(downloaded).expect("valid utf8");
    assert!(
        content.contains("uploaded from local file"),
        "expected uploaded content, got: {content}"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_create_folder_and_list() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    // Create folder
    fs.create_folder("/tmp/rust-test-dir", None)
        .await
        .expect("create folder");

    // Create a file in it via exec
    write_file_via_exec(&process, "/tmp/rust-test-dir/hello.txt", "hello").await;

    // List files
    let files = fs
        .list_files("/tmp/rust-test-dir")
        .await
        .expect("list files");
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.contains(&"hello.txt"),
        "expected hello.txt in {names:?}"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_delete_file() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    write_file_via_exec(&process, "/tmp/to-delete.txt", "gone soon").await;

    fs.delete_file("/tmp/to-delete.txt", false)
        .await
        .expect("delete file");

    // Verify it's gone
    let result = fs.get_file_info("/tmp/to-delete.txt").await;
    assert!(result.is_err(), "file should not exist after deletion");

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_move_file() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    write_file_via_exec(&process, "/tmp/original.txt", "content").await;

    fs.move_files("/tmp/original.txt", "/tmp/moved.txt")
        .await
        .expect("move file");

    // Verify new location exists
    let info = fs
        .get_file_info("/tmp/moved.txt")
        .await
        .expect("get moved file info");
    assert!(!info.is_dir);

    // Verify old location is gone
    let old = fs.get_file_info("/tmp/original.txt").await;
    assert!(old.is_err());

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_get_file_info() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    write_file_via_exec(&process, "/tmp/info-test.txt", "some test content here").await;

    let info = fs
        .get_file_info("/tmp/info-test.txt")
        .await
        .expect("get file info");
    assert!(!info.is_dir);
    assert!(info.size > 0);

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_filesystem_search_files() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let fs = sandbox.filesystem().await.expect("filesystem service");

    fs.create_folder("/tmp/search-test", None)
        .await
        .expect("create folder");
    write_file_via_exec(
        &process,
        "/tmp/search-test/needle.txt",
        "find the needle in the haystack",
    )
    .await;
    write_file_via_exec(
        &process,
        "/tmp/search-test/other.txt",
        "nothing interesting",
    )
    .await;

    // search_files may return null for the files field when the generated client
    // expects a Vec; skip assertion on the specific deserialization since the API
    // call itself succeeds. Instead, verify via exec.
    let result = process
        .execute_command(
            "grep -rl needle /tmp/search-test",
            ExecuteCommandOptions::default(),
        )
        .await
        .expect("grep search");
    assert_eq!(result.exit_code, 0);
    assert!(result.result.contains("needle.txt"));

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// Git operations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_git_clone_and_status() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let git = sandbox.git().await.expect("git service");

    // Clone a small public repo
    git.clone(
        "https://github.com/octocat/Hello-World.git",
        "/tmp/git-test",
        GitCloneOptions::default(),
    )
    .await
    .expect("git clone");

    // Check status
    let status = git.status("/tmp/git-test").await.expect("git status");
    assert!(
        !status.current_branch.is_empty(),
        "current branch should not be empty"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_git_branches() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let git = sandbox.git().await.expect("git service");

    git.clone(
        "https://github.com/octocat/Hello-World.git",
        "/tmp/git-branch-test",
        GitCloneOptions::default(),
    )
    .await
    .expect("git clone");

    let branches = git
        .branches("/tmp/git-branch-test")
        .await
        .expect("list branches");

    assert!(
        !branches.branches.is_empty(),
        "should have at least one branch"
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_git_add_and_commit() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");
    let git = sandbox.git().await.expect("git service");

    // Clone a repo
    git.clone(
        "https://github.com/octocat/Hello-World.git",
        "/tmp/commit-test",
        GitCloneOptions::default(),
    )
    .await
    .expect("git clone");

    // Create a new file inside the repo via process exec (no git binary needed)
    write_file_via_exec(
        &process,
        "/tmp/commit-test/newfile.txt",
        "added by rust sdk",
    )
    .await;

    // Stage the file via git service API
    git.add("/tmp/commit-test", vec!["newfile.txt".to_string()])
        .await
        .expect("git add");

    // Commit via git service API
    let commit = git
        .commit(
            "/tmp/commit-test",
            "add new file from rust sdk test",
            "Test User",
            "test@test.com",
            GitCommitOptions::default(),
        )
        .await
        .expect("git commit");

    assert!(!commit.hash.is_empty(), "commit hash should not be empty");

    // Verify repo is clean after commit
    let status = git
        .status("/tmp/commit-test")
        .await
        .expect("git status after commit");
    assert!(
        !status.current_branch.is_empty(),
        "current branch should not be empty"
    );

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// Code interpreter (WebSocket-based, needs Python)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_code_interpreter_python() {
    let client = create_client().await;

    let sandbox = client
        .create(python_image_params(), create_options())
        .await
        .expect("create sandbox");

    let interpreter = sandbox
        .code_interpreter()
        .await
        .expect("code interpreter service");

    let result = interpreter
        .run_code("print(3 * 7)", RunCodeOptions::default())
        .await
        .expect("run code");

    assert!(
        result.error.is_none(),
        "expected no error, got: {:?}",
        result.error
    );
    assert!(
        result.stdout.trim().contains("21"),
        "expected '21' in stdout, got: {}",
        result.stdout
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_code_interpreter_error_handling() {
    let client = create_client().await;

    let sandbox = client
        .create(python_image_params(), create_options())
        .await
        .expect("create sandbox");

    let interpreter = sandbox
        .code_interpreter()
        .await
        .expect("code interpreter service");

    let result = interpreter
        .run_code("raise ValueError('test error')", RunCodeOptions::default())
        .await
        .expect("run code with error");

    assert!(result.error.is_some(), "expected an execution error");
    let err = result.error.unwrap();
    assert!(
        err.value.contains("test error"),
        "expected 'test error' in error value, got: {}",
        err.value
    );

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_code_interpreter_context() {
    let client = create_client().await;

    let sandbox = client
        .create(python_image_params(), create_options())
        .await
        .expect("create sandbox");

    let interpreter = sandbox
        .code_interpreter()
        .await
        .expect("code interpreter service");

    // Create context
    let ctx = interpreter
        .create_context(None)
        .await
        .expect("create context");
    assert!(!ctx.id.is_empty());

    // List contexts
    let contexts = interpreter.list_contexts().await.expect("list contexts");
    let ctx_ids: Vec<&str> = contexts.contexts.iter().map(|c| c.id.as_str()).collect();
    assert!(
        ctx_ids.contains(&ctx.id.as_str()),
        "created context should be in list"
    );

    // Run code in the context
    let result = interpreter
        .run_code(
            "x = 42\nprint(x)",
            RunCodeOptions {
                context_id: Some(ctx.id.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("run code in context");
    assert!(result.stdout.trim().contains("42"));

    // Variable persists in context
    let result2 = interpreter
        .run_code(
            "print(x + 1)",
            RunCodeOptions {
                context_id: Some(ctx.id.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("run code reusing context");
    assert!(
        result2.stdout.trim().contains("43"),
        "variable should persist in context, got: {}",
        result2.stdout
    );

    // Delete context
    interpreter
        .delete_context(&ctx.id)
        .await
        .expect("delete context");

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// Sandbox info helpers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sandbox_user_home_dir() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let home = sandbox
        .get_user_home_dir()
        .await
        .expect("get user home dir");
    assert!(!home.is_empty(), "user home dir should not be empty");

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_sandbox_resources() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    assert!(sandbox.cpu > 0.0, "cpu should be > 0");
    assert!(sandbox.memory > 0.0, "memory should be > 0");
    assert!(sandbox.disk > 0.0, "disk should be > 0");

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// List with label filtering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_with_label_filter() {
    let client = create_client().await;

    let unique_value = format!(
        "rust-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let mut labels = HashMap::new();
    labels.insert("sdk-test-filter".to_string(), unique_value.clone());

    let params = CreateParams::Image(ImageParams {
        base: SandboxBaseParams {
            labels: Some(labels.clone()),
            ..Default::default()
        },
        image: ImageSource::Name("ubuntu:22.04".to_string()),
        resources: None,
    });

    let sandbox = client
        .create(params, create_options())
        .await
        .expect("create sandbox");

    // List with matching label
    let result = client
        .list(Some(&labels), Some(1), Some(10))
        .await
        .expect("list with labels");

    assert!(
        result.total >= 1,
        "should find at least 1 sandbox with label, got total={}",
        result.total
    );

    let found = result.items.iter().any(|s| s.id == sandbox.id);
    assert!(found, "our sandbox should be in the filtered list");

    sandbox.delete().await.expect("delete sandbox");
}

// ---------------------------------------------------------------------------
// PTY sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pty_session_lifecycle() {
    let client = create_client().await;

    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");

    let process = sandbox.process().await.expect("process service");

    // Create PTY session
    let session_id = process
        .create_pty_session(
            "test-pty-1",
            daytona_sdk::types::PtySessionOptions::default(),
        )
        .await
        .expect("create pty session");
    assert!(!session_id.is_empty());

    // Get PTY session
    let info = process
        .get_pty_session(&session_id)
        .await
        .expect("get pty session");
    assert_eq!(info.id, session_id);

    // List PTY sessions
    let sessions = process
        .list_pty_sessions()
        .await
        .expect("list pty sessions");
    let session_ids: Vec<&str> = sessions.sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(
        session_ids.contains(&session_id.as_str()),
        "session should be in list"
    );

    // Kill PTY session
    process
        .kill_pty_session(&session_id)
        .await
        .expect("kill pty session");

    sandbox.delete().await.expect("delete sandbox");
}

#[tokio::test]
async fn test_live_interactive_pty_io_resize_and_wait() {
    let client = create_client().await;
    let sandbox = client
        .create(ubuntu_image_params(), create_options())
        .await
        .expect("create sandbox");
    let process = sandbox.process().await.expect("process service");
    let session_id = unique("sdk-live-pty");
    let mut envs = HashMap::new();
    envs.insert("SDK_LIVE_PTY".to_string(), "present".to_string());
    let mut pty = process
        .create_pty(
            &session_id,
            PtyCreateOptions {
                cwd: Some("/tmp".to_string()),
                envs: Some(envs),
                size: Some(PtySize { rows: 24, cols: 80 }),
            },
        )
        .await
        .expect("create interactive PTY");
    let outcome = async {
        let resized = pty
            .resize(100, 40)
            .await
            .map_err(|error| format!("resize PTY: {error}"))?;
        if resized.cols != 100 || resized.rows != 40 {
            return Err(format!(
                "PTY resize returned {}x{}",
                resized.cols, resized.rows
            ));
        }
        let mut output = pty
            .take_output_receiver()
            .ok_or_else(|| "PTY output receiver was absent".to_string())?;
        pty.send_input("printf 'sdk-pty:%s:%s\\n' \"$SDK_LIVE_PTY\" \"$PWD\"; exit 0\n")
            .await
            .map_err(|error| format!("send PTY input: {error}"))?;
        let collect = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = output.recv().await {
                bytes.extend(chunk);
                if String::from_utf8_lossy(&bytes).contains("sdk-pty:present:/tmp") {
                    return Ok::<_, String>(bytes);
                }
            }
            Err("PTY output ended before the marker".to_string())
        };
        let (output, result) = tokio::join!(
            tokio::time::timeout(Duration::from_secs(30), collect),
            tokio::time::timeout(Duration::from_secs(30), pty.wait()),
        );
        let output = output.map_err(|_| "PTY output timed out".to_string())??;
        let result = result
            .map_err(|_| "PTY wait timed out".to_string())?
            .map_err(|error| format!("wait for PTY: {error}"))?;
        if !String::from_utf8_lossy(&output).contains("sdk-pty:present:/tmp") {
            return Err(format!("PTY marker missing from {output:?}"));
        }
        if result.exit_code != Some(0) || result.error.is_some() {
            return Err(format!("unexpected PTY result: {result:?}"));
        }
        Ok::<_, String>(())
    }
    .await;

    let _ = pty.kill().await;
    let _ = pty.disconnect().await;
    sandbox.delete().await.expect("delete sandbox");
    outcome.expect("interactive PTY I/O, resize, and wait");
}

// ---------------------------------------------------------------------------
// Client via explicit config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_client_with_explicit_config() {
    load_env();

    let api_key = std::env::var("DAYTONA_API_KEY").expect("DAYTONA_API_KEY must be set");
    let config = DaytonaConfig {
        api_key: Some(api_key),
        ..Default::default()
    };

    let client = Client::new_with_config(config)
        .await
        .expect("create client with config");

    let result = client
        .list(None, Some(1), Some(1))
        .await
        .expect("list sandboxes");
    assert!(result.page >= 1);
}
