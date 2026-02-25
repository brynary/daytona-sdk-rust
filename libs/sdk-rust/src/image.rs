use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::types::PipInstallOptions;

/// A context file to send with the Docker build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImageContext {
    pub source: String,
    pub destination: String,
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
    pub fn debian_slim(python_version: &str) -> Self {
        let base = format!("python:{}-slim", python_version);
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
    pub fn pip_install(mut self, packages: &[&str], options: PipInstallOptions) -> Self {
        let mut cmd = String::from("RUN pip install --no-cache-dir");

        if let Some(ref find_links) = options.find_links {
            cmd.push_str(&format!(" --find-links={}", find_links));
        }
        if let Some(ref index_url) = options.index_url {
            cmd.push_str(&format!(" --index-url={}", index_url));
        }
        if let Some(ref extra_urls) = options.extra_index_urls {
            for url in extra_urls {
                cmd.push_str(&format!(" --extra-index-url={}", url));
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
    pub fn entrypoint(mut self, args: &[&str]) -> Self {
        let json_args: Vec<String> = args.iter().map(|a| format!("\"{}\"", a)).collect();
        self.instructions
            .push(format!("ENTRYPOINT [{}]", json_args.join(", ")));
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

    /// Expose a port.
    pub fn expose(mut self, port: u16) -> Self {
        self.instructions.push(format!("EXPOSE {}", port));
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
            source: local_path.to_string(),
            destination: filename.clone(),
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
            source: local_path.to_string(),
            destination: dirname.clone(),
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
        let img = DockerImage::debian_slim("3.11");
        assert_eq!(img.dockerfile(), "FROM python:3.11-slim");
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
        assert!(df.contains("pip install --no-cache-dir numpy pandas"));
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
        assert!(df.contains("--index-url=https://custom.pypi.org/simple"));
        assert!(df.contains("--pre"));
        assert!(df.contains("torch"));
    }

    #[test]
    fn test_entrypoint_and_cmd() {
        let img = DockerImage::base("python:3.11")
            .entrypoint(&["python", "-m"])
            .cmd(&["myapp"]);
        let df = img.dockerfile();
        assert!(df.contains("ENTRYPOINT [\"python\", \"-m\"]"));
        assert!(df.contains("CMD [\"myapp\"]"));
    }

    #[test]
    fn test_add_local_file() {
        let img =
            DockerImage::base("python:3.11").add_local_file("/tmp/config.yaml", "/app/config.yaml");
        let df = img.dockerfile();
        assert!(df.contains("COPY config.yaml /app/config.yaml"));
        assert_eq!(img.contexts().len(), 1);
        assert_eq!(img.contexts()[0].source, "/tmp/config.yaml");
        assert_eq!(img.contexts()[0].destination, "config.yaml");
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
            .expose(8080)
            .label("version", "1.0")
            .volume("/data");
        let df = img.dockerfile();
        assert!(df.contains("EXPOSE 8080"));
        assert!(df.contains("LABEL version=\"1.0\""));
        assert!(df.contains("VOLUME [\"/data\"]"));
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
            .expose(8000)
            .user("appuser")
            .entrypoint(&["python"])
            .cmd(&["manage.py", "runserver"]);

        let df = img.dockerfile();
        let lines: Vec<&str> = df.lines().collect();
        assert_eq!(lines[0], "FROM python:3.11-slim");
        assert!(lines.len() >= 10);
    }
}
