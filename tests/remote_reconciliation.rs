use async_trait::async_trait;
use brooklet::{
    api::miniflux::{
        CategoryDto, EntriesDto, EntryDto, EntryIdsDto, EntryQuery, FeedDto, ServerIdentity,
    },
    error::BrookletError,
    model::{Account, ReaderPosition, StoragePolicy, classify_http_status},
    services::traits::{MinifluxApi, Repository, SecretStore},
    storage::sqlite::SqliteRepository,
    sync::{AccountSyncService, MinifluxApiFactory, SyncService},
};
use std::collections::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU16, Ordering},
};

const BASE: i64 = 1_790_000_000;
fn story(id: i64, changed: i64) -> EntryDto {
    EntryDto {
        id,
        feed_id: 7,
        title: format!("Story {id}"),
        url: format!("https://example.org/{id}"),
        author: None,
        published_at: jiff::Timestamp::from_second(BASE).unwrap().to_string(),
        changed_at: jiff::Timestamp::from_second(changed).unwrap().to_string(),
        content: "<p>Body</p>".into(),
        status: "unread".into(),
        starred: false,
        reading_time: 1,
        feed: None,
    }
}
#[derive(Default)]
struct Server {
    entries: Mutex<Vec<EntryDto>>,
    queries: Mutex<Vec<EntryQuery>>,
    probes: Mutex<Vec<i64>>,
    omitted: Mutex<HashSet<i64>>,
    race: AtomicBool,
    bad_page: AtomicU16,
    inventory_status: AtomicU16,
    probe_status: AtomicU16,
    upload_status: AtomicU16,
    inventory_hang: AtomicBool,
    inventory_queries: Mutex<Vec<(usize, usize)>>,
    repeat_inventory: AtomicBool,
}
fn status(value: &AtomicU16) -> Result<(), BrookletError> {
    let code = value.load(Ordering::Acquire);
    if code == 0 {
        Ok(())
    } else {
        Err(BrookletError::Http {
            status: code,
            kind: classify_http_status(code),
        })
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
    async fn entries(&self, query: &EntryQuery) -> Result<EntriesDto, BrookletError> {
        self.0.queries.lock().unwrap().push(query.clone());
        let mut entries = self.0.entries.lock().unwrap().clone();
        entries.retain(|entry| {
            let time = entry
                .changed_at
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_second();
            query.changed_after.is_none_or(|value| time > value)
                && query.changed_before.is_none_or(|value| time < value)
                && query.after_entry_id.is_none_or(|value| entry.id > value)
        });
        if query.order == "id" {
            entries.sort_by_key(|entry| entry.id);
        } else {
            entries.sort_by_key(|entry| (entry.changed_at.clone(), entry.id));
        }
        if query.direction == "desc" {
            entries.reverse();
        }
        let total = entries.len();
        let mut entries = entries
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect::<Vec<_>>();
        if query.limit > 1 {
            match self.0.bad_page.load(Ordering::Acquire) {
                1 if query.after_entry_id.is_some() => {
                    entries = (1..=100).map(|id| story(id, BASE)).collect()
                }
                2 if !entries.is_empty() => entries[0].changed_at = "bad timestamp".into(),
                3 if !entries.is_empty() => entries[0].status = "unexpected".into(),
                _ => {}
            }
            if self.0.race.swap(false, Ordering::AcqRel) {
                let mut remote = self.0.entries.lock().unwrap();
                remote.retain(|entry| entry.id != 25 && entry.id != 150);
                let entry = remote.iter_mut().find(|entry| entry.id == 50).unwrap();
                entry.changed_at = jiff::Timestamp::from_second(BASE + 120)
                    .unwrap()
                    .to_string();
                entry.title = "Late edit behind the ID cursor".into();
                remote.push(story(202, BASE + 10));
            }
        }
        Ok(EntriesDto { total, entries })
    }
    async fn entry_ids(&self, limit: usize, offset: usize) -> Result<EntryIdsDto, BrookletError> {
        self.0
            .inventory_queries
            .lock()
            .unwrap()
            .push((limit, offset));
        status(&self.0.inventory_status)?;
        if self.0.inventory_hang.load(Ordering::Acquire) {
            std::future::pending::<()>().await;
        }
        let omitted = self.0.omitted.lock().unwrap();
        let mut ids = self
            .0
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| !omitted.contains(&entry.id))
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        Ok(EntryIdsDto {
            total: ids.len(),
            entry_ids: ids
                .into_iter()
                .skip(if self.0.repeat_inventory.load(Ordering::Acquire) {
                    0
                } else {
                    offset
                })
                .take(limit)
                .collect(),
        })
    }
    async fn entry(&self, id: i64) -> Result<EntryDto, BrookletError> {
        self.0.probes.lock().unwrap().push(id);
        status(&self.0.probe_status)?;
        self.0
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or(BrookletError::Http {
                status: 404,
                kind: classify_http_status(404),
            })
    }
    async fn set_read(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
        status(&self.0.upload_status)
    }
    async fn set_starred(&self, _: &[i64], _: bool) -> Result<(), BrookletError> {
        status(&self.0.upload_status)
    }
    async fn save_to_integration(&self, _: i64) -> Result<(), BrookletError> {
        status(&self.0.upload_status)
    }
    async fn refresh_feeds(&self) -> Result<(), BrookletError> {
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
        Ok(Some("fixture".into()))
    }
    async fn delete_account_secrets(&self, _: i64) -> Result<(), BrookletError> {
        Ok(())
    }
}
async fn fixture(count: i64) -> (Arc<SqliteRepository>, AccountSyncService, Arc<Server>) {
    let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
    repo.save_account(&Account {
        id: 1,
        server_url: "https://miniflux.example".into(),
        username: "reader".into(),
        server_version: "2.3.2".into(),
    })
    .await
    .unwrap();
    let server = Arc::new(Server::default());
    *server.entries.lock().unwrap() = (1..=count).map(|id| story(id, BASE)).collect();
    let service = service(repo.clone(), server.clone());
    service.sync().await.unwrap();
    server.queries.lock().unwrap().clear();
    (repo, service, server)
}
fn service(repo: Arc<SqliteRepository>, server: Arc<Server>) -> AccountSyncService {
    AccountSyncService::new(repo, Arc::new(Secrets), Arc::new(Factory(server)))
}

