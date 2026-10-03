use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use async_trait::async_trait;
use brooklet::{
    api::miniflux::{ServerIdentity, UserDto, VersionDto},
    error::BrookletError,
    model::{Account, Entry, KarakeepConfig, KarakeepRoute, ReaderPosition, StoragePolicy},
    services::traits::{Repository, SecretStore},
    setup::{AccountSetupService, IdentityValidator, SetupRequest, SetupService},
    storage::sqlite::SqliteRepository,
};

#[derive(Default)]
struct Secrets {
    token: Mutex<Option<String>>,
    key: Mutex<Option<String>>,
    fail_store: AtomicBool,
    loads: AtomicUsize,
    deletes: AtomicUsize,
}

#[async_trait]
impl SecretStore for Secrets {
    async fn load_miniflux_token(&self, _: i64) -> Result<Option<String>, BrookletError> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        Ok(self.token.lock().unwrap().clone())
    }
    async fn store_miniflux_token(&self, _: i64, token: &str) -> Result<(), BrookletError> {
        if self.fail_store.load(Ordering::Relaxed) {
            return Err(BrookletError::SecretStore("keyring unavailable".into()));
        }
        *self.token.lock().unwrap() = Some(token.into());
        Ok(())
    }
    async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
        self.deletes.fetch_add(1, Ordering::Relaxed);
        *self.token.lock().unwrap() = None;
        *self.key.lock().unwrap() = None;
        Ok(())
    }
    async fn load_karakeep_key(&self, _: i64) -> Result<Option<String>, BrookletError> {
        Ok(self.key.lock().unwrap().clone())
    }
}

