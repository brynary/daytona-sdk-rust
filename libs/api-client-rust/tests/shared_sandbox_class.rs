//! Guards the postprocess step that aliases the spec's inline sandbox class
//! enums to the shared `models::SandboxClass`.

use daytona_api_client::models::{Sandbox, SandboxClass, SandboxListItem, SnapshotDto};

// Each accessor only compiles while the model's field is the shared type.
fn sandbox_class(sandbox: &Sandbox) -> Option<SandboxClass> {
    sandbox.sandbox_class
}

fn list_item_class(item: &SandboxListItem) -> Option<SandboxClass> {
    item.sandbox_class
}

fn snapshot_class(snapshot: &SnapshotDto) -> Option<SandboxClass> {
    snapshot.sandbox_class
}

#[test]
fn models_share_one_sandbox_class() {
    let sandbox = Sandbox {
        sandbox_class: Some(SandboxClass::CONTAINER),
        ..Default::default()
    };
    let item = SandboxListItem {
        sandbox_class: Some(SandboxClass::LINUX_VM),
        ..Default::default()
    };
    let snapshot = SnapshotDto {
        sandbox_class: Some(SandboxClass::WINDOWS),
        ..Default::default()
    };

    assert_eq!(sandbox_class(&sandbox), Some(SandboxClass::CONTAINER));
    assert_eq!(list_item_class(&item), Some(SandboxClass::LINUX_VM));
    assert_eq!(snapshot_class(&snapshot), Some(SandboxClass::WINDOWS));
}

#[test]
fn inline_module_paths_name_the_shared_type() {
    let from_sandbox: daytona_api_client::models::sandbox::SandboxClass = SandboxClass::ANDROID;
    let from_snapshot: daytona_api_client::models::snapshot_dto::SandboxClass =
        SandboxClass::ANDROID;
    assert_eq!(from_sandbox, from_snapshot);
}
