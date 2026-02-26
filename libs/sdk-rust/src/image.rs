use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::PipInstallOptions;

/// A context file to send with the Docker build.
///
/// Matches Go SDK's `DockerImageContext` and TypeScript SDK's `Context` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImageContext {
    /// Local path to the file or directory.
    pub source_path: String,
    /// Path within the build context archive.
    pub archive_path: String,
}

/// Builder for constructing Docker images using a fluent API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImage {
    instructions: Vec<String>,
    #[serde(skip)]
    contexts: Vec<DockerImageContext>,
}

impl DockerImage {
    /// Create a new image from a base image.
    pub fn base(base_image: &str) -> Self {
        DockerImage {
            instructions: vec![format!("FROM {}", base_image)],
            contexts: Vec::new(),
        }
    }

    /// Create a new image from a Debian slim base with Python pre-installed.
    ///
    /// Defaults to Python 3.12 if no version is specified, matching Go/TypeScript
    /// SDK behavior. Uses `slim-bookworm` variant for consistency with reference SDKs.
    pub fn debian_slim(python_version: Option<&str>) -> Self {
        let version = python_version.unwrap_or("3.12");
        let base = format!("python:{}-slim-bookworm", version);
        DockerImage {
            instructions: vec![format!("FROM {}", base)],
            contexts: Vec::new(),
        }
    }

    /// Create from an existing Dockerfile string.
    pub fn from_dockerfile(dockerfile: &str) -> Self {
        DockerImage {
            instructions: dockerfile.lines().map(|l| l.to_string()).collect(),
            contexts: Vec::new(),
        }
    }

    /// Add apt-get packages.
    pub fn apt_get(mut self, packages: &[&str]) -> Self {
        let pkg_list = packages.join(" ");
        self.instructions.push(format!(
            "RUN apt-get update && apt-get install -y {} && rm -rf /var/lib/apt/lists/*",
            pkg_list
        ));
        self
    }

    /// Add pip install packages.
    ///
    /// Matches Go SDK `PipInstall` behavior: uses `pip install` without
    /// `--no-cache-dir` (users can add it via `extra_options` if desired).
    pub fn pip_install(mut self, packages: &[&str], options: PipInstallOptions) -> Self {
        let mut cmd = String::from("RUN pip install");

        if let Some(ref find_links) = options.find_links {
            for link in find_links {
                cmd.push_str(&format!(" --find-links {}", link));
            }
        }
        if let Some(ref index_url) = options.index_url {
            cmd.push_str(&format!(" --index-url {}", index_url));
        }
        if let Some(ref extra_urls) = options.extra_index_urls {
            for url in extra_urls {
                cmd.push_str(&format!(" --extra-index-url {}", url));
            }
        }
        if options.pre == Some(true) {
            cmd.push_str(" --pre");
        }
        if let Some(ref extra) = options.extra_options {
            cmd.push_str(&format!(" {}", extra));
        }

        cmd.push(' ');
        cmd.push_str(&packages.join(" "));
        self.instructions.push(cmd);
        self
    }

    /// Add a RUN instruction.
    pub fn run(mut self, command: &str) -> Self {
        self.instructions.push(format!("RUN {}", command));
        self
    }

