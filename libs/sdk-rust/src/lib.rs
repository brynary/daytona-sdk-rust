pub mod client;
pub mod code_interpreter;
pub mod computer_use;
pub mod config;
pub mod error;
pub mod filesystem;
pub mod git;
pub mod image;
pub mod lsp;
pub mod process;
pub mod sandbox;
pub mod snapshot;
pub mod types;
pub mod volume;

// Core
pub use client::Client;
pub use config::DaytonaConfig;
pub use error::DaytonaError;
pub use sandbox::Sandbox;

// Re-export SandboxState from generated client (matches Go/TS enum usage)
pub use daytona_api_client::models::SandboxState;

// Re-export sandbox-related enums exposed on the Sandbox struct
pub use daytona_api_client::models::sandbox::BackupState;

// Types re-exports (mirrors TypeScript index.ts exports)
pub use types::{
    CodeLanguage, CreateParams, CreateSandboxOptions, CreateSnapshotParams, ExecuteCommandOptions,
    ExecuteResponse, GitCloneOptions, GitCommitOptions, GitDeleteBranchOptions, GitPullOptions,
    GitPushOptions, ImageParams, ImageSource, PaginatedSandboxes, PreviewLink, PtySessionOptions,
    PtySize, Resources, RunCodeOptions, SandboxBaseParams, ScreenshotOptions, ScreenshotRegion,
    SetFilePermissionsOptions, SnapshotParams, VolumeMount,
};

// Service re-exports
pub use code_interpreter::{CodeInterpreterService, ExecutionError, ExecutionResult, OutputMessage};
pub use computer_use::{
    ComputerUseService, DisplayService, KeyboardService, MouseService, RecordingService,
    ScreenshotService,
};
pub use filesystem::FileSystemService;
pub use git::GitService;
pub use image::DockerImage;
pub use lsp::LspService;
pub use process::{ProcessService, SessionExecuteResult};
pub use snapshot::SnapshotService;
pub use volume::VolumeService;

// Re-export generated toolbox client types used in public API return types.
// This mirrors the TypeScript SDK's `export type { FileInfo, GitStatus, ... }`
// from `@daytonaio/toolbox-api-client`, allowing SDK users to reference these
// types without adding `daytona-toolbox-client` as a direct dependency.
pub mod toolbox_types {
    pub use daytona_toolbox_client::models::{
        Command, CompletionItem, CompletionList, FileInfo, GitCommitResponse, GitStatus,
        InterpreterContext, ListBranchResponse, ListContextsResponse, LspSymbol, Match,
        PtyListResponse, PtySessionInfo, Recording, ReplaceResult, SearchFilesResponse, Session,
    };
}

// Re-export generated API client types used in public API return types.
// This allows SDK users to reference types like `VolumeDto` and `SnapshotDto`
// without adding `daytona-api-client` as a direct dependency.
pub mod api_types {
    pub use daytona_api_client::models::{
        PaginatedSnapshots, SignedPortPreviewUrl, SnapshotDto, SshAccessDto,
        SshAccessValidationDto, VolumeDto, VolumeState,
    };
}