struct Validator {
    username: Mutex<String>,
    fail: AtomicBool,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl IdentityValidator for Validator {
    async fn validate(
        &self,
        server_url: &str,
        token: String,
    ) -> Result<ServerIdentity, BrookletError> {
        self.calls.lock().unwrap().push(server_url.into());
        assert_eq!(token, "replacement");
        if self.fail.load(Ordering::Relaxed) {
            return Err(BrookletError::Http {
                status: 401,
                kind: brooklet::model::classify_http_status(401),
            });
        }
        Ok(ServerIdentity {
            user: UserDto {
                id: 7,
                username: self.username.lock().unwrap().clone(),
                is_admin: false,
            },
            version: VersionDto {
                version: "2.3.3".into(),
                commit: None,
            },
        })
    }
}

async fn populated_repo(path: &std::path::Path) -> Arc<SqliteRepository> {
    let repo = Arc::new(SqliteRepository::open(path).unwrap());
    repo.save_account(&Account {
        id: 1,
        server_url: "https://miniflux.example/root".into(),
        username: "reader".into(),
        server_version: "2.3.2".into(),
    })
    .await
    .unwrap();
    repo.merge_changed_page(
        1,
        &[Entry {
            id: 42,
            account_id: 1,
            feed_id: 7,
            feed_title: "Feed".into(),
            category_title: "News".into(),
            title: "Story".into(),
            url: "https://example.org/42".into(),
            author: None,
            published_at_ms: 0,
            html: "<p>Offline body</p>".into(),
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
    repo.save_reader_position(
        1,
        &ReaderPosition {
            entry_id: 42,
            first_visible_block: 2,
            offset_px: 15,
        },
    )
    .await
    .unwrap();
    repo.set_read_local(1, 42, true).await.unwrap();
    repo.set_starred_local(1, 42, true).await.unwrap();
    let entry = repo.cached_entry(1, 42).await.unwrap().unwrap();
    repo.queue_karakeep(&brooklet::model::KarakeepDelivery {
        id: 0,
        account_id: 1,
        entry_id: 42,
        canonical_url: entry.url,
        title: entry.title,
        route: KarakeepRoute::Miniflux,
        state: brooklet::model::DeliveryState::Queued,
        error: None,
    })
    .await
    .unwrap();
    repo.save_karakeep_config(
        1,
        &KarakeepConfig {
            route: KarakeepRoute::Miniflux,
            direct_endpoint: None,
        },
    )
    .await
    .unwrap();
    repo.save_storage_policy(
        1,
        &StoragePolicy {
            retain_read_days: Some(90),
            keep_at_most: 123,
        },
    )
    .await
    .unwrap();
    repo.complete_sync(1, 100, 200).await.unwrap();
    repo.record_delivery_error(1, Some("old token rejected"))
        .await
        .unwrap();
    repo
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    account: Option<Account>,
    entry: Option<Entry>,
    position: Option<ReaderPosition>,
    pending: Vec<brooklet::model::PendingMutation>,
    karakeep: Vec<brooklet::model::KarakeepDelivery>,
    config: Option<KarakeepConfig>,
    policy: StoragePolicy,
    status: brooklet::model::SyncStatus,
    cursor: Option<i64>,
}

async fn snapshot(repo: &SqliteRepository) -> Snapshot {
    Snapshot {
        account: repo.account().await.unwrap(),
        entry: repo.cached_entry(1, 42).await.unwrap(),
        position: repo.reader_position(42).await.unwrap(),
        pending: repo.pending_mutations(1).await.unwrap(),
        karakeep: repo.pending_karakeep(1).await.unwrap(),
        config: repo.karakeep_config(1).await.unwrap(),
        policy: repo.storage_policy(1).await.unwrap(),
        status: repo.sync_status(1).await.unwrap(),
        cursor: repo.sync_cursor(1).await.unwrap(),
    }
}

fn services(repo: Arc<SqliteRepository>) -> (AccountSetupService, Arc<Validator>, Arc<Secrets>) {
    let validator = Arc::new(Validator {
        username: Mutex::new("reader".into()),
        fail: AtomicBool::new(false),
        calls: Mutex::new(vec![]),
    });
    let secrets = Arc::new(Secrets::default());
    *secrets.token.lock().unwrap() = Some("old-token".into());
    *secrets.key.lock().unwrap() = Some("karakeep-key".into());
    (
        AccountSetupService::new(validator.clone(), repo, secrets.clone()),
        validator,
        secrets,
    )
}

#[tokio::test]
async fn reconnect_replaces_only_token_and_preserves_all_local_data_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.db");
    let repo = populated_repo(&path).await;
    let before = snapshot(&repo).await;
    let (service, validator, secrets) = services(repo.clone());
    let account = service.reconnect(" replacement ".into()).await.unwrap();
    assert_eq!(account, repo.account().await.unwrap().unwrap());
    assert_eq!(snapshot(&repo).await, before);
    assert_eq!(*secrets.token.lock().unwrap(), Some("replacement".into()));
    assert_eq!(*secrets.key.lock().unwrap(), Some("karakeep-key".into()));
    assert_eq!(secrets.loads.load(Ordering::Relaxed), 0);
    assert_eq!(secrets.deletes.load(Ordering::Relaxed), 0);
    assert_eq!(
        *validator.calls.lock().unwrap(),
        ["https://miniflux.example/root"]
    );
    let reopened = SqliteRepository::open(path).unwrap();
    assert_eq!(snapshot(&reopened).await, before);
}

#[tokio::test]
async fn missing_old_token_can_be_repaired_without_loading_it() {
    let directory = tempfile::tempdir().unwrap();
    let repo = populated_repo(&directory.path().join("cache.db")).await;
    let before = snapshot(&repo).await;
    let (service, _, secrets) = services(repo.clone());
    *secrets.token.lock().unwrap() = None;
    service.reconnect("replacement".into()).await.unwrap();
    assert_eq!(*secrets.token.lock().unwrap(), Some("replacement".into()));
    assert_eq!(secrets.loads.load(Ordering::Relaxed), 0);
    assert_eq!(snapshot(&repo).await, before);
}

#[tokio::test]
async fn rejected_or_different_user_tokens_leave_old_credentials_and_data_intact() {
    for different_user in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let repo = populated_repo(&directory.path().join("cache.db")).await;
        let before = snapshot(&repo).await;
        let (service, validator, secrets) = services(repo.clone());
        if different_user {
            *validator.username.lock().unwrap() = "someone-else".into();
        } else {
            validator.fail.store(true, Ordering::Relaxed);
        }
        let result = service.reconnect("replacement".into()).await;
        if different_user {
            assert!(matches!(result, Err(BrookletError::AccountMismatch)));
        } else {
            assert!(matches!(
                result,
                Err(BrookletError::Http { status: 401, .. })
            ));
        }
        assert_eq!(*secrets.token.lock().unwrap(), Some("old-token".into()));
        assert_eq!(*secrets.key.lock().unwrap(), Some("karakeep-key".into()));
        assert_eq!(secrets.deletes.load(Ordering::Relaxed), 0);
        assert_eq!(snapshot(&repo).await, before);
    }
}

#[tokio::test]
async fn keyring_failure_preserves_account_and_pending_work_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let repo = populated_repo(&directory.path().join("cache.db")).await;
    let before = snapshot(&repo).await;
    let (service, _, secrets) = services(repo.clone());
    secrets.fail_store.store(true, Ordering::Relaxed);
    assert!(matches!(
        service.reconnect("replacement".into()).await,
        Err(BrookletError::SecretStore(_))
    ));
    assert_eq!(*secrets.token.lock().unwrap(), Some("old-token".into()));
    assert_eq!(snapshot(&repo).await, before);
    secrets.fail_store.store(false, Ordering::Relaxed);
    service.reconnect("replacement".into()).await.unwrap();
    assert_eq!(snapshot(&repo).await, before);
}

#[tokio::test]
async fn empty_token_and_setup_overwrite_are_rejected_before_validation() {
    let directory = tempfile::tempdir().unwrap();
    let repo = populated_repo(&directory.path().join("cache.db")).await;
    let before = snapshot(&repo).await;
    let (service, validator, secrets) = services(repo.clone());
    assert!(service.reconnect("  ".into()).await.is_err());
    assert!(matches!(
        service
            .configure(SetupRequest {
                server_url: "https://another.example".into(),
                token: "replacement".into()
            })
            .await,
        Err(BrookletError::AccountAlreadyConfigured)
    ));
    assert!(validator.calls.lock().unwrap().is_empty());
    assert_eq!(*secrets.token.lock().unwrap(), Some("old-token".into()));
    assert_eq!(snapshot(&repo).await, before);
}

#[tokio::test]
async fn reconnect_after_logout_does_not_recreate_account_or_secret() {
    let directory = tempfile::tempdir().unwrap();
    let repo = populated_repo(&directory.path().join("cache.db")).await;
    let (service, validator, secrets) = services(repo.clone());
    repo.delete_account(1).await.unwrap();
    secrets.delete_account_secrets(1).await.unwrap();
    assert!(service.reconnect("replacement".into()).await.is_err());
    assert!(validator.calls.lock().unwrap().is_empty());
    assert!(repo.account().await.unwrap().is_none());
    assert!(secrets.token.lock().unwrap().is_none());
}

#[tokio::test]
async fn sync_uses_replacement_token_and_resumes_persisted_outgoing_work() {
    use brooklet::{
        api::miniflux::{CategoryDto, EntriesDto, EntryDto, EntryQuery, FeedDto},
        services::traits::MinifluxApi,
        sync::{AccountSyncService, MinifluxApiFactory, SyncService},
    };
    struct Api;
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
            Ok(EntriesDto {
                total: 0,
                entries: vec![],
            })
        }
        async fn entry(&self, _: i64) -> Result<EntryDto, BrookletError> {
            unreachable!()
        }
        async fn set_read(&self, ids: &[i64], read: bool) -> Result<(), BrookletError> {
            assert_eq!(ids, [42]);
            assert!(read);
            Ok(())
        }
        async fn set_starred(&self, ids: &[i64], starred: bool) -> Result<(), BrookletError> {
            assert_eq!(ids, [42]);
            assert!(starred);
            Ok(())
        }
        async fn save_to_integration(&self, id: i64) -> Result<(), BrookletError> {
            assert_eq!(id, 42);
            Ok(())
        }
        async fn refresh_feeds(&self) -> Result<(), BrookletError> {
            unreachable!()
        }
        async fn subscribe(&self, _: &str, _: Option<i64>) -> Result<i64, BrookletError> {
            unreachable!()
        }
    }
    struct Factory;
    impl MinifluxApiFactory for Factory {
        fn create(
            &self,
            server: &str,
            token: String,
        ) -> Result<Box<dyn MinifluxApi>, BrookletError> {
            assert_eq!(server, "https://miniflux.example/root");
            if token != "replacement" {
                return Err(BrookletError::Http {
                    status: 401,
                    kind: brooklet::model::classify_http_status(401),
                });
            }
            Ok(Box::new(Api))
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let repo = populated_repo(&directory.path().join("cache.db")).await;
    let (repair, _, secrets) = services(repo.clone());
    let sync = AccountSyncService::new(repo.clone(), secrets, Arc::new(Factory));
    assert!(sync.sync().await.is_err());
    assert_eq!(repo.pending_mutations(1).await.unwrap().len(), 2);
    assert_eq!(repo.pending_karakeep(1).await.unwrap().len(), 1);
    repair.reconnect("replacement".into()).await.unwrap();
    sync.sync().await.unwrap();
    assert!(repo.pending_mutations(1).await.unwrap().is_empty());
    assert!(repo.pending_karakeep(1).await.unwrap().is_empty());
    assert_eq!(repo.sync_status(1).await.unwrap().error, None);
    assert_eq!(
        repo.reader_position(42).await.unwrap().unwrap().offset_px,
        15
    );
    assert_eq!(
        repo.cached_entry(1, 42).await.unwrap().unwrap().html,
        "<p>Offline body</p>"
    );
}
