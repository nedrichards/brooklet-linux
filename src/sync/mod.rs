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

pub trait KarakeepApiFactory: Send + Sync {
    fn create(&self, endpoint: &str, key: String) -> Result<Box<dyn KarakeepApi>, BrookletError>;
}
pub struct ReqwestKarakeepApiFactory;
impl KarakeepApiFactory for ReqwestKarakeepApiFactory {
    fn create(&self, endpoint: &str, key: String) -> Result<Box<dyn KarakeepApi>, BrookletError> {
        Ok(Box::new(ReqwestKarakeepApi::new(endpoint, key)?))
    }
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
        scope: &str,
    ) -> Result<Vec<Entry>, BrookletError>;
    async fn categories_cached(&self) -> Result<Vec<Category>, BrookletError>;
    async fn feeds_cached(&self, category_id: Option<i64>) -> Result<Vec<Feed>, BrookletError>;
    async fn reader_position(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<ReaderPosition>, BrookletError>;
    async fn save_reader_position(&self, position: &ReaderPosition) -> Result<(), BrookletError>;
    async fn queue_karakeep(&self, entry: &Entry) -> Result<(), BrookletError>;
    async fn unfinished_karakeep(&self) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        Ok(Vec::new())
    }
    /// Retry with current settings; None dismisses only the local unfinished receipt.
    async fn recover_karakeep(&self, delivery_id: i64, retry: bool) -> Result<(), BrookletError> {
        let _ = (delivery_id, retry);
        Err(BrookletError::InvalidSetup("delivery recovery support"))
    }
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
    /// Upload queued changes without fetching articles.
    async fn flush_outgoing(&self) -> Result<bool, BrookletError>;
}

pub struct AccountSyncService {
    repository: Arc<dyn Repository>,
    secrets: Arc<dyn SecretStore>,
    api_factory: Arc<dyn MinifluxApiFactory>,
    karakeep_factory: Arc<dyn KarakeepApiFactory>,
    sync_lock: tokio::sync::Mutex<()>,
    running: AtomicBool,
    mutation_lock: tokio::sync::Mutex<()>,
    delivering: AtomicBool,
}

struct RunningGuard<'a>(&'a AtomicBool);

