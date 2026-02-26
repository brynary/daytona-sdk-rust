# Daytona SDK for Rust

A Rust SDK for interacting with the [Daytona](https://daytona.io) platform, enabling you to create, manage, and interact with cloud development environments (sandboxes).

## Installation

Add the SDK to your `Cargo.toml`:

```toml
[dependencies]
daytona-sdk = { git = "https://github.com/brynary/daytona-sdk-rust", branch = "main" }
tokio = { version = "1", features = ["full"] }
```

For reproducible builds, you can specify a tag once releases are available:

```toml
[dependencies]
daytona-sdk = { git = "https://github.com/brynary/daytona-sdk-rust", tag = "v0.1.0" }
```

## Configuration

The SDK requires an API key for authentication. You can configure it via:

1. **Environment variable** (recommended):
   ```bash
   export DAYTONA_API_KEY=your_api_key_here
   ```

2. **Explicit configuration**:
   ```rust
   use daytona_sdk::{Client, DaytonaConfig};

   let config = DaytonaConfig {
       api_key: Some("your_api_key_here".to_string()),
       ..Default::default()
   };

   let client = Client::new_with_config(config).await?;
   ```

## Quick Start

```rust
use daytona_sdk::{Client, CreateParams, CreateSandboxOptions, SandboxState};
use daytona_sdk::types::{ImageParams, ImageSource, SandboxBaseParams, ExecuteCommandOptions};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client (uses DAYTONA_API_KEY env var)
    let client = Client::new().await?;

    // Create a sandbox from a Docker image
    let params = CreateParams::Image(ImageParams {
        base: SandboxBaseParams::default(),
        image: ImageSource::Name("ubuntu:22.04".to_string()),
        resources: None,
    });

    let options = CreateSandboxOptions {
        timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    };

    let sandbox = client.create(params, options).await?;
    println!("Created sandbox: {} (state: {:?})", sandbox.id, sandbox.state);

    // Execute a command
    let process = sandbox.process().await?;
    let result = process
        .execute_command("echo 'Hello from Daytona!'", ExecuteCommandOptions::default())
        .await?;
    println!("Output: {}", result.result);

    // Clean up
    sandbox.delete().await?;

    Ok(())
}
```

## Features

### Sandbox Management

```rust
// Create a sandbox
let sandbox = client.create(params, options).await?;

// Get a sandbox by ID
let sandbox = client.get("sandbox-id").await?;

// List sandboxes
let sandboxes = client.list(None, Some(1), Some(10)).await?;

// Find a sandbox by ID or name
let sandbox = client.find_one(Some("id"), None).await?;

// Stop and start
sandbox.stop().await?;
sandbox.start().await?;

// Delete
sandbox.delete().await?;
```

### Process Execution

```rust
let process = sandbox.process().await?;

// Execute a command
let result = process
    .execute_command("ls -la", ExecuteCommandOptions::default())
    .await?;

// Execute with working directory
let result = process
    .execute_command(
        "pwd",
        ExecuteCommandOptions {
            cwd: Some("/tmp".to_string()),
            ..Default::default()
        },
    )
    .await?;

// Session-based execution (preserves state between commands)
process.create_session("my-session").await?;
process.execute_session_command("my-session", "export MY_VAR=hello", false, false).await?;
process.execute_session_command("my-session", "echo $MY_VAR", false, true).await?;
process.delete_session("my-session").await?;
```

### Filesystem Operations

```rust
let fs = sandbox.filesystem().await?;

// List files
let files = fs.list_files("/tmp").await?;

// Create a folder
fs.create_folder("/tmp/my-folder", None).await?;

// Download a file
let content = fs.download_file("/path/to/file.txt").await?;

// Delete a file
fs.delete_file("/path/to/file.txt", false).await?;

// Move files
fs.move_files("/old/path", "/new/path").await?;

// Get file info
let info = fs.get_file_info("/path/to/file").await?;
```

### Git Operations

```rust
let git = sandbox.git().await?;

// Clone a repository
git.clone(
    "https://github.com/user/repo.git",
    "/workspace/repo",
    GitCloneOptions::default(),
).await?;

// Check status
let status = git.status("/workspace/repo").await?;

// List branches
let branches = git.branches("/workspace/repo").await?;

// Stage files
git.add("/workspace/repo", vec!["file.txt".to_string()]).await?;

// Commit
let commit = git.commit(
    "/workspace/repo",
    "Commit message",
    "Author Name",
    "author@example.com",
    GitCommitOptions::default(),
).await?;
```

### Code Interpreter (Python)

```rust
let interpreter = sandbox.code_interpreter().await?;

// Run Python code
let result = interpreter
    .run_code("print(2 + 2)", RunCodeOptions::default())
    .await?;
println!("Output: {}", result.stdout);

// Create a context for persistent state
let ctx = interpreter.create_context(None).await?;
interpreter.run_code(
    "x = 42",
    RunCodeOptions {
        context_id: Some(ctx.id.clone()),
        ..Default::default()
    },
).await?;

// Variable persists in context
let result = interpreter.run_code(
    "print(x)",
    RunCodeOptions {
        context_id: Some(ctx.id),
        ..Default::default()
    },
).await?;
```

### Labels and Environment Variables

```rust
use std::collections::HashMap;

// Create sandbox with labels
let mut labels = HashMap::new();
labels.insert("environment".to_string(), "development".to_string());

let params = CreateParams::Image(ImageParams {
    base: SandboxBaseParams {
        labels: Some(labels),
        ..Default::default()
    },
    image: ImageSource::Name("ubuntu:22.04".to_string()),
    resources: None,
});

// Create sandbox with environment variables
let mut env_vars = HashMap::new();
env_vars.insert("MY_VAR".to_string(), "my_value".to_string());

let params = CreateParams::Image(ImageParams {
    base: SandboxBaseParams {
        env_vars: Some(env_vars),
        ..Default::default()
    },
    image: ImageSource::Name("ubuntu:22.04".to_string()),
    resources: None,
});

// Filter sandboxes by labels
let sandboxes = client.list(Some(&labels), Some(1), Some(10)).await?;
```

## Crate Structure

This repository is organized as a Cargo workspace with the following crates:

- **`daytona-sdk`** (`libs/sdk-rust`): The main SDK crate providing high-level APIs
- **`daytona-api-client`** (`libs/api-client-rust`): Generated OpenAPI client for Daytona API
- **`daytona-toolbox-client`** (`libs/toolbox-api-client-rust`): Generated OpenAPI client for Daytona Toolbox API

## Running Tests

Integration tests require a valid `DAYTONA_API_KEY`:

```bash
# Copy the example env file and add your API key
cp .env.example .env
# Edit .env and set DAYTONA_API_KEY

# Run tests (serial execution required due to sandbox limits)
cargo test --test integration -- --test-threads=1
```

## License

This project is licensed under the MIT License - see the [LICENSE.txt](LICENSE.txt) file for details.
