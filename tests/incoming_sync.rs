use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU16, Ordering},
};

use async_trait::async_trait;
use brooklet::{
    api::miniflux::{CategoryDto, EntriesDto, EntryDto, EntryQuery, FeedDto, ServerIdentity},
    error::BrookletError,
    model::{Account, MutationField, classify_http_status},
    services::traits::{MinifluxApi, Repository, SecretStore},
    storage::sqlite::SqliteRepository,
    sync::{AccountSyncService, MinifluxApiFactory, SyncService},
};

#[derive(Default)]
struct Server {
    read_status: AtomicU16,
    star_status: AtomicU16,
    save_status: AtomicU16,
    pull_status: AtomicU16,
    calls: Mutex<Vec<String>>,
}

impl Server {
    fn request(&self, name: &str, status: &AtomicU16) -> Result<(), BrookletError> {
        self.calls.lock().unwrap().push(name.into());
        let status = status.load(Ordering::Acquire);
        if status == 0 {
            Ok(())
        } else {
            Err(BrookletError::Http {
                status,
                kind: classify_http_status(status),
            })
        }
    }
}

fn story(id: i64) -> EntryDto {
    EntryDto {
        id,
        feed_id: 7,
        title: format!("Story {id}"),
        url: format!("https://example.org/{id}"),
        author: None,
        published_at: "2026-10-03T12:00:00Z".into(),
        changed_at: "2026-10-03T12:00:00Z".into(),
        content: "<p>Remote content</p>".into(),
        status: "unread".into(),
        starred: false,
        reading_time: 1,
        feed: None,
    }
}

struct Api(Arc<Server>);

#[async_trait]
impl MinifluxApi for Api {
    async fn validate(&self) -> Result<ServerIdentity, BrookletError> {
        unreachable!()
    }
    async fn categories(&self) -> Result<Vec<CategoryDto>, BrookletError> {
        Ok(vec![])
    }
    async fn feeds(&self) -> Result<Vec<FeedDto>, BrookletError> {
        Ok(vec![])
    }
    async fn entries(&self, _: &EntryQuery) -> Result<EntriesDto, BrookletError> {
        self.0.request("pull", &self.0.pull_status)?;
        Ok(EntriesDto {
            total: 2,
            entries: vec![story(42), story(99)],
        })
    }
    async fn entry(&self, _: i64) -> Result<EntryDto, BrookletError> {
        unreachable!()
    }
    async fn set_read(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
        self.0.request("read", &self.0.read_status)
    }
    async fn set_starred(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
        self.0.request("star", &self.0.star_status)
    }
    async fn save_to_integration(&self, _: i64) -> Result<(), BrookletError> {
        self.0.request("save", &self.0.save_status)
    }
    async fn refresh_feeds(&self) -> Result<(), BrookletError> {
        self.0.calls.lock().unwrap().push("refresh".into());
        Ok(())
    }
    async fn subscribe(&self, _: &str, _: Option<i64>) -> Result<i64, BrookletError> {
        unreachable!()
    }
}

struct Factory(Arc<Server>);
impl MinifluxApiFactory for Factory {
    fn create(&self, _: &str, _: String) -> Result<Box<dyn MinifluxApi>, BrookletError> {
        Ok(Box::new(Api(self.0.clone())))
    }
}

struct Secrets;
#[async_trait]
impl SecretStore for Secrets {
    async fn store_miniflux_token(&self, _: i64, _: &str) -> Result<(), BrookletError> {
        Ok(())
    }
    async fn load_miniflux_token(&self, _: i64) -> Result<Option<String>, BrookletError> {
        Ok(Some("test".into()))
    }
    async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
        Ok(())
    }
}

async fn fixture() -> (Arc<SqliteRepository>, AccountSyncService, Arc<Server>) {
    let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
    fixture_with_repository(repo).await
}

