use super::*;
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

async fn client(server: &MockServer, user_agent: Option<&str>) -> Client {
    Client::new_with_config(DaytonaConfig {
        api_key: Some("test-key".to_string()),
        organization_id: Some("org-1".to_string()),
        api_url: Some(format!("{}/api", server.uri())),
        user_agent: user_agent.map(str::to_string),
        http_client: Some(reqwest::Client::builder().no_proxy().build().unwrap()),
        ..Default::default()
    })
    .await
    .unwrap()
}

// Only fields required by SandboxListItem: deliberately no env, volumes, or buildInfo.
fn summary(id: &str) -> Value {
    json!({
        "id": id,
        "organizationId": "org-1",
        "name": "test-sandbox",
        "user": "daytona",
        "labels": {},
        "public": false,
        "networkBlockAll": false,
        "target": "us",
        "toolboxProxyUrl": "https://proxy.example.com/toolbox",
        "cpu": 2,
        "gpu": 0,
        "memory": 4,
        "disk": 20
    })
}

fn full_sandbox(id: &str) -> Value {
    let mut sandbox = summary(id);
    sandbox["env"] = json!({"APP_MODE": "production"});
    sandbox["state"] = json!("started");
    sandbox
}

fn page(items: impl Into<Vec<Value>>, next_cursor: Option<&str>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "items": items.into(),
        "nextCursor": next_cursor
    }))
}

fn at_cursor(mock: MockBuilder, cursor: Option<&str>) -> MockBuilder {
    match cursor {
        None => mock.and(query_param_is_missing("cursor")),
        Some(cursor) => mock.and(query_param("cursor", cursor)),
    }
}

#[tokio::test]
async fn list_uses_supported_path_and_preserves_headers_and_defaults() {
    for user_agent in [None, Some("embedding-app/9.9")] {
        let server = MockServer::start().await;
        let client = client(&server, user_agent).await;
        let expected_user_agent = user_agent
            .map(str::to_string)
            .unwrap_or_else(|| format!("daytona-sdk-rust/{SDK_VERSION}"));
        Mock::given(method("GET"))
            .and(path("/api/sandbox"))
            .and(header("authorization", "Bearer test-key"))
            .and(header("x-daytona-organization-id", "org-1"))
            .and(header("user-agent", expected_user_agent))
            .and(query_param_is_missing("page"))
            .and(query_param_is_missing("cursor"))
            .and(query_param_is_missing("labels"))
            .and(query_param_is_missing("limit"))
            .respond_with(page([summary("sb-1"), summary("sb-2")], None))
            .expect(1)
            .mount(&server)
            .await;

        // Annotated to check both types are re-exported at the crate root.
        let listed: crate::SandboxPage = client.list(None, None, None).await.unwrap();
        let ids: Vec<&str> = listed
            .items
            .iter()
            .map(|item: &crate::SandboxListItem| item.id.as_str())
            .collect();
        assert_eq!(ids, ["sb-1", "sb-2"]);
        assert!(listed.next_cursor.is_none());
        // No legacy request or eager hydration of inventory entries.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn list_round_trips_labels_and_opaque_cursors_including_empty_pages() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    let labels = HashMap::from([
        ("team/key".to_string(), "a+b & \"雪\"".to_string()),
        ("environment".to_string(), "prod=1%".to_string()),
    ]);
    let cursors = [" +/=?&%#雪 ", "", "eyJpZCI6InNiLTIifQ=="];
    let responses = [
        (None, page([summary("sb-1")], Some(cursors[0]))),
        (Some(cursors[0]), page([], Some(cursors[1]))),
        (Some(cursors[1]), page([summary("sb-2")], Some(cursors[2]))),
        (Some(cursors[2]), page([], None)),
    ];
    for (cursor, response) in responses {
        let mock = Mock::given(method("GET"))
            .and(path("/api/sandbox"))
            .and(query_param("limit", "2"))
            .and(query_param_is_missing("page"));
        at_cursor(mock, cursor)
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
    }

    let mut cursor = None;
    let mut ids = Vec::new();
    for expected_cursor in cursors.into_iter().map(Some).chain([None]) {
        let listed = client
            .list(Some(&labels), cursor.as_deref(), Some(2))
            .await
            .unwrap();
        assert_eq!(listed.next_cursor.as_deref(), expected_cursor);
        ids.extend(listed.items.into_iter().map(|item| item.id));
        cursor = listed.next_cursor;
    }
    assert_eq!(ids, ["sb-1", "sb-2"]);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for request in requests {
        let query: HashMap<_, _> = request.url.query_pairs().into_owned().collect();
        let decoded: HashMap<String, String> = serde_json::from_str(&query["labels"]).unwrap();
        assert_eq!(decoded, labels);
        let mut keys: Vec<_> = query.keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys.retain(|key| *key != "cursor");
        assert_eq!(keys, ["labels", "limit"]);
    }
}

