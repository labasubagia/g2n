use g2n::adapters::notion::{NotionApi, NotionClient, NotionContext};
use std::io::Write;
use std::sync::Arc;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn page_exists_queries_name() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/v1/databases/.+/query$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [{"id": "p1"}]
        })))
        .mount(&server)
        .await;

    let ctx = NotionContext {
        api_key: "k".into(),
        database_id: "db1".into(),
    };
    let client = NotionClient::with_base_url(ctx, server.uri()).unwrap();
    assert!(client.page_exists("a.png").await.unwrap());
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