impl<'a> RunningGuard<'a> {
    fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Release);
        Self(flag)
    }
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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
            karakeep_factory: Arc::new(ReqwestKarakeepApiFactory),
            sync_lock: tokio::sync::Mutex::new(()),
            running: AtomicBool::new(false),
            mutation_lock: tokio::sync::Mutex::new(()),
            delivering: AtomicBool::new(false),
        }
    }

    pub fn with_karakeep_factory(mut self, factory: Arc<dyn KarakeepApiFactory>) -> Self {
        self.karakeep_factory = factory;
        self
    }

    async fn direct_karakeep_api(
        &self,
        account_id: i64,
    ) -> Result<Box<dyn KarakeepApi>, BrookletError> {
        let config = self
            .repository
            .karakeep_config(account_id)
            .await?
            .ok_or(BrookletError::InvalidSetup("a Karakeep API endpoint"))?;
        let endpoint = config
            .direct_endpoint
            .as_deref()
            .ok_or(BrookletError::InvalidSetup("a Karakeep API endpoint"))?;
        let key = self
            .secrets
            .load_karakeep_key(account_id)
            .await?
            .filter(|key| !key.trim().is_empty())
            .ok_or(BrookletError::InvalidSetup("a Karakeep API key"))?;
        self.karakeep_factory.create(endpoint, key)
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
        let _running = RunningGuard::new(&self.running);
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
        result
    }

    async fn upload_mutations(
        &self,
        account: &crate::model::Account,
        api: &dyn MinifluxApi,
    ) -> Result<(), BrookletError> {
        let pending = self.repository.pending_mutations(account.id).await?;
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
                // Commit each accepted batch before any subsequent request can fail.
                for mutation in &batch {
                    self.repository.acknowledge_mutation(mutation).await?;
                }
            }
        }
        Ok(())
    }

    async fn upload_karakeep(
        &self,
        account: &crate::model::Account,
        api: &dyn MinifluxApi,
    ) -> Result<(), BrookletError> {
        for delivery in self.repository.pending_karakeep(account.id).await? {
            let result = match delivery.route {
                KarakeepRoute::Miniflux => api.save_to_integration(delivery.entry_id).await,
                KarakeepRoute::Direct => match self.direct_karakeep_api(account.id).await {
                    Ok(client) => client.save(&delivery.canonical_url, &delivery.title).await,
                    Err(error) => Err(error),
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
                            .defer_karakeep(
                                delivery.id,
                                &delivery_error_message(delivery.route, &error),
                            )
                            .await?;
                        return Err(error);
                    }
                    self.repository
                        .finish_karakeep(
                            delivery.id,
                            Some(&delivery_error_message(delivery.route, &error)),
                            now_ms(),
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    // Delivery and refresh have separate durable status. Service failures leave
    // queued intentions protected during merge; local storage failures still abort.
    async fn record_delivery_attempt(
        &self,
        account_id: i64,
        result: Result<(), BrookletError>,
    ) -> Result<(), BrookletError> {
        match result {
            Err(error @ (BrookletError::Database(_) | BrookletError::Storage(_))) => Err(error),
            result => {
                let error = result.err();
                if let Some(error) = &error {
                    tracing::warn!(account_id, failure_kind = ?error.failure_kind(),
                        "delivery failed; continuing incoming sync with pending intentions protected");
                }
                let message = error.as_ref().map(ToString::to_string);
                self.repository
                    .record_delivery_error(account_id, message.as_deref())
                    .await
            }
        }
    }

    async fn run_sync_account(
        &self,
        account: &crate::model::Account,
        refresh_feeds: bool,
    ) -> Result<SyncResult, BrookletError> {
        let api = self.api(account).await?;
        {
            let _guard = self.mutation_lock.lock().await;
            let delivery = async {
                self.upload_mutations(account, api.as_ref()).await?;
                self.upload_karakeep(account, api.as_ref()).await
            }
            .await;
            self.record_delivery_attempt(account.id, delivery).await?;
        }
        if refresh_feeds {
            api.refresh_feeds().await?;
        }
        let result = self.pull_incoming(account, api.as_ref()).await?;
        if !refresh_feeds {
            return Ok(result);
        }
        // Miniflux refreshes all feeds in the background and exposes no job token.
        // Poll every round even if one feed produced entries: other feeds may finish later.
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut result = result;
            for seconds in [2, 5, 10] {
                tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
                result = self.pull_incoming(account, api.as_ref()).await?;
            }
            Ok(result)
        })
        .await
        .map_err(|_| BrookletError::RefreshFollowUpTimeout)?
    }

    async fn pull_incoming(
        &self,
        account: &crate::model::Account,
        api: &dyn MinifluxApi,
    ) -> Result<SyncResult, BrookletError> {
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
                parsing_error_message: dto.parsing_error_message,
                parsing_error_count: dto.parsing_error_count,
                disabled: dto.disabled,
            })
            .collect::<Vec<_>>();
        self.repository
            .merge_metadata(account.id, &categories, &feeds)
            .await?;
        let cursor = self.repository.sync_cursor(account.id).await?.unwrap_or(0);
        let overlap = incremental_start(Some(cursor), 60);
        // Freeze advancement using a server-observed timestamp, not the client clock.
        let watermark = api
            .entries(&EntryQuery {
                limit: 1,
                ..EntryQuery::default()
            })
            .await?;
        if watermark.entries.len() > 1 {
            return Err(BrookletError::SyncResponse("invalid watermark page"));
        }
        let ceiling = watermark
            .entries
            .first()
            .map(|dto| {
                dto.changed_at
                    .parse::<jiff::Timestamp>()
                    .map(|time| time.as_second())
                    .map_err(|_| BrookletError::SyncResponse("invalid change timestamp"))
            })
            .transpose()?;
        let mut newest = cursor;
        let mut after_entry_id = None;
        let mut fetched = 0;
        loop {
            // Serialize each fetch+merge with uploads: a response fetched before
            // an upload must be merged before its local protection is acknowledged.
            let _guard = self.mutation_lock.lock().await;
            let page = api
                .entries(&EntryQuery {
                    status: None,
                    changed_after: Some(overlap),
                    changed_before: ceiling.map(|time| time.saturating_add(1)),
                    order: "id",
                    direction: "asc",
                    after_entry_id,
                    limit: PAGE_SIZE,
                    offset: 0,
                })
                .await?;
            let page_len = page.entries.len();
            if page_len > PAGE_SIZE {
                return Err(BrookletError::SyncResponse("oversized entry page"));
            }
            let mut previous = after_entry_id.unwrap_or(0);
            for dto in &page.entries {
                if dto.id <= previous {
                    return Err(BrookletError::SyncResponse(
                        "entry pagination did not advance",
                    ));
                }
                previous = dto.id;
                if dto.changed_at.parse::<jiff::Timestamp>().is_err() {
                    return Err(BrookletError::SyncResponse("invalid change timestamp"));
                }
                if !matches!(dto.status.as_str(), "read" | "unread" | "removed") {
                    return Err(BrookletError::SyncResponse("unknown entry status"));
                }
            }
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
            after_entry_id = Some(previous);
            if page_len < PAGE_SIZE || ceiling.is_none() {
                break;
            }
        }
        // A write behind the ID cursor will be replayed on the next pull.
        newest = cursor.max(newest.min(ceiling.unwrap_or(cursor)));
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.reconcile_missing(account, api),
        )
        .await
        .map_err(|_| BrookletError::SyncResponse("remote reconciliation timed out"))??;
        self.repository
            .complete_sync(account.id, newest, now_ms())
            .await?;
        self.repository
            .apply_retention(account.id, now_ms())
            .await?;
        let inbox = self.repository.unread_entries(account.id).await?;
        Ok(SyncResult { inbox, fetched })
    }
    async fn reconcile_missing(
        &self,
        account: &crate::model::Account,
        api: &dyn MinifluxApi,
    ) -> Result<(), BrookletError> {
        let cached = self.repository.cached_entry_ids(account.id).await?;
        if cached.is_empty() {
            return Ok(());
        }
        const LIMIT: usize = 10_000;
        let mut remote = std::collections::HashSet::new();
        let mut offset = 0;
        let mut bound = None;
        loop {
            let page = api.entry_ids(LIMIT, offset).await?;
            let count = page.entry_ids.len();
            if count > LIMIT || page.entry_ids.iter().any(|id| *id <= 0) {
                return Err(BrookletError::SyncResponse("invalid entry inventory"));
            }
            let added = page
                .entry_ids
                .into_iter()
                .filter(|id| remote.insert(*id))
                .count();
            if count > 0 && added == 0 {
                return Err(BrookletError::SyncResponse(
                    "inventory pagination did not advance",
                ));
            }
            let initial_total = *bound.get_or_insert(page.total);
            offset += count;
            if count == 0 || offset >= initial_total || cached.iter().all(|id| remote.contains(id))
            {
                break;
            }
        }
        for id in cached.into_iter().filter(|id| !remote.contains(id)) {
            let _guard = self.mutation_lock.lock().await;
            // Absence in an offset-paged inventory is only a hint. Confirm it
            // individually before deleting: concurrent changes can shift those pages.
            match api.entry(id).await {
                Err(BrookletError::Http { status: 404, .. }) => {
                    self.repository
                        .merge_changed_page(account.id, &[], &[id])
                        .await?;
                }
                Ok(dto) if dto.id == id && dto.status == "removed" => {
                    self.repository
                        .merge_changed_page(account.id, &[], &[id])
                        .await?;
                }
                Ok(dto) if dto.id == id && matches!(dto.status.as_str(), "read" | "unread") => {
                    self.repository
                        .merge_changed_page(account.id, &[map_entry(account.id, dto)], &[])
                        .await?;
                }
                Ok(_) => return Err(BrookletError::SyncResponse("invalid entry confirmation")),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[async_trait]
impl SyncService for AccountSyncService {
    async fn flush_outgoing(&self) -> Result<bool, BrookletError> {
        let _guard = self.mutation_lock.lock().await;
        let Some(account) = self.repository.account().await? else {
            return Ok(false);
        };
        if self
            .repository
            .pending_mutations(account.id)
            .await?
            .is_empty()
            && self
                .repository
                .pending_karakeep(account.id)
                .await?
                .is_empty()
        {
            return Ok(false);
        }
        let _delivering = RunningGuard::new(&self.delivering);
        let result = async {
            let api = self.api(&account).await?;
            self.upload_mutations(&account, api.as_ref()).await?;
            self.upload_karakeep(&account, api.as_ref()).await
        }
        .await;
        let error = result.as_ref().err().map(ToString::to_string);
        self.repository
            .record_delivery_error(account.id, error.as_deref())
            .await?;
        result?;
        Ok(!self
            .repository
            .pending_mutations(account.id)
            .await?
            .is_empty()
            || !self
                .repository
                .pending_karakeep(account.id)
                .await?
                .is_empty())
    }

    async fn cached_entry(&self, entry_id: EntryId) -> Result<Option<Entry>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.cached_entry(account.id, entry_id).await
    }

    async fn disconnect(&self) -> Result<(), BrookletError> {
        let _guard = self.sync_lock.lock().await;
        let _mutation_guard = self.mutation_lock.lock().await;
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
        scope: &str,
    ) -> Result<Vec<Entry>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository
            .search_entries(account.id, query, feed_id, category_id, read, scope)
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
    async fn unfinished_karakeep(&self) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        let account = self.configured_account().await?;
        self.repository.unfinished_karakeep(account.id).await
    }
    async fn recover_karakeep(&self, delivery_id: i64, retry: bool) -> Result<(), BrookletError> {
        let _guard = self.sync_lock.lock().await;
        let _mutation_guard = self.mutation_lock.lock().await;
        let account = self.configured_account().await?;
        let route = if retry {
            Some(
                self.repository
                    .karakeep_config(account.id)
                    .await?
                    .map_or(KarakeepRoute::Miniflux, |config| config.route),
            )
        } else {
            None
        };
        self.repository
            .recover_karakeep(account.id, delivery_id, route)
            .await
    }
    async fn save_karakeep_config(
        &self,
        config: &KarakeepConfig,
        key: Option<String>,
    ) -> Result<(), BrookletError> {
        let _guard = self.sync_lock.lock().await;
        let _mutation_guard = self.mutation_lock.lock().await;
        let account = self.configured_account().await?;
        if config.route != KarakeepRoute::Direct {
            return self
                .repository
                .save_karakeep_config(account.id, config)
                .await;
        }
        let endpoint = config
            .direct_endpoint
            .as_deref()
            .ok_or(BrookletError::InvalidSetup("a Karakeep API endpoint"))?;
        crate::services::url_policy::karakeep_url(endpoint)?;
        let old_key = self.secrets.load_karakeep_key(account.id).await?;
        let key = key
            .filter(|key| !key.trim().is_empty())
            .or_else(|| old_key.clone())
            .filter(|key| !key.trim().is_empty())
            .ok_or(BrookletError::InvalidSetup("a Karakeep API key"))?;
        self.karakeep_factory
            .create(endpoint, key.clone())?
            .validate()
            .await?;
        let replacing_key = old_key.as_ref() != Some(&key);
        if replacing_key {
            self.secrets.store_karakeep_key(account.id, &key).await?;
        }
        if let Err(error) = self
            .repository
            .save_karakeep_config(account.id, config)
            .await
        {
            // A failed metadata write must not silently replace the working key.
            if replacing_key {
                match old_key {
                    Some(old) => self.secrets.store_karakeep_key(account.id, &old).await?,
                    None => self.secrets.delete_karakeep_key(account.id).await?,
                }
            }
            return Err(error);
        }
        Ok(())
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
        status.running =
            self.running.load(Ordering::Acquire) || self.delivering.load(Ordering::Acquire);
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

fn delivery_error_message(route: KarakeepRoute, error: &BrookletError) -> String {
    match route {
        KarakeepRoute::Direct => error.karakeep_message(),
        KarakeepRoute::Miniflux => format!(
            "{} Check Miniflux’s Karakeep integration before retrying.",
            error.sync_message()
        ),
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

    #[derive(Default)]
    struct ApiControls {
        fail_star: AtomicBool,
        fail_read: AtomicBool,
        fail_refresh: AtomicBool,
        save_status: std::sync::atomic::AtomicU16,
        fail_pull: AtomicBool,
        read_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        pull_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        read_started: tokio::sync::Notify,
        pull_started: tokio::sync::Notify,
    }

    async fn gate(
        slot: &Mutex<Option<Arc<tokio::sync::Semaphore>>>,
        started: &tokio::sync::Notify,
    ) {
        let gate = slot.lock().unwrap().clone();
        started.notify_one();
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
    }

    struct FakeApi {
        controls: Arc<ApiControls>,
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
            if query.limit == 1 {
                let pages = self.pages.lock().unwrap();
                let page = pages.front().expect("watermark fixture");
                return Ok(EntriesDto {
                    total: page.total,
                    entries: page.entries.iter().take(1).cloned().collect(),
                });
            }
            self.events.lock().unwrap().push("pull".into());
            self.queries.lock().unwrap().push(query.clone());
            gate(&self.controls.pull_gate, &self.controls.pull_started).await;
            if self.controls.fail_pull.load(Ordering::Acquire) {
                return Err(BrookletError::Http {
                    status: 503,
                    kind: crate::model::FailureKind::Retryable,
                });
            }
            Ok(self.pages.lock().unwrap().pop_front().unwrap())
        }

        async fn entry_ids(
            &self,
            _: usize,
            _: usize,
        ) -> Result<crate::api::miniflux::EntryIdsDto, BrookletError> {
            let entry_ids: Vec<i64> = (1..=200).collect();
            Ok(crate::api::miniflux::EntryIdsDto {
                total: entry_ids.len(),
                entry_ids,
            })
        }
        async fn entry(&self, _entry_id: i64) -> Result<EntryDto, BrookletError> {
            unreachable!()
        }

        async fn set_read(&self, entry_ids: &[i64], read: bool) -> Result<(), BrookletError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("read:{read}:{entry_ids:?}"));
            gate(&self.controls.read_gate, &self.controls.read_started).await;
            if self.controls.fail_read.load(Ordering::Acquire) {
                return Err(BrookletError::Http {
                    status: 503,
                    kind: crate::model::FailureKind::Retryable,
                });
            }
            Ok(())
        }

        async fn set_starred(&self, entry_ids: &[i64], starred: bool) -> Result<(), BrookletError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("starred:{starred}:{entry_ids:?}"));
            if self.controls.fail_star.load(Ordering::Acquire) {
                return Err(BrookletError::Http {
                    status: 503,
                    kind: crate::model::FailureKind::Retryable,
                });
            }
            Ok(())
        }

        async fn save_to_integration(&self, entry_id: i64) -> Result<(), BrookletError> {
            self.events.lock().unwrap().push(format!("save:{entry_id}"));
            let status = self.controls.save_status.load(Ordering::Acquire);
            if status != 0 {
                return Err(BrookletError::Http {
                    status,
                    kind: crate::model::classify_http_status(status),
                });
            }
            Ok(())
        }

        async fn refresh_feeds(&self) -> Result<(), BrookletError> {
            self.events.lock().unwrap().push("refresh".into());
            if self.controls.fail_refresh.load(Ordering::Acquire) {
                return Err(BrookletError::Http {
                    status: 503,
                    kind: crate::model::FailureKind::Retryable,
                });
            }
            Ok(())
        }

        async fn subscribe(
            &self,
            _feed_url: &str,
            _category_id: Option<i64>,
        ) -> Result<i64, BrookletError> {
            unreachable!()
        }
    }

    struct FakeFactory {
        controls: Arc<ApiControls>,
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
                controls: self.controls.clone(),
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

    struct Harness {
        repository: Arc<crate::storage::sqlite::SqliteRepository>,
        service: Arc<AccountSyncService>,
        controls: Arc<ApiControls>,
        events: Arc<Mutex<Vec<String>>>,
        pages: Arc<Mutex<VecDeque<EntriesDto>>>,
    }

    fn remote_entry(id: i64, read: bool) -> EntryDto {
        EntryDto {
            id,
            feed_id: 7,
            title: format!("Story {id}"),
            url: format!("https://example.com/{id}"),
            author: None,
            published_at: "2026-10-02T12:00:00Z".into(),
            changed_at: "2026-10-02T12:00:00Z".into(),
            content: "<p>Story</p>".into(),
            status: if read { "read" } else { "unread" }.into(),
            starred: false,
            reading_time: 1,
            feed: None,
        }
    }

    impl Harness {
        async fn new() -> Self {
            Self::with_repository(Arc::new(
                crate::storage::sqlite::SqliteRepository::open_in_memory().unwrap(),
            ))
            .await
        }

        async fn with_repository(
            repository: Arc<crate::storage::sqlite::SqliteRepository>,
        ) -> Self {
            if repository.account().await.unwrap().is_none() {
                repository
                    .save_account(&Account {
                        id: 1,
                        server_url: "https://miniflux.example".into(),
                        username: "reader".into(),
                        server_version: "2.3.2".into(),
                    })
                    .await
                    .unwrap();
                repository
                    .merge_changed_page(
                        1,
                        &[
                            map_entry(1, remote_entry(42, false)),
                            map_entry(1, remote_entry(43, false)),
                        ],
                        &[],
                    )
                    .await
                    .unwrap();
            }
            let controls = Arc::new(ApiControls::default());
            let events = Arc::new(Mutex::new(Vec::new()));
            let pages = Arc::new(Mutex::new(VecDeque::from([EntriesDto {
                total: 0,
                entries: Vec::new(),
            }])));
            let service = Arc::new(AccountSyncService::new(
                repository.clone(),
                Arc::new(MemorySecrets),
                Arc::new(FakeFactory {
                    controls: controls.clone(),
                    events: events.clone(),
                    pages: pages.clone(),
                    queries: Arc::new(Mutex::new(Vec::new())),
                }),
            ));
            Self {
                repository,
                service,
                controls,
                events,
                pages,
            }
        }
    }

    async fn started(notification: &tokio::sync::Notify) {
        tokio::time::timeout(std::time::Duration::from_secs(3), notification.notified())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn standalone_delivery_batches_read_unread_star_and_unstar_without_pulling() {
        let harness = Harness::new().await;
        harness
            .service
            .set_read_many_local(&[42, 43], true)
            .await
            .unwrap();
        harness.service.set_read_local(43, false).await.unwrap();
        harness.service.set_starred_local(42, true).await.unwrap();
        harness.service.set_starred_local(43, false).await.unwrap();
        assert!(!harness.service.flush_outgoing().await.unwrap());
        assert_eq!(
            *harness.events.lock().unwrap(),
            [
                "read:false:[43]",
                "read:true:[42]",
                "starred:false:[43]",
                "starred:true:[42]",
            ]
        );
        assert_eq!(
            harness
                .repository
                .sync_status(1)
                .await
                .unwrap()
                .last_successful_sync_at_ms,
            None
        );
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn bulk_reads_use_one_upload_and_failed_uploads_keep_the_entire_batch() {
        let harness = Harness::new().await;
        harness
            .service
            .set_read_many_local(&[42, 43], true)
            .await
            .unwrap();
        harness.controls.fail_read.store(true, Ordering::Release);
        assert!(harness.service.flush_outgoing().await.is_err());
        assert_eq!(
            harness.repository.pending_mutations(1).await.unwrap().len(),
            2
        );
        harness.controls.fail_read.store(false, Ordering::Release);
        harness.service.flush_outgoing().await.unwrap();
        assert_eq!(
            *harness.events.lock().unwrap(),
            ["read:true:[42, 43]", "read:true:[42, 43]"]
        );
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn queue_notifications_deliver_only_the_latest_committed_actions() {
        let harness = Harness::new().await;
        let (notifications, changes) = tokio::sync::watch::channel(0);
        let writes =
            crate::outgoing::local_writes(&tokio::runtime::Handle::current(), notifications);
        let service = harness.service.clone();
        let worker = tokio::spawn(crate::outgoing::deliver(changes, move || {
            let service = service.clone();
            async move { service.flush_outgoing().await }
        }));
        let mut replies = Vec::new();
        for read in [true, false, true] {
            let service = harness.service.clone();
            let (reply, result) = tokio::sync::oneshot::channel();
            writes
                .send((
                    Box::pin(async move { service.set_read_local(42, read).await }),
                    reply,
                ))
                .unwrap();
            replies.push(result);
        }
        for reply in replies {
            reply.await.unwrap().unwrap();
        }
        started(&harness.controls.read_started).await;
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["read:true:[42]"]);
        assert!(
            harness
                .repository
                .cached_entry(1, 42)
                .await
                .unwrap()
                .unwrap()
                .read
        );
        worker.abort();
    }

    #[tokio::test]
    async fn karakeep_retry_does_not_replay_read_and_terminal_errors_stop_automatic_delivery() {
        let harness = Harness::new().await;
        harness.service.set_read_local(42, true).await.unwrap();
        let entry = harness
            .repository
            .cached_entry(1, 42)
            .await
            .unwrap()
            .unwrap();
        harness.service.queue_karakeep(&entry).await.unwrap();
        harness.controls.save_status.store(503, Ordering::Release);
        assert!(harness.service.flush_outgoing().await.is_err());
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            harness.repository.pending_karakeep(1).await.unwrap().len(),
            1
        );
        harness.events.lock().unwrap().clear();
        harness.controls.save_status.store(400, Ordering::Release);
        assert!(!harness.service.flush_outgoing().await.unwrap());
        assert_eq!(*harness.events.lock().unwrap(), ["save:42"]);
        assert!(
            harness
                .repository
                .pending_karakeep(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            harness
                .repository
                .cached_entry(1, 42)
                .await
                .unwrap()
                .unwrap()
                .delivery_state,
            Some(crate::model::DeliveryState::NeedsAttention)
        );
        harness.events.lock().unwrap().clear();
        harness.service.flush_outgoing().await.unwrap();
        assert!(harness.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn accepted_karakeep_delivery_is_not_replayed_by_the_following_sync() {
        let harness = Harness::new().await;
        let entry = harness
            .repository
            .cached_entry(1, 42)
            .await
            .unwrap()
            .unwrap();
        harness.service.queue_karakeep(&entry).await.unwrap();
        harness.service.flush_outgoing().await.unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["save:42"]);
        harness.events.lock().unwrap().clear();
        harness.service.sync().await.unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["pull"]);
    }

    #[tokio::test]
    async fn queued_actions_are_accepted_before_server_feed_refresh_can_fail() {
        let harness = Harness::new().await;
        harness.service.set_read_local(42, true).await.unwrap();
        harness.controls.fail_refresh.store(true, Ordering::Release);
        assert!(harness.service.refresh_feeds().await.is_err());
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            *harness.events.lock().unwrap(),
            ["read:true:[42]", "refresh"]
        );
    }

    #[tokio::test]
    async fn successful_batch_is_acknowledged_before_a_later_batch_fails() {
        let harness = Harness::new().await;
        harness.service.set_read_local(42, true).await.unwrap();
        harness.service.set_starred_local(43, true).await.unwrap();
        harness.controls.fail_star.store(true, Ordering::Release);
        assert!(harness.service.flush_outgoing().await.is_err());
        assert!(
            harness
                .repository
                .sync_status(1)
                .await
                .unwrap()
                .error
                .is_some()
        );
        assert_eq!(harness.repository.sync_cursor(1).await.unwrap(), None);
        let pending = harness.repository.pending_mutations(1).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].field, MutationField::Starred);
        harness.events.lock().unwrap().clear();
        harness.controls.fail_star.store(false, Ordering::Release);
        assert!(!harness.service.flush_outgoing().await.unwrap());
        assert_eq!(*harness.events.lock().unwrap(), ["starred:true:[43]"]);
        assert_eq!(harness.repository.sync_status(1).await.unwrap().error, None);
    }

    #[tokio::test]
    async fn a_failed_download_never_replays_an_accepted_write() {
        let harness = Harness::new().await;
        harness.service.set_read_local(42, true).await.unwrap();
        harness.controls.fail_pull.store(true, Ordering::Release);
        assert!(harness.service.sync().await.is_err());
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
        harness.events.lock().unwrap().clear();
        harness.controls.fail_pull.store(false, Ordering::Release);
        harness.service.sync().await.unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["pull"]);
    }

    #[tokio::test]
    async fn another_clients_newer_state_is_imported_after_our_upload() {
        let harness = Harness::new().await;
        harness.service.set_read_local(42, true).await.unwrap();
        harness.service.flush_outgoing().await.unwrap();
        // The server's current state now reflects another client's unread action.
        *harness.pages.lock().unwrap() = VecDeque::from([EntriesDto {
            total: 1,
            entries: vec![remote_entry(42, false)],
        }]);
        harness.events.lock().unwrap().clear();
        harness.service.sync().await.unwrap();
        assert!(
            !harness
                .repository
                .cached_entry(1, 42)
                .await
                .unwrap()
                .unwrap()
                .read
        );
        assert_eq!(*harness.events.lock().unwrap(), ["pull"]);
    }

    #[tokio::test]
    async fn read_unread_read_while_uploading_retains_the_latest_revision_for_delivery() {
        let harness = Harness::new().await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness.controls.read_gate.lock().unwrap() = Some(gate.clone());
        harness.service.set_read_local(42, true).await.unwrap();
        let first = harness
            .repository
            .pending_mutations(1)
            .await
            .unwrap()
            .remove(0);
        let service = harness.service.clone();
        let upload = tokio::spawn(async move { service.flush_outgoing().await });
        started(&harness.controls.read_started).await;
        harness.service.set_read_local(42, false).await.unwrap();
        harness.service.set_read_local(42, true).await.unwrap();
        *harness.controls.read_gate.lock().unwrap() = None;
        gate.add_permits(1);
        assert!(upload.await.unwrap().unwrap());
        let latest = harness
            .repository
            .pending_mutations(1)
            .await
            .unwrap()
            .remove(0);
        assert!(latest.revision > first.revision);
        assert!(latest.desired);
        assert!(!harness.service.flush_outgoing().await.unwrap());
        assert_eq!(
            *harness.events.lock().unwrap(),
            ["read:true:[42]", "read:true:[42]"]
        );
    }

    #[tokio::test]
    async fn stale_inflight_page_merges_before_upload_acknowledgement() {
        let harness = Harness::new().await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness.controls.pull_gate.lock().unwrap() = Some(gate.clone());
        *harness.pages.lock().unwrap() = VecDeque::from([EntriesDto {
            total: 1,
            entries: vec![remote_entry(42, false)],
        }]);
        let service = harness.service.clone();
        let refresh = tokio::spawn(async move { service.sync().await });
        started(&harness.controls.pull_started).await;
        harness.service.set_read_local(42, true).await.unwrap();
        let service = harness.service.clone();
        let upload = tokio::spawn(async move { service.flush_outgoing().await });
        tokio::task::yield_now().await;
        assert_eq!(*harness.events.lock().unwrap(), ["pull"]);
        gate.add_permits(1);
        refresh.await.unwrap().unwrap();
        upload.await.unwrap().unwrap();
        assert!(
            harness
                .repository
                .cached_entry(1, 42)
                .await
                .unwrap()
                .unwrap()
                .read
        );
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(*harness.events.lock().unwrap(), ["pull", "read:true:[42]"]);
    }

    #[tokio::test]
    async fn cancelled_refresh_releases_upload_lock_and_clears_running_status() {
        let harness = Harness::new().await;
        *harness.controls.pull_gate.lock().unwrap() =
            Some(Arc::new(tokio::sync::Semaphore::new(0)));
        let service = harness.service.clone();
        let refresh = tokio::spawn(async move { service.sync().await });
        started(&harness.controls.pull_started).await;
        assert!(harness.service.sync_status().await.unwrap().running);
        harness.service.set_read_local(42, true).await.unwrap();
        refresh.abort();
        assert!(refresh.await.unwrap_err().is_cancelled());
        assert!(!harness.service.sync_status().await.unwrap().running);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            harness.service.flush_outgoing(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(*harness.events.lock().unwrap(), ["pull", "read:true:[42]"]);
    }

    #[tokio::test]
    async fn simultaneous_manual_sync_and_delivery_do_not_upload_the_same_snapshot_twice() {
        let harness = Harness::new().await;
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *harness.controls.read_gate.lock().unwrap() = Some(gate.clone());
        harness.service.set_read_local(42, true).await.unwrap();
        let service = harness.service.clone();
        let upload = tokio::spawn(async move { service.flush_outgoing().await });
        started(&harness.controls.read_started).await;
        let service = harness.service.clone();
        let refresh = tokio::spawn(async move { service.sync().await });
        gate.add_permits(1);
        upload.await.unwrap().unwrap();
        refresh.await.unwrap().unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["read:true:[42]", "pull"]);
    }

    #[tokio::test]
    async fn startup_delivers_persisted_changes_after_a_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("offline.db");
        {
            let harness = Harness::with_repository(Arc::new(
                crate::storage::sqlite::SqliteRepository::open(&path).unwrap(),
            ))
            .await;
            harness.service.set_read_local(42, true).await.unwrap();
        }
        let harness = Harness::with_repository(Arc::new(
            crate::storage::sqlite::SqliteRepository::open(&path).unwrap(),
        ))
        .await;
        let (_sender, changes) = tokio::sync::watch::channel(0);
        let service = harness.service.clone();
        let worker = tokio::spawn(crate::outgoing::deliver(changes, move || {
            let service = service.clone();
            async move { service.flush_outgoing().await }
        }));
        started(&harness.controls.read_started).await;
        // Wait for the HTTP success to have been committed to the real database.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !harness
                .repository
                .pending_mutations(1)
                .await
                .unwrap()
                .is_empty()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(*harness.events.lock().unwrap(), ["read:true:[42]"]);
        worker.abort();
    }

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
                controls: Arc::new(ApiControls::default()),
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
                    parsing_error_message: String::new(),
                    parsing_error_count: 0,
                    disabled: false,
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
                controls: Arc::new(ApiControls::default()),
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
                .map(|query| query.after_entry_id)
                .collect::<Vec<_>>(),
            [None, Some(100)]
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
                revision: 1,
            }]),
            cursor: Mutex::new(None),
        });
        let service = AccountSyncService::new(
            repository.clone(),
            Arc::new(MemorySecrets),
            Arc::new(FakeFactory {
                controls: Arc::new(ApiControls::default()),
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
    async fn sync_merges_confirmed_remote_state_after_acknowledging_upload() {
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
                revision: 1,
            }]),
            cursor: Mutex::new(Some(1_000)),
        });
        let queries = Arc::new(Mutex::new(Vec::new()));
        let service = AccountSyncService::new(
            repository.clone(),
            Arc::new(MemorySecrets),
            Arc::new(FakeFactory {
                controls: Arc::new(ApiControls::default()),
                pages: Arc::new(Mutex::new(VecDeque::from([EntriesDto {
                    total: 1,
                    entries: vec![EntryDto {
                        status: "read".into(),
                        ..remote
                    }],
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
