use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;

use crate::{
    api::{
        karakeep::ReqwestKarakeepApi,
        miniflux::{EntryDto, EntryQuery, ReqwestMinifluxApi},
    },
    error::BrookletError,
    model::{
        Category, Entry, EntryId, Feed, KarakeepConfig, KarakeepDelivery, KarakeepRoute,
        MutationField, ReaderPosition, StoragePolicy, SyncStatus, incremental_start,
    },
    services::traits::{KarakeepApi, MinifluxApi, Repository, SecretStore},
};

const PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncResult {
    pub inbox: Vec<Entry>,
    pub fetched: usize,
}

pub trait MinifluxApiFactory: Send + Sync {
    fn create(
        &self,
        server_url: &str,
        token: String,
    ) -> Result<Box<dyn MinifluxApi>, BrookletError>;
}

pub struct ReqwestMinifluxApiFactory;

impl MinifluxApiFactory for ReqwestMinifluxApiFactory {
    fn create(
        &self,
        server_url: &str,
        token: String,
    ) -> Result<Box<dyn MinifluxApi>, BrookletError> {
        Ok(Box::new(ReqwestMinifluxApi::new(server_url, token)?))
    }
}

#[async_trait]
pub trait SyncService: Send + Sync {
    async fn cached_entry(&self, entry_id: EntryId) -> Result<Option<Entry>, BrookletError>;

    async fn disconnect(&self) -> Result<(), BrookletError>;
    async fn cached_inbox(&self) -> Result<Vec<Entry>, BrookletError>;
    async fn set_read_local(&self, entry_id: EntryId, read: bool) -> Result<(), BrookletError>;
    async fn set_read_many_local(
        &self,
        entry_ids: &[EntryId],
        read: bool,
    ) -> Result<(), BrookletError>;
    async fn set_starred_local(
        &self,
        entry_id: EntryId,
        starred: bool,
    ) -> Result<(), BrookletError>;
    async fn entries_for_view(&self, view: &str) -> Result<Vec<Entry>, BrookletError>;
    async fn search_entries(
        &self,
        query: &str,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
    ) -> Result<Vec<Entry>, BrookletError>;
    async fn categories_cached(&self) -> Result<Vec<Category>, BrookletError>;
    async fn feeds_cached(&self, category_id: Option<i64>) -> Result<Vec<Feed>, BrookletError>;
    async fn reader_position(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<ReaderPosition>, BrookletError>;
    async fn save_reader_position(&self, position: &ReaderPosition) -> Result<(), BrookletError>;
    async fn queue_karakeep(&self, entry: &Entry) -> Result<(), BrookletError>;
    async fn karakeep_config(&self) -> Result<Option<KarakeepConfig>, BrookletError>;
    async fn save_karakeep_config(
        &self,
        config: &KarakeepConfig,
        key: Option<String>,
    ) -> Result<(), BrookletError>;
    async fn storage_policy(&self) -> Result<StoragePolicy, BrookletError>;
    async fn save_storage_policy(&self, policy: &StoragePolicy) -> Result<(), BrookletError>;
    async fn sync_status(&self) -> Result<SyncStatus, BrookletError>;
    async fn refresh_feeds(&self) -> Result<SyncResult, BrookletError>;
    async fn subscribe(
        &self,
        feed_url: &str,
        category_id: Option<i64>,
    ) -> Result<(), BrookletError>;
    async fn sync(&self) -> Result<SyncResult, BrookletError>;
}

pub struct AccountSyncService {
    repository: Arc<dyn Repository>,
    secrets: Arc<dyn SecretStore>,
    api_factory: Arc<dyn MinifluxApiFactory>,
    sync_lock: tokio::sync::Mutex<()>,
    running: AtomicBool,
}

impl AccountSyncService {
    pub fn new(
        repository: Arc<dyn Repository>,
        secrets: Arc<dyn SecretStore>,
        api_factory: Arc<dyn MinifluxApiFactory>,
    ) -> Self {
        Self {
            repository,
            secrets,
            api_factory,
            sync_lock: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
        }
    }

    async fn configured_account(&self) -> Result<crate::model::Account, BrookletError> {
        self.repository
            .account()
            .await?
            .ok_or(BrookletError::InvalidSetup("a configured Miniflux account"))
    }