#[tokio::test]
async fn list_accepts_limit_boundaries() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    for limit in [1, 100, 200] {
        Mock::given(path("/api/sandbox"))
            .and(query_param("limit", limit.to_string()))
            .respond_with(page([], None))
            .expect(1)
            .mount(&server)
            .await;
        client.list(None, None, Some(limit)).await.unwrap();
    }
}

#[tokio::test]
async fn list_rejects_invalid_limits_without_sending_requests() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    for limit in [i32::MIN, -1, 0, 201, i32::MAX] {
        let err = client.list(None, None, Some(limit)).await.unwrap_err();
        assert!(matches!(err, DaytonaError::General(_)));
        assert_eq!(err.message(), "limit must be between 1 and 200");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn list_and_find_one_propagate_api_errors() {
    for status in [400, 401, 404, 429, 500] {
        let server = MockServer::start().await;
        let client = client(&server, None).await;
        Mock::given(path("/api/sandbox"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "message": "listing failed"
            })))
            .expect(2)
            .mount(&server)
            .await;
        let list_err = client.list(None, None, None).await.unwrap_err();
        let find_err = client.find_one(None, None).await.unwrap_err();
        for err in [list_err, find_err] {
            assert_eq!(err.status_code(), Some(status));
            assert_eq!(err.message(), "listing failed");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn list_propagates_malformed_responses() {
    for body in [
        "not json".to_string(),
        json!({"items": []}).to_string(),
        json!({"items": [ {"id": "incomplete"} ], "nextCursor": null}).to_string(),
    ] {
        let server = MockServer::start().await;
        let client = client(&server, None).await;
        Mock::given(path("/api/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let err = client.list(None, None, None).await.unwrap_err();
        assert!(matches!(err, DaytonaError::General(_)));
    }
}

#[tokio::test]
async fn list_propagates_transport_errors() {
    let server = MockServer::start().await;
    let mut client = client(&server, None).await;
    // An unsupported scheme fails locally without contacting any cloud endpoint.
    client.api_config.base_path = "invalid://localhost".to_string();
    assert!(matches!(
        client.list(None, None, None).await.unwrap_err(),
        DaytonaError::General(_)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn list_all_follows_cursors_across_empty_pages() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    let labels = HashMap::from([("env".to_string(), "prod".to_string())]);
    let responses = [
        (None, page([summary("sb-1"), summary("sb-2")], Some("a"))),
        (Some("a"), page([], Some("b"))),
        (Some("b"), page([summary("sb-3")], None)),
    ];
    for (cursor, response) in responses {
        let mock = Mock::given(path("/api/sandbox"))
            .and(query_param("labels", r#"{"env":"prod"}"#))
            .and(query_param("limit", "2"));
        at_cursor(mock, cursor)
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
    }

    let items: Vec<_> = client
        .list_all(Some(&labels), Some(2))
        .try_collect()
        .await
        .unwrap();
    let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["sb-1", "sb-2", "sb-3"]);
}

#[tokio::test]
async fn list_all_yields_items_before_reporting_a_repeated_cursor() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    for (cursor, response) in [
        (None, page([summary("sb-1")], Some("a"))),
        (Some("a"), page([summary("sb-2")], Some("a"))),
    ] {
        at_cursor(Mock::given(path("/api/sandbox")), cursor)
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
    }

    let mut stream = pin!(client.list_all(None, None));
    assert_eq!(stream.try_next().await.unwrap().unwrap().id, "sb-1");
    assert_eq!(stream.try_next().await.unwrap().unwrap().id, "sb-2");
    let err = stream.try_next().await.unwrap_err();
    assert_eq!(err.message(), "sandbox listing returned a repeated cursor");
    assert!(stream.try_next().await.unwrap().is_none());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn list_all_rejects_invalid_limits_without_sending_requests() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    let mut stream = pin!(client.list_all(None, Some(0)));
    let err = stream.try_next().await.unwrap_err();
    assert_eq!(err.message(), "limit must be between 1 and 200");
    assert!(stream.try_next().await.unwrap().is_none());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn find_one_by_id_or_name_only_fetches_details() {
    for id_or_name in ["sb-1", "test-sandbox"] {
        let server = MockServer::start().await;
        let client = client(&server, None).await;
        Mock::given(method("GET"))
            .and(path(format!("/api/sandbox/{id_or_name}")))
            .and(query_param("verbose", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(full_sandbox("sb-1")))
            .expect(1)
            .mount(&server)
            .await;
        let labels = HashMap::from([("ignored".to_string(), "label".to_string())]);
        let sandbox = client
            .find_one(Some(id_or_name), Some(&labels))
            .await
            .unwrap();
        assert_eq!(sandbox.id, "sb-1");
        assert_eq!(sandbox.env.get("APP_MODE").unwrap(), "production");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn find_one_hydrates_first_match_after_empty_page() {
    let server = MockServer::start().await;
    let client = client(&server, Some("embedding-app/9.9")).await;
    let labels = HashMap::from([("env".to_string(), "prod".to_string())]);
    let responses = [
        (None, page([], Some("next+/=&"))),
        (
            Some("next+/=&"),
            page([summary("sb-match")], Some("do-not-fetch")),
        ),
    ];
    for (cursor, response) in responses {
        let mock = Mock::given(method("GET"))
            .and(path("/api/sandbox"))
            .and(query_param("labels", r#"{"env":"prod"}"#))
            .and(query_param_is_missing("limit"))
            .and(query_param_is_missing("page"));
        at_cursor(mock, cursor)
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/api/sandbox/sb-match"))
        .and(query_param("verbose", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(full_sandbox("sb-match")))
        .expect(1)
        .mount(&server)
        .await;

    // An empty ID retains the existing label-search behavior.
    let sandbox = client.find_one(Some(""), Some(&labels)).await.unwrap();
    assert_eq!(sandbox.id, "sb-match");
    assert_eq!(sandbox.env.get("APP_MODE").unwrap(), "production");
    assert_eq!(sandbox.state, Some(models::SandboxState::Started));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in requests {
        assert_eq!(request.headers["authorization"], "Bearer test-key");
        assert_eq!(request.headers["x-daytona-organization-id"], "org-1");
        assert_eq!(request.headers["user-agent"], "embedding-app/9.9");
    }
}

#[tokio::test]
async fn find_one_stops_when_no_match_remains() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    Mock::given(path("/api/sandbox"))
        .and(query_param_is_missing("limit"))
        .respond_with(page([], None))
        .expect(1)
        .mount(&server)
        .await;
    let err = client.find_one(None, None).await.unwrap_err();
    assert!(matches!(err, DaytonaError::NotFound { .. }));
    assert_eq!(err.message(), "no sandbox found matching criteria");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn find_one_propagates_hydration_errors_without_trying_another_match() {
    for status in [404, 429, 500] {
        let server = MockServer::start().await;
        let client = client(&server, None).await;
        Mock::given(path("/api/sandbox"))
            .respond_with(page([summary("sb-gone")], Some("do-not-fetch")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/sandbox/sb-gone"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "message": "details unavailable"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let err = client.find_one(None, None).await.unwrap_err();
        assert_eq!(err.status_code(), Some(status));
        assert_eq!(err.message(), "details unavailable");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn find_one_propagates_failure_after_an_empty_page() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    Mock::given(path("/api/sandbox"))
        .and(query_param_is_missing("cursor"))
        .respond_with(page([], Some("next")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/sandbox"))
        .and(query_param("cursor", "next"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "message": "temporarily unavailable"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let err = client.find_one(None, None).await.unwrap_err();
    assert_eq!(err.status_code(), Some(503));
    assert_eq!(err.message(), "temporarily unavailable");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn find_one_rejects_cursor_cycles() {
    let server = MockServer::start().await;
    let client = client(&server, None).await;
    for (cursor, next_cursor) in [(None, "a"), (Some("a"), "b"), (Some("b"), "a")] {
        at_cursor(Mock::given(path("/api/sandbox")), cursor)
            .respond_with(page([], Some(next_cursor)))
            .expect(1)
            .mount(&server)
            .await;
    }
    let err = client.find_one(None, None).await.unwrap_err();
    assert_eq!(err.message(), "sandbox listing returned a repeated cursor");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}
