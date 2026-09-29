//! Bounded persistent article images. All SQLite work runs off the UI thread.
use crate::{
    error::BrookletError,
    services::url_policy::{
        MAX_ARTICLE_IMAGE_BYTES, fetch_article_image, image_client, validate_article_image_url,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

pub const DEFAULT_IMAGE_CACHE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IMAGE_CACHE_ENTRIES: u64 = 8192;

pub struct ImageCache {
    path: PathBuf,
    budget: u64,
    connection: Mutex<Option<Connection>>,
    keys: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    downloads: tokio::sync::Semaphore,
    generation: AtomicU64,
    client: reqwest::Client,
}

impl ImageCache {
    pub fn new(path: PathBuf, budget: u64) -> Result<Arc<Self>, BrookletError> {
        Ok(Arc::new(Self {
            path,
            budget,
            connection: Mutex::new(None),
            keys: Mutex::new(HashMap::new()),
            downloads: tokio::sync::Semaphore::new(4),
            generation: AtomicU64::new(0),
            client: image_client()?,
        }))
    }

    pub async fn fetch(self: &Arc<Self>, url: &str) -> Result<Vec<u8>, BrookletError> {
        self.get_or_fetch(url, || fetch_article_image(&self.client, url))
            .await
    }

    pub async fn get_or_fetch<F, Fut>(
        self: &Arc<Self>,
        url: &str,
        fetch: F,
    ) -> Result<Vec<u8>, BrookletError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, BrookletError>>,
    {
        let key = validate_article_image_url(url)?.to_string();
        let generation = self.generation.load(Ordering::Acquire);
        let lock = {
            let mut keys = self.keys.lock().expect("image key lock");
            keys.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = keys.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                keys.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _key_guard = lock.lock().await;
        let lookup = key.clone();
        match self.database(move |db| {
            let bytes: Option<Vec<u8>> = db.query_row("SELECT bytes FROM images WHERE url=?1 AND length(bytes)<=?2", params![lookup, MAX_ARTICLE_IMAGE_BYTES], |row| row.get(0)).optional()?;
            if bytes.is_some() { db.execute("UPDATE images SET accessed=(SELECT COALESCE(MAX(accessed),0)+1 FROM images) WHERE url=?1", [&lookup])?; }
            Ok(bytes)
        }).await {
            Ok(Some(bytes)) if bytes.len() <= MAX_ARTICLE_IMAGE_BYTES => return Ok(bytes),
            Ok(_) => {},
            Err(error) => tracing::warn!(%error, "image cache unavailable; fetching image"),
        }
        let _download = self
            .downloads
            .acquire()
            .await
            .expect("image downloads remain open");
        let bytes = fetch().await?;
        if bytes.is_empty() || bytes.len() > MAX_ARTICLE_IMAGE_BYTES {
            return Err(BrookletError::InvalidServiceUrl(
                "article image size is invalid".into(),
            ));
        }
        if bytes.len() as u64 <= self.budget {
            let stored = bytes.clone();
            let cache = self.clone();
            if let Err(error) = self.database(move |db| {
                if cache.generation.load(Ordering::Acquire) != generation { return Ok(()); }
                let transaction = db.transaction()?;
                transaction.execute("INSERT OR REPLACE INTO images(url,bytes,accessed) VALUES(?1,?2,(SELECT COALESCE(MAX(accessed),0)+1 FROM images))", params![key, stored])?;
                let (mut total, mut count): (u64, u64) = transaction.query_row("SELECT COALESCE(SUM(length(bytes)),0),COUNT(*) FROM images", [], |row| Ok((row.get(0)?,row.get(1)?)))?;
                while total > cache.budget || count > MAX_IMAGE_CACHE_ENTRIES {
                    let (oldest, size): (String,u64) = transaction.query_row("SELECT url,length(bytes) FROM images ORDER BY accessed,url LIMIT 1", [], |row| Ok((row.get(0)?,row.get(1)?)))?;
                    transaction.execute("DELETE FROM images WHERE url=?1", [oldest])?; total -= size; count -= 1;
                }
                transaction.commit()?;
                db.execute_batch("PRAGMA incremental_vacuum(256)")?;
                Ok(())
            }).await { tracing::warn!(%error, "image cache write failed"); }
        }
        Ok(bytes)
    }

    pub async fn invalidate(self: &Arc<Self>, url: String) -> Result<(), BrookletError> {
        let url = validate_article_image_url(&url)?.to_string();
        self.database(move |db| {
            db.execute("DELETE FROM images WHERE url=?1", [url])?;
            Ok(())
        })
        .await
    }

    pub async fn clear(self: &Arc<Self>) -> Result<(), BrookletError> {
        let cache = self.clone();
        self.database(move |db| {
            cache.generation.fetch_add(1, Ordering::AcqRel);
            db.execute_batch("DELETE FROM images; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
        .await
    }

    async fn database<T, F>(self: &Arc<Self>, operation: F) -> Result<T, BrookletError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = cache.connection.lock().expect("image database lock");
            if connection.is_none() {
                if let Some(parent) = cache.path.parent() { std::fs::create_dir_all(parent).map_err(BrookletError::Storage)?; }
                let db = Connection::open(&cache.path).map_err(BrookletError::Database)?;
                db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; CREATE TABLE IF NOT EXISTS images(url TEXT PRIMARY KEY, bytes BLOB NOT NULL, accessed INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS images_lru ON images(accessed);").map_err(BrookletError::Database)?;
                *connection = Some(db);
            }
            operation(connection.as_mut().expect("image database opened")).map_err(BrookletError::Database)
        }).await.map_err(|error| BrookletError::Storage(std::io::Error::other(error)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cache(path: &std::path::Path, budget: u64) -> Arc<ImageCache> {
        ImageCache::new(path.join("images.db"), budget).unwrap()
    }
    async fn put(cache: &Arc<ImageCache>, name: &str, value: u8) -> Vec<u8> {
        cache
            .get_or_fetch(&format!("https://example.com/{name}"), || async {
                Ok(vec![value; 4])
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn cache_survives_restart_and_reads_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let first = cache(dir.path(), 32);
        put(&first, "one", 1).await;
        drop(first);
        let reopened = cache(dir.path(), 32);
        assert_eq!(
            reopened
                .get_or_fetch("https://example.com/one", || async {
                    panic!("offline cache hit must not fetch")
                })
                .await
                .unwrap(),
            vec![1; 4]
        );
    }
    #[tokio::test]
    async fn lru_eviction_respects_recent_reads_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 8);
        put(&cache, "one", 1).await;
        put(&cache, "two", 2).await;
        put(&cache, "one", 9).await;
        put(&cache, "three", 3).await;
        assert_eq!(put(&cache, "one", 9).await, vec![1; 4]);
        assert_eq!(put(&cache, "two", 9).await, vec![9; 4]);
        cache.clear().await.unwrap();
        assert_eq!(put(&cache, "one", 7).await, vec![7; 4]);
    }
    #[tokio::test]
    async fn duplicate_requests_share_one_fetch_and_failures_are_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 32);
        let count = AtomicU64::new(0);
        let request = || {
            cache.get_or_fetch("https://example.com/one", || async {
                count.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
                Ok(vec![1; 4])
            })
        };
        let (a, b) = tokio::join!(request(), request());
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(
            cache
                .get_or_fetch("https://example.com/fail", || async {
                    Err(BrookletError::InvalidSetup("failure"))
                })
                .await
                .is_err()
        );
        assert_eq!(put(&cache, "fail", 2).await, vec![2; 4]);
        assert!(
            cache
                .get_or_fetch("http://127.0.0.1/image", || async {
                    panic!("unsafe image must not fetch")
                })
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn cancellation_releases_download_and_key_locks() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 32);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch("https://example.com/one", || async {
                        started_tx.send(()).unwrap();
                        std::future::pending::<Result<Vec<u8>, BrookletError>>().await
                    })
                    .await
            })
        };
        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), put(&cache, "one", 4))
                .await
                .unwrap(),
            vec![4; 4]
        );
    }

    #[tokio::test]
    async fn over_budget_images_are_displayed_without_being_retained() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 3);
        assert_eq!(put(&cache, "one", 1).await, vec![1; 4]);
        assert_eq!(put(&cache, "one", 2).await, vec![2; 4]);
        let empty = cache
            .get_or_fetch("https://example.com/empty", || async { Ok(Vec::new()) })
            .await;
        assert!(empty.is_err());
    }

    #[tokio::test]
    async fn clear_during_download_prevents_repopulation() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path(), 32);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let task = {
            let cache = cache.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch("https://example.com/one", || async {
                        started_tx.send(()).unwrap();
                        finish_rx.await.unwrap();
                        Ok(vec![1; 4])
                    })
                    .await
                    .unwrap()
            })
        };
        started_rx.await.unwrap();
        cache.clear().await.unwrap();
        finish_tx.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(put(&cache, "one", 3).await, vec![3; 4]);
    }
}