    async fn api(
        &self,
        account: &crate::model::Account,
    ) -> Result<Box<dyn MinifluxApi>, BrookletError> {
        let token = self
            .secrets
            .load_miniflux_token(account.id)
            .await?
            .ok_or_else(|| BrookletError::SecretStore("the Miniflux token is missing".into()))?;
        self.api_factory.create(&account.server_url, token)
    }

    async fn run_sync(&self, refresh_feeds: bool) -> Result<SyncResult, BrookletError> {
        let _guard = self.sync_lock.lock().await;
        let account = self.configured_account().await?;
        self.running.store(true, Ordering::Release);
        tracing::info!(account_id = account.id, refresh_feeds, "sync started");
        let result = self.run_sync_account(&account, refresh_feeds).await;
        if let Err(error) = &result {
            let message = error.to_string();
            tracing::warn!(account_id = account.id, failure_kind = ?error.failure_kind(), "sync failed; cached articles remain available");
            let _ = self
                .repository
                .record_sync_error(account.id, &message, now_ms())
                .await;
        }
        if let Ok(completed) = &result {
            tracing::info!(
                account_id = account.id,
                fetched = completed.fetched,
                inbox = completed.inbox.len(),
                "sync completed"
            );
        }
        self.running.store(false, Ordering::Release);
        result
    }