#[tokio::test]
async fn concurrent_changes_do_not_skip_rows_or_advance_past_late_edits() {
    let (repo, service, server) = fixture(201).await;
    server.race.store(true, Ordering::Release);
    let result = service.sync().await.unwrap();
    assert_eq!(result.fetched, 200);
    let ids = repo.cached_entry_ids(1).await.unwrap();
    assert!(!ids.contains(&25) && !ids.contains(&150) && ids.contains(&201));
    assert!(!ids.contains(&202));
    assert_eq!(
        repo.cached_entry(1, 50).await.unwrap().unwrap().title,
        "Story 50"
    );
    assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE));
    let queries = server.queries.lock().unwrap().clone();
    assert_eq!(queries[0].limit, 1);
    assert_eq!(
        queries[1..]
            .iter()
            .map(|q| q.after_entry_id)
            .collect::<Vec<_>>(),
        [None, Some(100), Some(201)]
    );
    assert!(queries[1..].iter().all(|q| q.offset == 0
        && q.order == "id"
        && q.direction == "asc"
        && q.changed_before == Some(BASE + 1)));
    service.sync().await.unwrap();
    assert_eq!(
        repo.cached_entry(1, 50).await.unwrap().unwrap().title,
        "Late edit behind the ID cursor"
    );
    assert!(repo.cached_entry(1, 202).await.unwrap().is_some());
    assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE + 120));
}

#[tokio::test]
async fn hard_deletion_is_confirmed_and_cleans_reader_positions() {
    let (repo, service, server) = fixture(2).await;
    service
        .save_reader_position(&ReaderPosition {
            entry_id: 1,
            first_visible_block: 3,
            offset_px: 7,
        })
        .await
        .unwrap();
    server.entries.lock().unwrap().retain(|entry| entry.id != 1);
    service.sync().await.unwrap();
    assert_eq!(*server.probes.lock().unwrap(), [1]);
    assert!(repo.cached_entry(1, 1).await.unwrap().is_none());
    assert!(service.reader_position(1).await.unwrap().is_none());
}