    /// Set an environment variable.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.instructions.push(format!("ENV {}={}", key, value));
        self
    }

    /// Set the working directory.
    pub fn workdir(mut self, dir: &str) -> Self {
        self.instructions.push(format!("WORKDIR {}", dir));
        self
    }

    /// Set the entrypoint.
    ///
    /// Uses JSON serialization for proper escaping, matching Go SDK's
    /// `json.Marshal(cmd)` approach.
    pub fn entrypoint(mut self, args: &[&str]) -> Self {
        let json_array = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
        self.instructions
            .push(format!("ENTRYPOINT {}", json_array));
        self
    }

    /// Set the CMD instruction.
    pub fn cmd(mut self, args: &[&str]) -> Self {
        let json_args: Vec<String> = args.iter().map(|a| format!("\"{}\"", a)).collect();
        self.instructions
            .push(format!("CMD [{}]", json_args.join(", ")));
        self
    }

    /// Set the user.
    pub fn user(mut self, user: &str) -> Self {
        self.instructions.push(format!("USER {}", user));
        self
    }

    /// Add a COPY instruction.
    pub fn copy(mut self, src: &str, dst: &str) -> Self {
        self.instructions.push(format!("COPY {} {}", src, dst));
        self
    }

    /// Add an ADD instruction.
    pub fn add(mut self, src: &str, dst: &str) -> Self {
        self.instructions.push(format!("ADD {} {}", src, dst));
        self
    }

    /// Expose one or more ports.
    ///
    /// Matches Go SDK `Expose([]int)` behavior of accepting multiple ports
    /// in a single call.
    pub fn expose(mut self, ports: &[u16]) -> Self {
        let port_strs: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
        self.instructions
            .push(format!("EXPOSE {}", port_strs.join(" ")));
        self
    }

    /// Add a LABEL instruction.
    pub fn label(mut self, key: &str, value: &str) -> Self {
        self.instructions
            .push(format!("LABEL {}=\"{}\"", key, value));
        self
    }

    /// Add a VOLUME instruction.
    pub fn volume(mut self, vol_path: &str) -> Self {
        self.instructions.push(format!("VOLUME [\"{}\"]", vol_path));
        self
    }

    /// Add a local file to the build context.
    pub fn add_local_file(mut self, local_path: &str, container_path: &str) -> Self {
        let filename = PathBuf::from(local_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        self.contexts.push(DockerImageContext {
            source_path: local_path.to_string(),
            archive_path: filename.clone(),
        });
        self.instructions
            .push(format!("COPY {} {}", filename, container_path));
        self
    }

    /// Add a local directory to the build context.
    pub fn add_local_dir(mut self, local_path: &str, container_path: &str) -> Self {
        let dirname = PathBuf::from(local_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "dir".to_string());

        self.contexts.push(DockerImageContext {
            source_path: local_path.to_string(),
            archive_path: dirname.clone(),
        });
        self.instructions
            .push(format!("COPY {} {}", dirname, container_path));
        self
    }

    /// Get the generated Dockerfile content.
    pub fn dockerfile(&self) -> String {
        self.instructions.join("\n")
    }

    /// Get the build contexts (files to send with the build).
    pub fn contexts(&self) -> &[DockerImageContext] {
        &self.contexts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_image() {
        let img = DockerImage::base("ubuntu:22.04");
        assert_eq!(img.dockerfile(), "FROM ubuntu:22.04");
    }

    #[test]
    fn test_debian_slim() {
        let img = DockerImage::debian_slim(Some("3.11"));
        assert_eq!(img.dockerfile(), "FROM python:3.11-slim-bookworm");
    }

    #[test]
    fn test_debian_slim_default_version() {
        let img = DockerImage::debian_slim(None);
        assert_eq!(img.dockerfile(), "FROM python:3.12-slim-bookworm");
    }

    #[test]
    fn test_from_dockerfile() {
        let dockerfile = "FROM alpine\nRUN apk add git";
        let img = DockerImage::from_dockerfile(dockerfile);
        assert_eq!(img.dockerfile(), dockerfile);
    }

    #[test]
    fn test_builder_chaining() {
        let img = DockerImage::base("ubuntu:22.04")
            .apt_get(&["curl", "git"])
            .pip_install(&["numpy", "pandas"], PipInstallOptions::default())
            .run("echo 'hello'")
            .env("APP_ENV", "production")
            .workdir("/app");

        let df = img.dockerfile();
        assert!(df.contains("FROM ubuntu:22.04"));
        assert!(df.contains("apt-get update && apt-get install -y curl git"));
        assert!(df.contains("pip install numpy pandas"));
        assert!(df.contains("RUN echo 'hello'"));
        assert!(df.contains("ENV APP_ENV=production"));
        assert!(df.contains("WORKDIR /app"));
    }

    #[test]
    fn test_pip_install_with_options() {
        let opts = PipInstallOptions {
            index_url: Some("https://custom.pypi.org/simple".to_string()),
            pre: Some(true),
            ..Default::default()
        };
        let img = DockerImage::base("python:3.11").pip_install(&["torch"], opts);
        let df = img.dockerfile();
        assert!(df.contains("--index-url https://custom.pypi.org/simple"));
        assert!(df.contains("--pre"));
        assert!(df.contains("torch"));
    }

    #[test]
    fn test_entrypoint_and_cmd() {
        let img = DockerImage::base("python:3.11")
            .entrypoint(&["python", "-m"])
            .cmd(&["myapp"]);
        let df = img.dockerfile();
        assert!(df.contains("ENTRYPOINT [\"python\",\"-m\"]"));
        assert!(df.contains("CMD [\"myapp\"]"));
    }

    #[test]
    fn test_add_local_file() {
        let img =
            DockerImage::base("python:3.11").add_local_file("/tmp/config.yaml", "/app/config.yaml");
        let df = img.dockerfile();
        assert!(df.contains("COPY config.yaml /app/config.yaml"));
        assert_eq!(img.contexts().len(), 1);
        assert_eq!(img.contexts()[0].source_path, "/tmp/config.yaml");
        assert_eq!(img.contexts()[0].archive_path, "config.yaml");
    }

    #[test]
    fn test_add_local_dir() {
        let img = DockerImage::base("python:3.11").add_local_dir("/tmp/myproject", "/app");
        let df = img.dockerfile();
        assert!(df.contains("COPY myproject /app"));
        assert_eq!(img.contexts().len(), 1);
    }

    #[test]
    fn test_expose_and_label() {
        let img = DockerImage::base("nginx")
            .expose(&[8080])
            .label("version", "1.0")
            .volume("/data");
        let df = img.dockerfile();
        assert!(df.contains("EXPOSE 8080"));
        assert!(df.contains("LABEL version=\"1.0\""));
        assert!(df.contains("VOLUME [\"/data\"]"));
    }

    #[test]
    fn test_expose_multiple_ports() {
        let img = DockerImage::base("nginx").expose(&[8080, 8443]);
        let df = img.dockerfile();
        assert!(df.contains("EXPOSE 8080 8443"));
    }

    #[test]
    fn test_user_copy_add() {
        let img = DockerImage::base("ubuntu")
            .user("appuser")
            .copy("./src", "/app/src")
            .add("https://example.com/file.tar.gz", "/tmp/");
        let df = img.dockerfile();
        assert!(df.contains("USER appuser"));
        assert!(df.contains("COPY ./src /app/src"));
        assert!(df.contains("ADD https://example.com/file.tar.gz /tmp/"));
    }

    #[test]
    fn test_empty_contexts() {
        let img = DockerImage::base("ubuntu:22.04").run("echo test");
        assert!(img.contexts().is_empty());
    }

    #[test]
    fn test_full_complex_image() {
        let img = DockerImage::base("python:3.11-slim")
            .apt_get(&["build-essential", "libpq-dev"])
            .workdir("/app")
            .copy("requirements.txt", "/app/requirements.txt")
            .pip_install(&["-r", "requirements.txt"], PipInstallOptions::default())
            .copy(".", "/app")
            .env("PYTHONUNBUFFERED", "1")
            .expose(&[8000])
            .user("appuser")
            .entrypoint(&["python"])
            .cmd(&["manage.py", "runserver"]);

        let df = img.dockerfile();
        let lines: Vec<&str> = df.lines().collect();
        assert_eq!(lines[0], "FROM python:3.11-slim");
        assert!(lines.len() >= 10);
    }

    #[test]
    fn test_pip_install_find_links_multiple() {
        let opts = PipInstallOptions {
            find_links: Some(vec![
                "https://mirror1.example.com".to_string(),
                "https://mirror2.example.com".to_string(),
            ]),
            ..Default::default()
        };
        let img = DockerImage::base("python:3.11").pip_install(&["numpy"], opts);
        let df = img.dockerfile();
        assert!(df.contains("--find-links https://mirror1.example.com"));
        assert!(df.contains("--find-links https://mirror2.example.com"));
    }
}