    async fn run_sync_account(
        &self,
        account: &crate::model::Account,
        refresh_feeds: bool,
    ) -> Result<SyncResult, BrookletError> {
        let api = self.api(account).await?;
        if refresh_feeds {
            api.refresh_feeds().await?;
        }
        let pending = self.repository.pending_mutations(account.id).await?;
        let mut accepted = Vec::new();
        for field in [MutationField::Read, MutationField::Starred] {
            for desired in [false, true] {
                let batch = pending
                    .iter()
                    .filter(|mutation| mutation.field == field && mutation.desired == desired)
                    .cloned()
                    .collect::<Vec<_>>();
                if batch.is_empty() {
                    continue;
                }
                let ids = batch
                    .iter()
                    .map(|mutation| mutation.entry_id)
                    .collect::<Vec<_>>();
                match field {
                    MutationField::Read => api.set_read(&ids, desired).await?,
                    MutationField::Starred => api.set_starred(&ids, desired).await?,
                }
                accepted.extend(batch);
            }
        }
        let karakeep_config = self.repository.karakeep_config(account.id).await?;
        let direct_api = if let Some(config) = karakeep_config
            .as_ref()
            .filter(|config| config.route == KarakeepRoute::Direct)
        {
            match (
                &config.direct_endpoint,
                self.secrets.load_karakeep_key(account.id).await?,
            ) {
                (Some(endpoint), Some(key)) => Some(ReqwestKarakeepApi::new(endpoint, key)?),
                _ => None,
            }
        } else {
            None
        };
        for delivery in self.repository.pending_karakeep(account.id).await? {
            let result = match delivery.route {
                KarakeepRoute::Miniflux => api.save_to_integration(delivery.entry_id).await,
                KarakeepRoute::Direct => match &direct_api {
                    Some(client) => client.save(&delivery.canonical_url, &delivery.title).await,
                    None => Err(BrookletError::InvalidSetup(
                        "a Karakeep endpoint and API key",
                    )),
                },
            };
            match result {
                Ok(()) => {
                    self.repository
                        .finish_karakeep(delivery.id, None, now_ms())
                        .await?
                }
                Err(error) => {
                    if error.failure_kind() == crate::model::FailureKind::Retryable {
                        self.repository
                            .defer_karakeep(delivery.id, &error.to_string())
                            .await?;
                        return Err(error);
                    }
                    self.repository
                        .finish_karakeep(delivery.id, Some(&error.to_string()), now_ms())
                        .await?;
                }
            }
        }
        let categories = api
            .categories()
            .await?
            .into_iter()
            .map(|dto| Category {
                id: dto.id,
                title: dto.title,
            })
            .collect::<Vec<_>>();
        let feeds = api
            .feeds()
            .await?
            .into_iter()
            .map(|dto| Feed {
                id: dto.id,
                category_id: dto.category.map_or(0, |category| category.id),
                title: dto.title,
                site_url: dto.site_url,
                feed_url: dto.feed_url,
            })
            .collect::<Vec<_>>();
        self.repository
            .merge_metadata(account.id, &categories, &feeds)
            .await?;
        let cursor = self.repository.sync_cursor(account.id).await?.unwrap_or(0);
        let overlap = incremental_start(Some(cursor), 60);
        let mut newest = cursor;
        let mut offset = 0;
        let mut fetched = 0;
        loop {
            let page = api
                .entries(&EntryQuery {
                    status: None,
                    changed_after: Some(overlap),
                    limit: PAGE_SIZE,
                    offset,
                    ..EntryQuery::default()
                })
                .await?;
            let page_len = page.entries.len();
            let mut entries = Vec::new();
            let mut removed = Vec::new();
            for dto in page.entries {
                if let Ok(timestamp) = dto.changed_at.parse::<jiff::Timestamp>() {
                    newest = newest.max(timestamp.as_second());
                }
                match dto.status.as_str() {
                    "read" | "unread" => entries.push(map_entry(account.id, dto)),
                    "removed" => removed.push(dto.id),
                    _ => {}
                }
            }
            self.repository
                .merge_changed_page(account.id, &entries, &removed)
                .await?;
            fetched += page_len;
            offset += page_len;
            if page_len == 0 || offset >= page.total {
                break;
            }
        }
        self.repository
            .complete_sync(account.id, newest, now_ms())
            .await?;
        for mutation in &accepted {
            self.repository.acknowledge_mutation(mutation).await?;
        }
        self.repository
            .apply_retention(account.id, now_ms())
            .await?;
        let inbox = self.repository.unread_entries(account.id).await?;
        Ok(SyncResult { inbox, fetched })
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[async_trait]
impl SyncService for AccountSyncService {
    async fn cached_entry(&self, entry_id: EntryId) -> Result<Option<Entry>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.cached_entry(account.id, entry_id).await
    }

    async fn disconnect(&self) -> Result<(), BrookletError> {
        let _guard = self.sync_lock.lock().await;
        if let Some(account) = self.repository.account().await? {
            self.secrets.delete_account_secrets(account.id).await?;
            self.repository.delete_account(account.id).await?;
        }
        Ok(())
    }

    async fn cached_inbox(&self) -> Result<Vec<Entry>, BrookletError> {
        let account = self
            .repository
            .account()
            .await?
            .ok_or(BrookletError::InvalidSetup("a configured Miniflux account"))?;
        self.repository.unread_entries(account.id).await
    }

    async fn set_read_local(&self, entry_id: EntryId, read: bool) -> Result<(), BrookletError> {
        let account = self
            .repository
            .account()
            .await?
            .ok_or(BrookletError::InvalidSetup("a configured Miniflux account"))?;
        self.repository
            .set_read_local(account.id, entry_id, read)
            .await
    }

    async fn set_read_many_local(
        &self,
        entry_ids: &[EntryId],
        read: bool,
    ) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .set_read_many_local(account.id, entry_ids, read)
            .await
    }

    async fn set_starred_local(
        &self,
        entry_id: EntryId,
        starred: bool,
    ) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .set_starred_local(account.id, entry_id, starred)
            .await
    }