async fn fixture_with_repository(
    repo: Arc<SqliteRepository>,
) -> (Arc<SqliteRepository>, AccountSyncService, Arc<Server>) {
    repo.save_account(&Account {
        id: 1,
        server_url: "https://miniflux.example".into(),
        username: "reader".into(),
        server_version: "2.3.2".into(),
    })
    .await
    .unwrap();
    let server = Arc::new(Server::default());
    let service = AccountSyncService::new(
        repo.clone(),
        Arc::new(Secrets),
        Arc::new(Factory(server.clone())),
    );
    service.sync().await.unwrap();
    server.calls.lock().unwrap().clear();
    (repo, service, server)
}

#[tokio::test]
async fn failed_read_upload_still_imports_articles_and_protects_local_intentions() {
    let (repo, service, server) = fixture().await;
    service.set_read_local(42, true).await.unwrap();
    service.set_starred_local(42, true).await.unwrap();
    // Remove a previously cached row so its return proves an incoming merge.
    repo.merge_changed_page(1, &[], &[99]).await.unwrap();
    server.read_status.store(503, Ordering::Release);
    let result = service.sync().await.unwrap();
    assert!(result.inbox.iter().any(|entry| entry.id == 99));
    let local = repo.cached_entry(1, 42).await.unwrap().unwrap();
    assert!(local.read && local.starred);
    assert_eq!(repo.pending_mutations(1).await.unwrap().len(), 2);
    assert_eq!(*server.calls.lock().unwrap(), ["read", "pull"]);
    let status = repo.sync_status(1).await.unwrap();
    assert!(status.last_successful_sync_at_ms.is_some());
    assert!(status.error.unwrap().contains("503"));
    // A later successful attempt drains the queue and clears only its error.
    server.read_status.store(0, Ordering::Release);
    server.calls.lock().unwrap().clear();
    service.sync().await.unwrap();
    assert!(repo.pending_mutations(1).await.unwrap().is_empty());
    assert_eq!(repo.sync_status(1).await.unwrap().error, None);
    assert_eq!(*server.calls.lock().unwrap(), ["read", "star", "pull"]);
}

#[tokio::test]
async fn failed_star_upload_keeps_only_unsent_intention_after_successful_pull() {
    let (repo, service, server) = fixture().await;
    service.set_read_local(42, true).await.unwrap();
    service.set_starred_local(42, true).await.unwrap();
    server.star_status.store(503, Ordering::Release);
    service.sync().await.unwrap();
    let pending = repo.pending_mutations(1).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].field, MutationField::Starred);
    assert!(repo.cached_entry(1, 42).await.unwrap().unwrap().starred);
    assert_eq!(*server.calls.lock().unwrap(), ["read", "star", "pull"]);
    server.star_status.store(0, Ordering::Release);
    server.calls.lock().unwrap().clear();
    service.sync().await.unwrap();
    assert_eq!(*server.calls.lock().unwrap(), ["star", "pull"]);
}

#[tokio::test]
async fn karakeep_outage_does_not_block_pull_or_requested_feed_refresh() {
    let (repo, service, server) = fixture().await;
    let entry = repo.cached_entry(1, 42).await.unwrap().unwrap();
    service.queue_karakeep(&entry).await.unwrap();
    server.save_status.store(503, Ordering::Release);
    let result = service.refresh_feeds().await.unwrap();
    assert_eq!(result.fetched, 2);
    let calls = server.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert!(calls[..2].contains(&"save".into()) && calls[..2].contains(&"refresh".into()));
    assert_eq!(calls[2], "pull");
    assert_eq!(repo.pending_karakeep(1).await.unwrap().len(), 1);
    assert!(
        repo.sync_status(1)
            .await
            .unwrap()
            .error
            .unwrap()
            .contains("503")
    );
    server.save_status.store(0, Ordering::Release);
    server.calls.lock().unwrap().clear();
    service.sync().await.unwrap();
    assert!(repo.pending_karakeep(1).await.unwrap().is_empty());
    assert_eq!(repo.sync_status(1).await.unwrap().error, None);
    assert_eq!(*server.calls.lock().unwrap(), ["save", "pull"]);
}

