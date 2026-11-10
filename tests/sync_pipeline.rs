use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use g2n::adapters::drive::{DriveApi, DriveError};
use g2n::adapters::notion::{NotionApi, NotionError};
use g2n::domain::models::{AppConfig, GoogleConfig, NotionConfig};
use g2n::services::sync::{SyncOptions, run_sync};
use serde_json::Value;
type AddedRecord = (String, String, Option<String>);
type AddedStore = Arc<Mutex<Vec<AddedRecord>>>;
struct FakeDrive {
    images: Vec<g2n::domain::models::DriveImage>,
}

#[async_trait]
impl DriveApi for FakeDrive {
    async fn list_images(
        &self,
        _folder: &str,
    ) -> Result<Vec<g2n::domain::models::DriveImage>, DriveError> {
        Ok(self.images.clone())
    }
    async fn download(&self, file_id: &str, _name: &str) -> Result<Vec<u8>, DriveError> {
        Ok(format!("bytes-{file_id}").into_bytes())
    }
}

#[derive(Clone)]
struct FakeNotion {
    exists: Arc<Mutex<Vec<String>>>,
    added: AddedStore,
}

#[async_trait]
impl NotionApi for FakeNotion {
    async fn list_existing_names(&self) -> Result<HashSet<String>, NotionError> {
        Ok(self.exists.lock().unwrap().iter().cloned().collect())
    }
    async fn add_page(
        &self,
        path: &Path,
        prompt: &str,
        created_time: Option<&str>,
    ) -> Result<Value, NotionError> {
        self.added.lock().unwrap().push((
            path.file_name().unwrap().to_str().unwrap().to_owned(),
            prompt.to_owned(),
            created_time.map(str::to_owned),
        ));
        Ok(Value::Null)
    }
}

fn config() -> AppConfig {
    AppConfig {
        google: GoogleConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            folder_id: "folder1".into(),
            scope: "scope".into(),
        },
        notion: NotionConfig {
            api_key: "k".into(),
            database_id: "db1".into(),
            requests_per_second: 3.0,
        },
    }
}

#[tokio::test]
async fn sync_uploads_new_and_skips_existing() {
    let images = vec![
        g2n::domain::models::DriveImage {
            id: "f1".into(),
            name: "a.png".into(),
            prompt: "cat".into(),
            created_time: Some("2024-01-01T00:00:00Z".into()),
            mime_type: "image/png".into(),
        },
        g2n::domain::models::DriveImage {
            id: "f2".into(),
            name: "b.png".into(),
            prompt: "dog".into(),
            created_time: None,
            mime_type: "image/png".into(),
        },
    ];
    let drive = FakeDrive { images };
    let notion = FakeNotion {
        exists: Arc::new(Mutex::new(vec!["a.png".to_owned()])),
        added: Arc::new(Mutex::new(vec![])),
    };
    let dir = tempfile::tempdir().unwrap();
    let options = SyncOptions {
        download_dir: dir.path().to_owned(),
        concurrency: 4,
    };

    run_sync(&options, &config(), &drive, &notion)
        .await
        .unwrap();

    let added = notion.added.lock().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].0, "b.png");
    assert_eq!(added[0].1, "dog");
    assert_eq!(added[0].2, None);
}

#[tokio::test]
async fn sync_errors_when_no_images() {
    let drive = FakeDrive { images: vec![] };
    let notion = FakeNotion {
        exists: Arc::new(Mutex::new(vec![])),
        added: Arc::new(Mutex::new(vec![])),
    };
    let dir = tempfile::tempdir().unwrap();
    let options = SyncOptions {
        download_dir: dir.path().to_owned(),
        concurrency: 4,
    };
    let err = run_sync(&options, &config(), &drive, &notion)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no images found"));
}