    async fn entries_for_view(&self, view: &str) -> Result<Vec<Entry>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.entries_for_view(account.id, view).await
    }

    async fn search_entries(
        &self,
        query: &str,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
    ) -> Result<Vec<Entry>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .search_entries(account.id, query, feed_id, category_id, read)
            .await
    }

    async fn categories_cached(&self) -> Result<Vec<Category>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.categories_cached(account.id).await
    }

    async fn feeds_cached(&self, category_id: Option<i64>) -> Result<Vec<Feed>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.feeds_cached(account.id, category_id).await
    }

    async fn reader_position(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<ReaderPosition>, BrookletError> {
        self.repository.reader_position(entry_id).await
    }
    async fn save_reader_position(&self, position: &ReaderPosition) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .save_reader_position(account.id, position)
            .await
    }
    async fn queue_karakeep(&self, entry: &Entry) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        let route = self
            .repository
            .karakeep_config(account.id)
            .await?
            .map_or(KarakeepRoute::Miniflux, |config| config.route);
        let canonical_url = crate::services::url_policy::canonical_url(&entry.url)
            .map_err(|_| BrookletError::InvalidServiceUrl("article URL is invalid".into()))?;
        self.repository
            .queue_karakeep(&KarakeepDelivery {
                id: 0,
                account_id: account.id,
                entry_id: entry.id,
                canonical_url,
                title: entry.title.clone(),
                route,
                state: crate::model::DeliveryState::Queued,
                error: None,
            })
            .await
    }
    async fn karakeep_config(&self) -> Result<Option<KarakeepConfig>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.karakeep_config(account.id).await
    }
    async fn save_karakeep_config(
        &self,
        config: &KarakeepConfig,
        key: Option<String>,
    ) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        if config.route == KarakeepRoute::Direct {
            let endpoint = config
                .direct_endpoint
                .as_deref()
                .ok_or(BrookletError::InvalidSetup("a Karakeep API endpoint"))?;
            crate::services::url_policy::service_url(endpoint)?;
            if let Some(key) = key.as_deref() {
                self.secrets.store_karakeep_key(account.id, key).await?;
            }
        }
        self.repository
            .save_karakeep_config(account.id, config)
            .await
    }
    async fn storage_policy(&self) -> Result<StoragePolicy, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.storage_policy(account.id).await
    }
    async fn save_storage_policy(&self, policy: &StoragePolicy) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .save_storage_policy(account.id, policy)
            .await
    }
    async fn sync_status(&self) -> Result<SyncStatus, BrookletError> {
        let account = self.configured_account().await?;
        let mut status = self.repository.sync_status(account.id).await?;
        status.running = self.running.load(Ordering::Acquire);
        Ok(status)
    }
    async fn refresh_feeds(&self) -> Result<SyncResult, BrookletError> {
        self.run_sync(true).await
    }
    async fn subscribe(
        &self,
        feed_url: &str,
        category_id: Option<i64>,
    ) -> Result<(), BrookletError> {
        let account = self.configured_account().await?;
        let api = self.api(&account).await?;
        api.subscribe(feed_url, category_id).await?;
        Ok(())
    }

    async fn sync(&self) -> Result<SyncResult, BrookletError> {
        self.run_sync(false).await
    }
}