#[tokio::test]
async fn inventory_omission_is_not_a_deletion_and_probe_failure_keeps_cache_cursor() {
    let (repo, service, server) = fixture(2).await;
    server.omitted.lock().unwrap().insert(1);
    server.probe_status.store(503, Ordering::Release);
    assert!(service.sync().await.is_err());
    assert!(repo.cached_entry(1, 1).await.unwrap().is_some());
    assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE));
    server.probe_status.store(0, Ordering::Release);
    server.entries.lock().unwrap()[0].title = "Still present".into();
    service.sync().await.unwrap();
    assert_eq!(
        repo.cached_entry(1, 1).await.unwrap().unwrap().title,
        "Still present"
    );
}

#[tokio::test]
async fn deleted_pending_work_is_retained_then_pruned_even_after_restart_and_overlap() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deleted.sqlite");
    let (memory, original, server) = fixture(2).await;
    let repo = Arc::new(SqliteRepository::open(&path).unwrap());
    repo.save_account(&memory.account().await.unwrap().unwrap())
        .await
        .unwrap();
    drop(original);
    let service = service(repo.clone(), server.clone());
    service.sync().await.unwrap();
    service.set_starred_local(1, true).await.unwrap();
    let article = repo.cached_entry(1, 2).await.unwrap().unwrap();
    service.queue_karakeep(&article).await.unwrap();
    server.upload_status.store(503, Ordering::Release);
    {
        let mut entries = server.entries.lock().unwrap();
        entries.retain(|entry| entry.id == 1);
        entries[0].status = "removed".into();
        entries[0].changed_at = jiff::Timestamp::from_second(BASE + 1).unwrap().to_string();
    }
    service.sync().await.unwrap();
    assert_eq!(repo.cached_entry_ids(1).await.unwrap(), [1, 2]);
    assert_eq!(repo.pending_mutations(1).await.unwrap().len(), 1);
    assert_eq!(repo.pending_karakeep(1).await.unwrap().len(), 1);
    drop(service);
    drop(repo);
    let repo = Arc::new(SqliteRepository::open(&path).unwrap());
    // Simulate acknowledgement/dismissal while the original deletion is outside
    // the next incoming window. Cleanup must use the persisted marker.
    let mutation = repo.pending_mutations(1).await.unwrap().remove(0);
    repo.acknowledge_mutation(&mutation).await.unwrap();
    let receipt = repo.pending_karakeep(1).await.unwrap().remove(0);
    repo.recover_karakeep(1, receipt.id, None).await.unwrap();
    repo.save_storage_policy(
        1,
        &StoragePolicy {
            retain_read_days: None,
            keep_at_most: 5000,
        },
    )
    .await
    .unwrap();
    repo.complete_sync(1, BASE + 1000, 1).await.unwrap();
    repo.apply_retention(1, 1).await.unwrap();
    assert!(repo.cached_entry_ids(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn malformed_or_repeated_pages_fail_without_advancing_cursor() {
    for mode in [1, 2, 3] {
        let (repo, service, server) = fixture(101).await;
        server.entries.lock().unwrap()[100].changed_at = jiff::Timestamp::from_second(BASE + 120)
            .unwrap()
            .to_string();
        server.bad_page.store(mode, Ordering::Release);
        assert!(matches!(
            service.sync().await,
            Err(BrookletError::SyncResponse(_))
        ));
        assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE));
        assert!(service.sync_status().await.unwrap().refresh_error.is_some());
    }
}

#[tokio::test(start_paused = true)]
async fn inventory_timeout_keeps_cache_and_allows_retry() {
    let (repo, service, server) = fixture(2).await;
    server.inventory_hang.store(true, Ordering::Release);
    assert!(matches!(
        service.sync().await,
        Err(BrookletError::SyncResponse(
            "remote reconciliation timed out"
        ))
    ));
    assert_eq!(repo.cached_entry_ids(1).await.unwrap(), [1, 2]);
    assert!(!service.sync_status().await.unwrap().running);
    server.inventory_hang.store(false, Ordering::Release);
    server.inventory_status.store(401, Ordering::Release);
    assert!(service.sync().await.is_err());
    assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE));
    server.inventory_status.store(0, Ordering::Release);
    service.sync().await.unwrap();
    assert_eq!(service.sync_status().await.unwrap().refresh_error, None);
}