#[tokio::test]
async fn simultaneous_delivery_and_pull_failures_preserve_cursor_and_both_errors() {
    let (repo, service, server) = fixture().await;
    let cursor = repo.sync_cursor(1).await.unwrap();
    service.set_read_local(42, true).await.unwrap();
    server.read_status.store(400, Ordering::Release);
    server.pull_status.store(503, Ordering::Release);
    assert!(service.sync().await.is_err());
    assert_eq!(*server.calls.lock().unwrap(), ["read", "pull"]);
    assert_eq!(repo.sync_cursor(1).await.unwrap(), cursor);
    assert_eq!(repo.pending_mutations(1).await.unwrap().len(), 1);
    let error = repo.sync_status(1).await.unwrap().error.unwrap();
    assert!(error.contains("400") && error.contains("503"));
    // Delivery recovery must not conceal the unresolved refresh failure.
    server.read_status.store(0, Ordering::Release);
    assert!(service.sync().await.is_err());
    let error = repo.sync_status(1).await.unwrap().error.unwrap();
    assert!(!error.contains("400") && error.contains("503"));
}

#[tokio::test]
async fn local_delivery_storage_failure_still_aborts_incoming_sync() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.db");
    let (repo, service, server) =
        fixture_with_repository(Arc::new(SqliteRepository::open(&path).unwrap())).await;
    let cursor = repo.sync_cursor(1).await.unwrap();
    let entry = repo.cached_entry(1, 42).await.unwrap().unwrap();
    service.queue_karakeep(&entry).await.unwrap();
    // Simulate inability to persist an accepted delivery, rather than a service
    // outage. Continuing here would hide the local cache failure.
    rusqlite::Connection::open(&path).unwrap().execute_batch(
        "CREATE TRIGGER fail_delivery BEFORE UPDATE ON karakeep_deliveries BEGIN SELECT RAISE(FAIL, 'storage unavailable'); END;"
    ).unwrap();
    assert!(matches!(
        service.sync().await,
        Err(BrookletError::Database(_))
    ));
    assert_eq!(*server.calls.lock().unwrap(), ["save"]);
    assert_eq!(repo.sync_cursor(1).await.unwrap(), cursor);
    assert_eq!(repo.pending_karakeep(1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn failed_delivery_before_first_pull_does_not_prevent_bootstrap() {
    let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
    repo.save_account(&Account {
        id: 1,
        server_url: "https://miniflux.example".into(),
        username: "reader".into(),
        server_version: "2.3.2".into(),
    })
    .await
    .unwrap();
    repo.merge_changed_page(
        1,
        &[brooklet::model::Entry {
            id: 42,
            account_id: 1,
            feed_id: 7,
            feed_title: "Feed".into(),
            category_title: "News".into(),
            title: "Story".into(),
            url: "https://example.org/42".into(),
            author: None,
            published_at_ms: 0,
            html: "<p>Cached</p>".into(),
            content_revision: 0,
            read: false,
            starred: false,
            reading_minutes: 1,
            delivery_state: None,
            delivery_error: None,
        }],
        &[],
    )
    .await
    .unwrap();
    repo.queue_karakeep(&brooklet::model::KarakeepDelivery {
        id: 0,
        account_id: 1,
        entry_id: 42,
        canonical_url: "https://example.org/42".into(),
        title: "Story".into(),
        route: brooklet::model::KarakeepRoute::Miniflux,
        state: brooklet::model::DeliveryState::Queued,
        error: None,
    })
    .await
    .unwrap();
    let server = Arc::new(Server::default());
    server.save_status.store(503, Ordering::Release);
    let service = AccountSyncService::new(
        repo.clone(),
        Arc::new(Secrets),
        Arc::new(Factory(server.clone())),
    );
    assert_eq!(repo.sync_cursor(1).await.unwrap(), None);
    let result = service.sync().await.unwrap();
    assert_eq!(result.fetched, 2);
    assert_eq!(result.inbox.len(), 2);
    assert!(repo.sync_cursor(1).await.unwrap().is_some());
    assert!(
        repo.sync_status(1)
            .await
            .unwrap()
            .error
            .unwrap()
            .contains("503")
    );
}

#[tokio::test]
async fn karakeep_terminal_failure_is_visible_after_reopen_and_retry_uses_current_route() {
    use brooklet::model::{DeliveryState, KarakeepConfig, KarakeepRoute};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.db");
    let (repo, service, server) =
        fixture_with_repository(Arc::new(SqliteRepository::open(&path).unwrap())).await;
    service
        .queue_karakeep(&repo.cached_entry(1, 42).await.unwrap().unwrap())
        .await
        .unwrap();
    server.save_status.store(403, Ordering::Release);
    service.sync().await.unwrap();
    let failed = service.unfinished_karakeep().await.unwrap().remove(0);
    assert_eq!(failed.state, DeliveryState::NeedsAttention);
    assert!(failed.error.as_deref().unwrap().contains("token"));
    assert!(repo.pending_karakeep(1).await.unwrap().is_empty());
    let reopened = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        reopened.unfinished_karakeep(1).await.unwrap(),
        vec![failed.clone()]
    );
    // A stored direct failure can deliberately switch to the current integration route.
    repo.recover_karakeep(1, failed.id, Some(KarakeepRoute::Direct))
        .await
        .unwrap();
    repo.finish_karakeep(failed.id, Some("direct failure"), 0)
        .await
        .unwrap();
    service
        .save_karakeep_config(
            &KarakeepConfig {
                route: KarakeepRoute::Miniflux,
                direct_endpoint: None,
            },
            None,
        )
        .await
        .unwrap();
    service.recover_karakeep(failed.id, true).await.unwrap();
    let queued = repo.pending_karakeep(1).await.unwrap().remove(0);
    assert_eq!(queued.route, KarakeepRoute::Miniflux);
    assert_eq!(queued.error, None);
    server.save_status.store(0, Ordering::Release);
    service.sync().await.unwrap();
    assert!(service.unfinished_karakeep().await.unwrap().is_empty());
    // Stale controls cannot replay a receipt that has succeeded.
    service.recover_karakeep(failed.id, true).await.unwrap();
    service.recover_karakeep(failed.id, false).await.unwrap();
    assert!(repo.pending_karakeep(1).await.unwrap().is_empty());
    assert_eq!(
        repo.cached_entry(1, 42)
            .await
            .unwrap()
            .unwrap()
            .delivery_state,
        Some(DeliveryState::Saved)
    );
}

#[tokio::test]
async fn karakeep_dismiss_is_account_scoped_and_preserves_article_and_pending_edits() {
    let (repo, service, server) = fixture().await;
    service
        .queue_karakeep(&repo.cached_entry(1, 42).await.unwrap().unwrap())
        .await
        .unwrap();
    service.set_read_local(42, true).await.unwrap();
    server.save_status.store(503, Ordering::Release);
    service.sync().await.unwrap();
    let delivery = service.unfinished_karakeep().await.unwrap().remove(0);
    assert!(delivery.error.is_some());
    repo.recover_karakeep(99, delivery.id, None).await.unwrap();
    assert_eq!(service.unfinished_karakeep().await.unwrap().len(), 1);
    let before = repo.cached_entry(1, 42).await.unwrap().unwrap();
    // Create a fresh unsent intention before dismissing only the receipt.
    service.set_starred_local(42, true).await.unwrap();
    service.recover_karakeep(delivery.id, false).await.unwrap();
    assert!(service.unfinished_karakeep().await.unwrap().is_empty());
    let article = repo.cached_entry(1, 42).await.unwrap().unwrap();
    assert_eq!(article.read, before.read);
    assert!(article.starred);
    assert_eq!(article.delivery_state, None);
    assert_eq!(repo.pending_mutations(1).await.unwrap().len(), 1);
    server.calls.lock().unwrap().clear();
    service.sync().await.unwrap();
    assert!(
        !server
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == "save")
    );
}

