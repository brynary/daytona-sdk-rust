use daytona_api_client::models::{ListSandboxesResponse, Sandbox, SandboxListItem};
use serde_json::json;

#[test]
fn list_items_deserialize_without_full_sandbox_fields() {
    let item = json!({
        "id": "sb-1",
        "organizationId": "org-1",
        "name": "sandbox-1",
        "target": "us",
        "user": "daytona",
        "public": false,
        "networkBlockAll": false,
        "cpu": 2,
        "gpu": 0,
        "memory": 4,
        "disk": 20,
        "labels": {},
        "toolboxProxyUrl": "https://proxy.example.com/toolbox"
    });
    let summary: SandboxListItem = serde_json::from_value(item.clone()).unwrap();
    assert_eq!(summary.id, "sb-1");
    assert!(summary.state.is_none());
    assert!(serde_json::from_value::<Sandbox>(item.clone()).is_err());

    let page: ListSandboxesResponse = serde_json::from_value(json!({
        "items": [item], "nextCursor": "opaque+/=&%雪"
    }))
    .unwrap();
    assert_eq!(page.items, [summary]);
    assert_eq!(page.next_cursor.as_deref(), Some("opaque+/=&%雪"));
    let round_trip: ListSandboxesResponse =
        serde_json::from_value(serde_json::to_value(&page).unwrap()).unwrap();
    assert_eq!(round_trip, page);
}

#[test]
fn next_cursor_is_required_but_nullable() {
    let page: ListSandboxesResponse = serde_json::from_value(json!({
        "items": [], "nextCursor": null
    }))
    .unwrap();
    assert!(page.next_cursor.is_none());
    assert_eq!(
        serde_json::to_value(page).unwrap()["nextCursor"],
        json!(null)
    );
    assert!(serde_json::from_value::<ListSandboxesResponse>(json!({"items": []})).is_err());
    assert!(serde_json::from_value::<ListSandboxesResponse>(json!({"nextCursor": null})).is_err());
}