#[tokio::test]
async fn restored_remote_entry_clears_deletion_marker_and_completed_receipts_still_expire() {
    let (repo, service, server) = fixture(2).await;
    service.set_starred_local(1, true).await.unwrap();
    server.upload_status.store(503, Ordering::Release);
    server.entries.lock().unwrap().retain(|entry| entry.id != 1);
    service.sync().await.unwrap();
    assert!(repo.cached_entry(1, 1).await.unwrap().is_some());
    server.entries.lock().unwrap().push(story(1, BASE + 120));
    service.sync().await.unwrap();
    let mutation = repo.pending_mutations(1).await.unwrap().remove(0);
    repo.acknowledge_mutation(&mutation).await.unwrap();
    let article = repo.cached_entry(1, 2).await.unwrap().unwrap();
    service.queue_karakeep(&article).await.unwrap();
    let receipt = repo.pending_karakeep(1).await.unwrap().remove(0);
    repo.finish_karakeep(receipt.id, None, 0).await.unwrap();
    repo.save_storage_policy(
        1,
        &StoragePolicy {
            retain_read_days: None,
            keep_at_most: 5000,
        },
    )
    .await
    .unwrap();
    repo.apply_retention(1, 31 * 86_400_000).await.unwrap();
    assert_eq!(repo.cached_entry_ids(1).await.unwrap(), [1, 2]);
    assert_eq!(
        repo.cached_entry(1, 2)
            .await
            .unwrap()
            .unwrap()
            .delivery_state,
        None
    );
}

#[tokio::test]
async fn inventory_pages_confirm_omissions_and_reject_non_progress() {
    let (repo, service, server) = fixture(10_005).await;
    server.inventory_queries.lock().unwrap().clear();
    server.omitted.lock().unwrap().insert(1);
    service.sync().await.unwrap();
    assert_eq!(
        *server.inventory_queries.lock().unwrap(),
        [(10_000, 0), (10_000, 10_000)]
    );
    assert_eq!(*server.probes.lock().unwrap(), [1]);
    assert!(repo.cached_entry(1, 1).await.unwrap().is_some());
    server.repeat_inventory.store(true, Ordering::Release);
    assert!(matches!(
        service.sync().await,
        Err(BrookletError::SyncResponse(
            "inventory pagination did not advance"
        ))
    ));
    assert_eq!(repo.sync_cursor(1).await.unwrap(), Some(BASE));
    assert_eq!(repo.cached_entry_ids(1).await.unwrap().len(), 10_005);
}

#[tokio::test]
async fn deletion_marker_upgrade_preserves_existing_cache_and_pending_work() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.sqlite");
    let (memory, _, server) = fixture(1).await;
    let repo = Arc::new(SqliteRepository::open(&path).unwrap());
    repo.save_account(&memory.account().await.unwrap().unwrap())
        .await
        .unwrap();
    let service = service(repo.clone(), server);
    service.sync().await.unwrap();
    service.set_starred_local(1, true).await.unwrap();
    let before = repo.cached_entry(1, 1).await.unwrap().unwrap();
    let pending = repo.pending_mutations(1).await.unwrap();
    drop(service);
    drop(repo);
    // Recreate the previous released schema with its existing rows and queue.
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    connection
        .execute_batch("ALTER TABLE entries DROP COLUMN remote_removed")
        .unwrap();
    connection
        .pragma_update(None, "user_version", version - 1)
        .unwrap();
    drop(connection);
    let upgraded = SqliteRepository::open(&path).unwrap();
    assert_eq!(upgraded.cached_entry(1, 1).await.unwrap().unwrap(), before);
    assert_eq!(upgraded.pending_mutations(1).await.unwrap(), pending);
    let connection = rusqlite::Connection::open(path).unwrap();
    let marker: i64 = connection
        .query_row(
            "SELECT remote_removed FROM entries WHERE account_id=1 AND id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker, 0);
}