#[derive(Default)]
struct KarakeepSecrets {
    key: Mutex<Option<String>>,
    fail: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl SecretStore for KarakeepSecrets {
    async fn load_miniflux_token(&self, _: i64) -> Result<Option<String>, BrookletError> {
        Ok(Some("miniflux-token".into()))
    }
    async fn store_miniflux_token(&self, _: i64, _: &str) -> Result<(), BrookletError> {
        unreachable!()
    }
    async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
        unreachable!()
    }
    async fn load_karakeep_key(&self, _: i64) -> Result<Option<String>, BrookletError> {
        Ok(self.key.lock().unwrap().clone())
    }
    async fn store_karakeep_key(&self, _: i64, key: &str) -> Result<(), BrookletError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(BrookletError::SecretStore("unavailable".into()));
        }
        *self.key.lock().unwrap() = Some(key.into());
        Ok(())
    }
    async fn delete_karakeep_key(&self, _: i64) -> Result<(), BrookletError> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
}
#[derive(Default)]
struct KarakeepServer {
    status: AtomicU16,
    calls: Mutex<Vec<(String, String)>>,
}
struct KarakeepClient(Arc<KarakeepServer>);
#[async_trait]
impl brooklet::services::traits::KarakeepApi for KarakeepClient {
    async fn validate(&self) -> Result<(), BrookletError> {
        let status = self.0.status.load(Ordering::Acquire);
        if status == 0 {
            Ok(())
        } else {
            Err(BrookletError::Http {
                status,
                kind: classify_http_status(status),
            })
        }
    }
    async fn save(&self, _: &str, _: &str) -> Result<(), BrookletError> {
        unreachable!("settings validation must never create a bookmark")
    }
}
struct KarakeepFactory(Arc<KarakeepServer>);
impl brooklet::sync::KarakeepApiFactory for KarakeepFactory {
    fn create(
        &self,
        endpoint: &str,
        key: String,
    ) -> Result<Box<dyn brooklet::services::traits::KarakeepApi>, BrookletError> {
        self.0.calls.lock().unwrap().push((endpoint.into(), key));
        Ok(Box::new(KarakeepClient(self.0.clone())))
    }
}

