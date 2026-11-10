use g2n::adapters::drive::{DriveApi, DriveClient};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[tokio::test]
async fn lists_images_with_description_and_created_time() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("^/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "file1",
                    "name": "a.png",
                    "description": "a cat in a hat",
                    "mimeType": "image/png",
                    "createdTime": "2024-01-01T00:00:00Z"
                },
                {
                    "id": "file2",
                    "name": "b.jpg",
                    "description": "",
                    "mimeType": "image/jpeg",
                    "createdTime": null
                }
            ]
        })))
        .mount(&server)
        .await;

    let client = DriveClient::with_base_url("token".to_owned(), server.uri()).unwrap();
    let images = client.list_images("folder1").await.unwrap();

    assert_eq!(images.len(), 2);
    assert_eq!(images[0].id, "file1");
    assert_eq!(images[0].prompt, "a cat in a hat");
    assert_eq!(
        images[0].created_time.as_deref(),
        Some("2024-01-01T00:00:00Z")
    );
    assert_eq!(images[1].prompt, "");
}

#[tokio::test]
async fn downloads_image_bytes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("^/drive/v3/files/file1$"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
        .mount(&server)
        .await;

    let client = DriveClient::with_base_url("token".to_owned(), server.uri()).unwrap();
    let bytes = client.download("file1", "a.png").await.unwrap();
    assert_eq!(bytes, vec![1, 2, 3]);
}
#[tokio::test]
async fn lists_images_recursively_for_nested_folders() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_mock = Arc::clone(&calls);
    Mock::given(method("GET"))
        .and(path_regex("^/drive/v3/files"))
        .respond_with(move |_: &Request| {
            let call = calls_mock.fetch_add(1, Ordering::SeqCst);
            let body = match call {
                0 => serde_json::json!({
                    "files": [
                        {"id": "file1", "name": "a.png", "description": "top", "mimeType": "image/png", "createdTime": null},
                        {"id": "fld1", "name": "sub", "description": "", "mimeType": "application/vnd.google-apps.folder", "createdTime": null}
                    ]
                }),
                1 => serde_json::json!({
                    "files": [
                        {"id": "file2", "name": "b.jpg", "description": "deep", "mimeType": "image/jpeg", "createdTime": null}
                    ]
                }),
                _ => serde_json::json!({ "files": [] }),
            };
            ResponseTemplate::new(200).set_body_json(body)
        })
        .mount(&server)
        .await;

    let client = DriveClient::with_base_url("token".to_owned(), server.uri()).unwrap();
    let images = client.list_images("folder1").await.unwrap();

    let names: Vec<&str> = images.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["a.png", "b.jpg"]);
    assert_eq!(images[1].prompt, "deep");
    assert!(calls.load(Ordering::SeqCst) >= 2);
}
