use std::sync::Arc;

use crate::outgoing::LocalWrite;

use adw::glib;

use crate::{
    error::BrookletError,
    model::{
        Account, Category, Entry, EntryId, Feed, KarakeepConfig, KarakeepDelivery, ReaderPosition,
        StoragePolicy, SyncStatus,
    },
    setup::{SetupRequest, SetupService},
    sync::{SyncResult, SyncService},
};

pub struct AppController {
    runtime: tokio::runtime::Runtime,
    account_operations: Arc<tokio::sync::Mutex<()>>,
    setup_service: Arc<dyn SetupService>,
    sync_service: Arc<dyn SyncService>,
    image_cache: Arc<crate::services::image_cache::ImageCache>,
    image_decoders: Arc<tokio::sync::Semaphore>,
    parsers: Arc<tokio::sync::Semaphore>,
    local_writes: tokio::sync::mpsc::UnboundedSender<LocalWrite>,
    outgoing_changes: tokio::sync::watch::Sender<u64>,
}

impl AppController {
    pub fn cached_entry(
        &self,
        entry_id: EntryId,
        callback: impl FnOnce(Result<Option<Entry>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.cached_entry(entry_id).await },
            callback,
        );
    }

    pub fn parse_entry(
        &self,
        entry_id: EntryId,
        callback: impl FnOnce(
            Result<(Vec<crate::model::DocumentBlock>, Option<ReaderPosition>), BrookletError>,
        ) + 'static,
    ) -> tokio::task::AbortHandle {
        let service = self.sync_service.clone();
        let parsers = self.parsers.clone();
        self.dispatch_abortable(
            async move {
                let permit = parsers.acquire_owned().await.expect("parser remains open");
                let position = service.reader_position(entry_id).await?;
                let entry = service
                    .cached_entry(entry_id)
                    .await?
                    .ok_or(BrookletError::InvalidSetup("a cached article body"))?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    (
                        crate::reader::parse_document(&entry.html, Some(&entry.url)),
                        position,
                    )
                })
                .await
                .map_err(|error| BrookletError::Storage(std::io::Error::other(error)))
            },
            callback,
        )
    }
    pub fn backend_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }
    pub fn image_decoders(&self) -> Arc<tokio::sync::Semaphore> {
        self.image_decoders.clone()
    }
    pub fn new(
        setup_service: Arc<dyn SetupService>,
        sync_service: Arc<dyn SyncService>,
    ) -> Result<Self, std::io::Error> {
        let image_cache = crate::services::image_cache::ImageCache::new(
            glib::user_cache_dir()
                .join(crate::config::APP_ID)
                .join("images.db"),
            crate::services::image_cache::DEFAULT_IMAGE_CACHE_BYTES,
        )
        .map_err(std::io::Error::other)?;
        Self::with_image_cache(setup_service, sync_service, image_cache)
    }

    /// Inject an isolated cache for offline rendering tests.
    pub fn with_image_cache(
        setup_service: Arc<dyn SetupService>,
        sync_service: Arc<dyn SyncService>,
        image_cache: Arc<crate::services::image_cache::ImageCache>,
    ) -> Result<Self, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("brooklet-backend")
            .enable_all()
            .build()?;
        let (outgoing_changes, changes) = tokio::sync::watch::channel(0);
        let outgoing_service = sync_service.clone();
        runtime.spawn(crate::outgoing::deliver(changes, move || {
            let service = outgoing_service.clone();
            async move { service.flush_outgoing().await }
        }));
        let local_writes =
            crate::outgoing::local_writes(runtime.handle(), outgoing_changes.clone());
        Ok(Self {
            runtime,
            account_operations: Arc::new(tokio::sync::Mutex::new(())),
            local_writes,
            outgoing_changes,
            setup_service,
            sync_service,
            image_cache,
            image_decoders: Arc::new(tokio::sync::Semaphore::new(2)),
            parsers: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    pub fn existing_account(
        &self,
        callback: impl FnOnce(Result<Option<Account>, BrookletError>) + 'static,
    ) {
        let service = self.setup_service.clone();
        self.dispatch(async move { service.existing_account().await }, callback);
    }

    pub fn configure(
        &self,
        request: SetupRequest,
        callback: impl FnOnce(Result<Account, BrookletError>) + 'static,
    ) {
        let service = self.setup_service.clone();
        let changes = self.outgoing_changes.clone();
        self.dispatch_account(
            async move {
                let account = service.configure(request).await?;
                changes.send_modify(|generation| *generation = generation.wrapping_add(1));
                Ok(account)
            },
            callback,
        );
    }

    /// Account changes share a lock with logout. A reconnect that was queued
    /// behind logout must observe the missing account rather than revive it.
    pub fn reconnect(
        &self,
        token: String,
        callback: impl FnOnce(Result<Account, BrookletError>) + 'static,
    ) {
        let service = self.setup_service.clone();
        self.dispatch_account(async move { service.reconnect(token).await }, callback);
    }

    fn dispatch_account<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = Result<T, BrookletError>> + Send + 'static,
        callback: impl FnOnce(Result<T, BrookletError>) + 'static,
    ) {
        let operations = self.account_operations.clone();
        self.dispatch(
            async move {
                let _guard = operations.lock().await;
                future.await
            },
            callback,
        );
    }

    pub fn cached_inbox(&self, callback: impl FnOnce(Result<Vec<Entry>, BrookletError>) + 'static) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.cached_inbox().await }, callback);
    }

    pub fn set_read_local(
        &self,
        entry_id: EntryId,
        read: bool,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_write(
            async move { service.set_read_local(entry_id, read).await },
            callback,
        );
    }

    pub fn sync(&self, callback: impl FnOnce(Result<SyncResult, BrookletError>) + 'static) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.sync().await }, callback);
    }

    pub fn disconnect(&self, callback: impl FnOnce(Result<(), BrookletError>) + 'static) {
        let service = self.sync_service.clone();
        let operations = self.account_operations.clone();
        let cache = self.image_cache.clone();
        self.dispatch_write(
            async move {
                let _guard = operations.lock().await;
                service.disconnect().await?;
                cache.clear().await
            },
            callback,
        );
    }

    pub fn entries_for_view(
        &self,
        view: String,
        callback: impl FnOnce(Result<Vec<Entry>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.entries_for_view(&view).await },
            callback,
        );
    }

    pub fn search_entries(
        &self,
        query: String,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
        callback: impl FnOnce(Result<Vec<Entry>, BrookletError>) + 'static,
    ) -> tokio::task::AbortHandle {
        let service = self.sync_service.clone();
        self.dispatch_abortable(
            async move {
                service
                    .search_entries(&query, feed_id, category_id, read)
                    .await
            },
            callback,
        )
    }

    pub fn categories_cached(
        &self,
        callback: impl FnOnce(Result<Vec<Category>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.categories_cached().await }, callback);
    }

    pub fn feeds_cached(
        &self,
        category_id: Option<i64>,
        callback: impl FnOnce(Result<Vec<Feed>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.feeds_cached(category_id).await },
            callback,
        );
    }

    pub fn set_read_many_local(
        &self,
        entry_ids: Vec<EntryId>,
        read: bool,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_write(
            async move { service.set_read_many_local(&entry_ids, read).await },
            callback,
        );
    }

    pub fn set_starred_local(
        &self,
        entry_id: EntryId,
        starred: bool,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_write(
            async move { service.set_starred_local(entry_id, starred).await },
            callback,
        );
    }

    pub fn reader_position(
        &self,
        entry_id: EntryId,
        callback: impl FnOnce(Result<Option<ReaderPosition>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.reader_position(entry_id).await },
            callback,
        );
    }

    pub fn save_reader_position(
        &self,
        position: ReaderPosition,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_write(
            async move { service.save_reader_position(&position).await },
            callback,
        );
    }

    pub fn queue_karakeep(
        &self,
        entry: Entry,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_write(
            async move { service.queue_karakeep(&entry).await },
            callback,
        );
    }

    pub fn unfinished_karakeep(
        &self,
        callback: impl FnOnce(Result<Vec<KarakeepDelivery>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.unfinished_karakeep().await }, callback);
    }
    pub fn recover_karakeep(
        &self,
        id: i64,
        retry: bool,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_account(
            async move { service.recover_karakeep(id, retry).await },
            callback,
        );
    }

    pub fn karakeep_config(
        &self,
        callback: impl FnOnce(Result<Option<KarakeepConfig>, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.karakeep_config().await }, callback);
    }

    pub fn save_karakeep_config(
        &self,
        config: KarakeepConfig,
        key: Option<String>,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch_account(
            async move { service.save_karakeep_config(&config, key).await },
            callback,
        );
    }

    pub fn storage_policy(
        &self,
        callback: impl FnOnce(Result<StoragePolicy, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.storage_policy().await }, callback);
    }

    pub fn save_storage_policy(
        &self,
        policy: StoragePolicy,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.save_storage_policy(&policy).await },
            callback,
        );
    }

    pub fn sync_status(&self, callback: impl FnOnce(Result<SyncStatus, BrookletError>) + 'static) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.sync_status().await }, callback);
    }

    pub fn refresh_feeds(
        &self,
        callback: impl FnOnce(Result<SyncResult, BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(async move { service.refresh_feeds().await }, callback);
    }

    pub fn subscribe(
        &self,
        feed_url: String,
        category_id: Option<i64>,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let service = self.sync_service.clone();
        self.dispatch(
            async move { service.subscribe(&feed_url, category_id).await },
            callback,
        );
    }

    pub fn fetch_image(
        &self,
        url: String,
        callback: impl FnOnce(Result<Vec<u8>, BrookletError>) + 'static,
    ) -> tokio::task::AbortHandle {
        let cache = self.image_cache.clone();
        self.dispatch_abortable(async move { cache.fetch(&url).await }, callback)
    }

    pub fn invalidate_image(&self, url: String) {
        let cache = self.image_cache.clone();
        self.runtime.spawn(async move {
            let _ = cache.invalidate(url).await;
        });
    }

    pub fn clear_image_cache(&self, callback: impl FnOnce(Result<(), BrookletError>) + 'static) {
        let cache = self.image_cache.clone();
        self.dispatch(async move { cache.clear().await }, callback);
    }

    /// A queue barrier lets shutdown wait for preceding local commits, without
    /// waiting for the network or cancelling an in-flight delivery.
    pub fn drain_local_writes(&self, callback: impl FnOnce(Result<(), BrookletError>) + 'static) {
        self.dispatch_write(async { Ok(()) }, callback);
    }

    fn dispatch_write(
        &self,
        future: impl Future<Output = Result<(), BrookletError>> + Send + 'static,
        callback: impl FnOnce(Result<(), BrookletError>) + 'static,
    ) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.local_writes
            .send((Box::pin(future), sender))
            .expect("local write worker remains alive");
        glib::MainContext::default().spawn_local(async move {
            if let Ok(result) = receiver.await {
                callback(result);
            }
        });
    }

    fn dispatch<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T, BrookletError>> + Send + 'static,
        callback: impl FnOnce(Result<T, BrookletError>) + 'static,
    ) {
        self.dispatch_abortable(future, callback);
    }

    fn dispatch_abortable<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T, BrookletError>> + Send + 'static,
        callback: impl FnOnce(Result<T, BrookletError>) + 'static,
    ) -> tokio::task::AbortHandle {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = self.runtime.spawn(async move {
            let _ = sender.send(future.await);
        });
        glib::MainContext::default().spawn_local(async move {
            if let Ok(result) = receiver.await {
                callback(result);
            }
        });
        task.abort_handle()
    }
}