#[tokio::test]
async fn direct_settings_require_valid_endpoint_key_and_server_before_persistence() {
    use brooklet::model::{KarakeepConfig, KarakeepRoute};
    let (repo, _, server) = fixture().await;
    let secrets = Arc::new(KarakeepSecrets::default());
    let direct_server = Arc::new(KarakeepServer::default());
    let service = AccountSyncService::new(repo.clone(), secrets.clone(), Arc::new(Factory(server)))
        .with_karakeep_factory(Arc::new(KarakeepFactory(direct_server.clone())));
    let mut config = KarakeepConfig {
        route: KarakeepRoute::Direct,
        direct_endpoint: Some("http://insecure.example/api/v1/bookmarks".into()),
    };
    assert!(
        service
            .save_karakeep_config(&config, Some("new-key".into()))
            .await
            .is_err()
    );
    config.direct_endpoint = Some("https://karakeep.example/api/v1/bookmarks".into());
    assert!(service.save_karakeep_config(&config, None).await.is_err());
    assert!(direct_server.calls.lock().unwrap().is_empty());
    direct_server.status.store(401, Ordering::Release);
    assert!(
        service
            .save_karakeep_config(&config, Some("bad-key".into()))
            .await
            .is_err()
    );
    assert_eq!(repo.karakeep_config(1).await.unwrap(), None);
    assert_eq!(*secrets.key.lock().unwrap(), None);
    direct_server.status.store(0, Ordering::Release);
    service
        .save_karakeep_config(&config, Some("good-key".into()))
        .await
        .unwrap();
    assert_eq!(repo.karakeep_config(1).await.unwrap(), Some(config.clone()));
    config.direct_endpoint = Some("https://karakeep.example/new/api/v1/bookmarks".into());
    direct_server.status.store(403, Ordering::Release);
    assert!(
        service
            .save_karakeep_config(&config, Some("replacement".into()))
            .await
            .is_err()
    );
    assert_eq!(*secrets.key.lock().unwrap(), Some("good-key".into()));
    assert_ne!(repo.karakeep_config(1).await.unwrap(), Some(config.clone()));
    direct_server.status.store(0, Ordering::Release);
    secrets.fail.store(true, Ordering::Release);
    assert!(
        service
            .save_karakeep_config(&config, Some("replacement".into()))
            .await
            .is_err()
    );
    assert_ne!(repo.karakeep_config(1).await.unwrap(), Some(config.clone()));
    // Retaining a readable saved key needs no keyring write.
    service.save_karakeep_config(&config, None).await.unwrap();
    assert_eq!(
        direct_server.calls.lock().unwrap().last().unwrap().1,
        "good-key"
    );
    assert_eq!(*secrets.key.lock().unwrap(), Some("good-key".into()));
}

