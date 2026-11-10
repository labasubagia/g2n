use g2n::adapters::notion::{NotionApi, NotionClient, NotionContext};
use std::io::Write;
use std::sync::Arc;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn list_existing_names_returns_titles() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/v1/databases/.+/query$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [
                {"properties": {"Name": {"title": [{"text": {"content": "a.png"}}]}}},
                {"properties": {"Name": {"title": [{"text": {"content": "b.png"}}]}}}
            ],
            "has_more": false
        })))
        .mount(&server)
        .await;

    let ctx = NotionContext {
        api_key: "k".into(),
        database_id: "db1".into(),
    };
    let client = NotionClient::with_base_url(ctx, server.uri()).unwrap();
    let names = client.list_existing_names(&|_| {}).await.unwrap();
    assert!(names.contains("a.png"));
    assert!(names.contains("b.png"));
    assert_eq!(names.len(), 2);
}
#[tokio::test]
async fn list_existing_names_paginates() {
    let server = MockServer::start().await;
    let bodies: Arc<std::sync::Mutex<Vec<serde_json::Value>>> =
        Arc::new(std::sync::Mutex::new(vec![]));
    Mock::given(method("POST"))
        .and(path_regex("^/v1/databases/.+/query$"))
        .respond_with({
            let bodies = Arc::clone(&bodies);
            move |req: &wiremock::Request| {
                let mut bodies = bodies.lock().unwrap();
                bodies.push(serde_json::from_slice(&req.body).unwrap());
                if bodies.len() == 1 {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "results": [
                            {"properties": {"Name": {"title": [{"text": {"content": "a.png"}}]}}}
                        ],
                        "has_more": true,
                        "next_cursor": "cur1"
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "results": [
                            {"properties": {"Name": {"title": [{"text": {"content": "b.png"}}]}}}
                        ],
                        "has_more": false
                    }))
                }
            }
        })
        .mount(&server)
        .await;

    let ctx = NotionContext {
        api_key: "k".into(),
        database_id: "db1".into(),
    };
    let client = NotionClient::with_base_url(ctx, server.uri()).unwrap();
    let names = client.list_existing_names(&|_| {}).await.unwrap();
    assert_eq!(names.len(), 2);

    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert!(bodies[0].get("start_cursor").is_none());
    assert_eq!(bodies[1]["start_cursor"], "cur1");
}
#[tokio::test]
async fn add_page_uploads_and_creates_page_with_created_time() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/v1/file_uploads$"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "up1"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("^/v1/file_uploads/up1/send$"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "up1"
        })))
        .mount(&server)
        .await;

    let captured: Arc<std::sync::Mutex<Option<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(None));
    Mock::given(method("POST"))
        .and(path_regex("^/v1/pages$"))
        .respond_with({
            let captured = Arc::clone(&captured);
            move |req: &wiremock::Request| {
                *captured.lock().unwrap() = Some(req.body.clone());
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "page1"}))
            }
        })
        .mount(&server)
        .await;

    // Write a real image file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.png");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&[137, 80, 78, 71]).unwrap();
    drop(f);

    let ctx = NotionContext {
        api_key: "k".into(),
        database_id: "db1".into(),
    };
    let client = NotionClient::with_base_url(ctx, server.uri()).unwrap();
    let page = client
        .add_page(&path, "a cat", Some("2024-01-01T00:00:00Z"))
        .await
        .unwrap();
    assert_eq!(page["id"], "page1");

    let body: serde_json::Value =
        serde_json::from_slice(captured.lock().unwrap().as_ref().unwrap()).unwrap();
    assert_eq!(
        body["properties"]["Name"]["title"][0]["text"]["content"],
        "a.png"
    );
    assert_eq!(
        body["properties"]["Created Time"]["date"]["start"],
        "2024-01-01T00:00:00Z"
    );
    assert!(body["markdown"].as_str().unwrap().contains("a cat"));
}