fn map_entry(account_id: i64, entry: EntryDto) -> Entry {
    let feed_title = entry
        .feed
        .as_ref()
        .map(|feed| feed.title.clone())
        .unwrap_or_else(|| "Unknown feed".into());
    let category_title = entry
        .feed
        .as_ref()
        .and_then(|feed| feed.category.as_ref())
        .map(|category| category.title.clone())
        .unwrap_or_default();
    let published_at_ms = entry
        .published_at
        .parse::<jiff::Timestamp>()
        .map(|timestamp| timestamp.as_millisecond())
        .unwrap_or_default();

    Entry {
        id: entry.id,
        account_id,
        feed_id: entry.feed_id,
        feed_title,
        category_title,
        title: entry.title,
        url: entry.url,
        author: entry.author.filter(|author| !author.trim().is_empty()),
        published_at_ms,
        html: entry.content,
        content_revision: 0,
        read: entry.status == "read",
        starred: entry.starred,
        reading_minutes: entry.reading_time,
        delivery_state: None,
        delivery_error: None,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use crate::{
        api::miniflux::{CategoryDto, EntriesDto, FeedDto, ServerIdentity},
        model::{Account, PendingMutation, ReaderPosition},
    };

    use super::*;

    struct FakeApi {
        pages: Arc<Mutex<VecDeque<EntriesDto>>>,
        queries: Arc<Mutex<Vec<EntryQuery>>>,
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl MinifluxApi for FakeApi {
        async fn validate(&self) -> Result<ServerIdentity, BrookletError> {
            unreachable!()
        }

        async fn categories(&self) -> Result<Vec<CategoryDto>, BrookletError> {
            Ok(Vec::new())
        }

        async fn feeds(&self) -> Result<Vec<FeedDto>, BrookletError> {
            Ok(Vec::new())
        }

        async fn entries(&self, query: &EntryQuery) -> Result<EntriesDto, BrookletError> {
            self.events.lock().unwrap().push("pull".into());
            self.queries.lock().unwrap().push(query.clone());
            Ok(self.pages.lock().unwrap().pop_front().unwrap())
        }

        async fn entry(&self, _entry_id: i64) -> Result<EntryDto, BrookletError> {
            unreachable!()
        }

        async fn set_read(&self, entry_ids: &[i64], read: bool) -> Result<(), BrookletError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("read:{read}:{entry_ids:?}"));
            Ok(())
        }

        async fn set_starred(&self, entry_ids: &[i64], starred: bool) -> Result<(), BrookletError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("starred:{starred}:{entry_ids:?}"));
            Ok(())
        }

        async fn save_to_integration(&self, _entry_id: i64) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn refresh_feeds(&self) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn subscribe(
            &self,
            _feed_url: &str,
            _category_id: Option<i64>,
        ) -> Result<FeedDto, BrookletError> {
            unreachable!()
        }
    }

    struct FakeFactory {
        pages: Arc<Mutex<VecDeque<EntriesDto>>>,
        queries: Arc<Mutex<Vec<EntryQuery>>>,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl MinifluxApiFactory for FakeFactory {
        fn create(
            &self,
            _server_url: &str,
            token: String,
        ) -> Result<Box<dyn MinifluxApi>, BrookletError> {
            assert_eq!(token, "secret-token");
            Ok(Box::new(FakeApi {
                pages: self.pages.clone(),
                queries: self.queries.clone(),
                events: self.events.clone(),
            }))
        }
    }

    struct MemoryRepository {
        account: Account,
        deleted: AtomicBool,
        entries: Mutex<Vec<Entry>>,
        pending: Mutex<Vec<PendingMutation>>,
        cursor: Mutex<Option<i64>>,
    }

    #[async_trait]
    impl Repository for MemoryRepository {
        async fn account(&self) -> Result<Option<Account>, BrookletError> {
            Ok((!self.deleted.load(Ordering::Acquire)).then(|| self.account.clone()))
        }

        async fn save_account(&self, _account: &Account) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn delete_account(&self, _account_id: i64) -> Result<(), BrookletError> {
            self.deleted.store(true, Ordering::Release);
            self.entries.lock().unwrap().clear();
            self.pending.lock().unwrap().clear();
            Ok(())
        }

        async fn replace_unread_snapshot(
            &self,
            _account_id: i64,
            entries: &[Entry],
        ) -> Result<(), BrookletError> {
            *self.entries.lock().unwrap() = entries.to_vec();
            Ok(())
        }

        async fn merge_changed_page(
            &self,
            _account_id: i64,
            entries: &[Entry],
            removed_ids: &[i64],
        ) -> Result<(), BrookletError> {
            let pending = self.pending.lock().unwrap().clone();
            let mut stored = self.entries.lock().unwrap();
            stored.retain(|entry| {
                !removed_ids.contains(&entry.id)
                    || pending.iter().any(|mutation| mutation.entry_id == entry.id)
            });
            for incoming in entries {
                if let Some(existing) = stored.iter_mut().find(|entry| entry.id == incoming.id) {
                    let read = existing.read;
                    let starred = existing.starred;
                    *existing = incoming.clone();
                    if pending.iter().any(|mutation| {
                        mutation.entry_id == incoming.id && mutation.field == MutationField::Read
                    }) {
                        existing.read = read;
                    }
                    if pending.iter().any(|mutation| {
                        mutation.entry_id == incoming.id && mutation.field == MutationField::Starred
                    }) {
                        existing.starred = starred;
                    }
                } else {
                    stored.push(incoming.clone());
                }
            }
            Ok(())
        }

        async fn unread_entries(&self, _account_id: i64) -> Result<Vec<Entry>, BrookletError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| !entry.read)
                .cloned()
                .collect())
        }

        async fn sync_cursor(&self, _account_id: i64) -> Result<Option<i64>, BrookletError> {
            Ok(*self.cursor.lock().unwrap())
        }

        async fn complete_sync(
            &self,
            _account_id: i64,
            cursor: i64,
            _now_ms: i64,
        ) -> Result<(), BrookletError> {
            *self.cursor.lock().unwrap() = Some(cursor);
            Ok(())
        }

        async fn set_read_local(
            &self,
            _account_id: i64,
            entry_id: i64,
            read: bool,
        ) -> Result<(), BrookletError> {
            if let Some(entry) = self
                .entries
                .lock()
                .unwrap()
                .iter_mut()
                .find(|entry| entry.id == entry_id)
            {
                entry.read = read;
            }
            Ok(())
        }

        async fn pending_mutations(
            &self,
            _account_id: i64,
        ) -> Result<Vec<PendingMutation>, BrookletError> {
            Ok(self.pending.lock().unwrap().clone())
        }

        async fn acknowledge_mutation(
            &self,
            mutation: &PendingMutation,
        ) -> Result<(), BrookletError> {
            self.pending
                .lock()
                .unwrap()
                .retain(|pending| pending != mutation);
            Ok(())
        }

        async fn reader_position(
            &self,
            _entry_id: i64,
        ) -> Result<Option<ReaderPosition>, BrookletError> {
            Ok(None)
        }
    }

    struct MemorySecrets;

    #[async_trait]
    impl SecretStore for MemorySecrets {
        async fn load_miniflux_token(
            &self,
            _account_id: i64,
        ) -> Result<Option<String>, BrookletError> {
            Ok(Some("secret-token".into()))
        }

        async fn store_miniflux_token(
            &self,
            _account_id: i64,
            _token: &str,
        ) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn delete_account_secrets(&self, _account_id: i64) -> Result<(), BrookletError> {
            unreachable!()
        }
    }

    struct LogoutSecrets {
        deleted: AtomicBool,
    }

    #[async_trait]
    impl SecretStore for LogoutSecrets {
        async fn load_miniflux_token(
            &self,
            _account_id: i64,
        ) -> Result<Option<String>, BrookletError> {
            Ok((!self.deleted.load(Ordering::Acquire)).then(|| "secret-token".into()))
        }

        async fn store_miniflux_token(
            &self,
            _account_id: i64,
            _token: &str,
        ) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn delete_account_secrets(&self, _account_id: i64) -> Result<(), BrookletError> {
            self.deleted.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn logout_removes_credentials_and_account_without_a_network_request() {
        let repository = Arc::new(MemoryRepository {
            account: Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            },
            deleted: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
            pending: Mutex::new(Vec::new()),
            cursor: Mutex::new(None),
        });
        let secrets = Arc::new(LogoutSecrets {
            deleted: AtomicBool::new(false),
        });
        let service = AccountSyncService::new(
            repository.clone(),
            secrets.clone(),
            Arc::new(FakeFactory {
                pages: Arc::new(Mutex::new(VecDeque::new())),
                queries: Arc::new(Mutex::new(Vec::new())),
                events: Arc::new(Mutex::new(Vec::new())),
            }),
        );

        service.disconnect().await.unwrap();

        assert!(repository.account().await.unwrap().is_none());
        assert!(secrets.load_miniflux_token(1).await.unwrap().is_none());
    }

    #[test]
    fn maps_miniflux_metadata_for_the_native_inbox() {
        let entry = map_entry(
            1,
            EntryDto {
                id: 42,
                feed_id: 7,
                title: "A useful story".into(),
                url: "https://example.com/story".into(),
                author: Some("Ada".into()),
                published_at: "2026-09-21T12:30:00Z".into(),
                changed_at: "2026-09-21T12:30:00Z".into(),
                content: "<p>Hello</p>".into(),
                status: "unread".into(),
                starred: true,
                reading_time: 4,
                feed: Some(crate::api::miniflux::FeedDto {
                    id: 7,
                    category: Some(crate::api::miniflux::CategoryDto {
                        id: 2,
                        title: "News".into(),
                    }),
                    title: "Example".into(),
                    site_url: String::new(),
                    feed_url: String::new(),
                }),
            },
        );

        assert_eq!(entry.account_id, 1);
        assert_eq!(entry.feed_title, "Example");
        assert_eq!(entry.category_title, "News");
        assert_eq!(entry.reading_minutes, 4);
        assert!(entry.published_at_ms > 0);
    }

    #[tokio::test]
    async fn sync_paginates_then_persists_the_complete_unread_snapshot() {
        let dto = |id| EntryDto {
            id,
            feed_id: 7,
            title: format!("Story {id}"),
            url: format!("https://example.com/{id}"),
            author: None,
            published_at: "2026-09-21T12:30:00Z".into(),
            changed_at: "2026-09-21T12:30:00Z".into(),
            content: "<p>Hello</p>".into(),
            status: "unread".into(),
            starred: false,
            reading_time: 2,
            feed: None,
        };
        let pages = Arc::new(Mutex::new(VecDeque::from([
            EntriesDto {
                total: 101,
                entries: (1..=100).map(dto).collect(),
            },
            EntriesDto {
                total: 101,
                entries: vec![dto(101)],
            },
        ])));
        let queries = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let repository = Arc::new(MemoryRepository {
            account: Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            },
            deleted: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
            pending: Mutex::new(Vec::new()),
            cursor: Mutex::new(None),
        });
        let service = AccountSyncService::new(
            repository,
            Arc::new(MemorySecrets),
            Arc::new(FakeFactory {
                pages,
                queries: queries.clone(),
                events,
            }),
        );

        let result = service.sync().await.unwrap();

        assert_eq!(result.fetched, 101);
        assert_eq!(result.inbox.len(), 101);
        assert_eq!(
            queries
                .lock()
                .unwrap()
                .iter()
                .map(|query| query.offset)
                .collect::<Vec<_>>(),
            [0, 100]
        );
    }

    #[tokio::test]
    async fn sync_pushes_coalesced_intentions_before_pulling() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let repository = Arc::new(MemoryRepository {
            account: Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            },
            deleted: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
            pending: Mutex::new(vec![PendingMutation {
                account_id: 1,
                entry_id: 42,
                field: MutationField::Read,
                desired: true,
            }]),
            cursor: Mutex::new(None),
        });
        let service = AccountSyncService::new(
            repository.clone(),
            Arc::new(MemorySecrets),
            Arc::new(FakeFactory {
                pages: Arc::new(Mutex::new(VecDeque::from([EntriesDto {
                    total: 0,
                    entries: Vec::new(),
                }]))),
                queries: Arc::new(Mutex::new(Vec::new())),
                events: events.clone(),
            }),
        );

        service.sync().await.unwrap();

        assert_eq!(&*events.lock().unwrap(), &["read:true:[42]", "pull"]);
        assert!(repository.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn overlapping_stale_page_cannot_reverse_just_pushed_read_intent() {
        let remote = EntryDto {
            id: 42,
            feed_id: 7,
            title: "Story".into(),
            url: "https://example.com/42".into(),
            author: None,
            published_at: "1970-01-01T00:16:40Z".into(),
            changed_at: "1970-01-01T00:16:40Z".into(),
            content: "<p>Story</p>".into(),
            status: "unread".into(),
            starred: false,
            reading_time: 2,
            feed: None,
        };
        let mut local = map_entry(1, remote.clone());
        local.read = true;
        let repository = Arc::new(MemoryRepository {
            account: Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            },
            deleted: AtomicBool::new(false),
            entries: Mutex::new(vec![local]),
            pending: Mutex::new(vec![PendingMutation {
                account_id: 1,
                entry_id: 42,
                field: MutationField::Read,
                desired: true,
            }]),
            cursor: Mutex::new(Some(1_000)),
        });
        let queries = Arc::new(Mutex::new(Vec::new()));
        let service = AccountSyncService::new(
            repository.clone(),
            Arc::new(MemorySecrets),
            Arc::new(FakeFactory {
                pages: Arc::new(Mutex::new(VecDeque::from([EntriesDto {
                    total: 1,
                    entries: vec![remote],
                }]))),
                queries: queries.clone(),
                events: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        let result = service.sync().await.unwrap();
        assert!(result.inbox.is_empty());
        assert_eq!(queries.lock().unwrap()[0].changed_after, Some(940));
        assert!(repository.pending.lock().unwrap().is_empty());
        assert!(repository.entries.lock().unwrap()[0].read);
    }
}