#[tokio::test]
async fn failed_settings_database_write_restores_only_karakeep_secret() {
    use brooklet::model::{KarakeepConfig, KarakeepRoute};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.db");
    let (repo, _, server) =
        fixture_with_repository(Arc::new(SqliteRepository::open(&path).unwrap())).await;
    let secrets = Arc::new(KarakeepSecrets::default());
    let direct = Arc::new(KarakeepServer::default());
    let service = AccountSyncService::new(repo.clone(), secrets.clone(), Arc::new(Factory(server)))
        .with_karakeep_factory(Arc::new(KarakeepFactory(direct)));
    let config = KarakeepConfig {
        route: KarakeepRoute::Direct,
        direct_endpoint: Some("https://karakeep.example/api/v1/bookmarks".into()),
    };
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER reject_config BEFORE INSERT ON karakeep_config BEGIN SELECT RAISE(FAIL,'disk failure'); END;").unwrap();
    for old in [None, Some("old-key".to_string())] {
        *secrets.key.lock().unwrap() = old.clone();
        assert!(matches!(
            service
                .save_karakeep_config(&config, Some("new-key".into()))
                .await,
            Err(BrookletError::Database(_))
        ));
        assert_eq!(*secrets.key.lock().unwrap(), old);
        assert_eq!(repo.karakeep_config(1).await.unwrap(), None);
        assert!(repo.cached_entry(1, 42).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn direct_delivery_missing_key_is_actionable_without_breaking_integration_route() {
    use brooklet::model::{DeliveryState, KarakeepConfig, KarakeepRoute};
    let (repo, service, server) = fixture().await;
    // Legacy direct settings may lack their secret after a keyring loss.
    repo.save_karakeep_config(
        1,
        &KarakeepConfig {
            route: KarakeepRoute::Direct,
            direct_endpoint: Some("https://karakeep.example/api/v1/bookmarks".into()),
        },
    )
    .await
    .unwrap();
    service
        .queue_karakeep(&repo.cached_entry(1, 42).await.unwrap().unwrap())
        .await
        .unwrap();
    service.sync().await.unwrap();
    let failed = service.unfinished_karakeep().await.unwrap().remove(0);
    assert_eq!(failed.state, DeliveryState::NeedsAttention);
    assert!(failed.error.unwrap().contains("Karakeep API key"));
    service
        .save_karakeep_config(
            &KarakeepConfig {
                route: KarakeepRoute::Miniflux,
                direct_endpoint: Some("https://karakeep.example/api/v1/bookmarks".into()),
            },
            None,
        )
        .await
        .unwrap();
    service.recover_karakeep(failed.id, true).await.unwrap();
    service.sync().await.unwrap();
    assert!(service.unfinished_karakeep().await.unwrap().is_empty());
    assert!(
        server
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == "save")
    );
}
