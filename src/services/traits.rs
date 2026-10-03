use async_trait::async_trait;

use crate::{
    api::miniflux::{CategoryDto, EntriesDto, EntryDto, EntryQuery, FeedDto, ServerIdentity},
    error::BrookletError,
    model::{
        Account, Category, Entry, EntryId, Feed, KarakeepConfig, KarakeepDelivery, PendingMutation,
        ReaderPosition, StoragePolicy, SyncStatus,
    },
};

#[async_trait]
pub trait MinifluxApi: Send + Sync {
    async fn validate(&self) -> Result<ServerIdentity, BrookletError>;
    async fn categories(&self) -> Result<Vec<CategoryDto>, BrookletError>;
    async fn feeds(&self) -> Result<Vec<FeedDto>, BrookletError>;
    async fn entries(&self, query: &EntryQuery) -> Result<EntriesDto, BrookletError>;
    async fn entry(&self, entry_id: EntryId) -> Result<EntryDto, BrookletError>;
    async fn set_read(&self, entry_ids: &[EntryId], read: bool) -> Result<(), BrookletError>;
    async fn set_starred(&self, entry_ids: &[EntryId], starred: bool) -> Result<(), BrookletError>;
    async fn save_to_integration(&self, entry_id: EntryId) -> Result<(), BrookletError>;
    async fn refresh_feeds(&self) -> Result<(), BrookletError>;
    async fn subscribe(
        &self,
        feed_url: &str,
        category_id: Option<i64>,
    ) -> Result<i64, BrookletError>;
}

#[async_trait]
pub trait KarakeepApi: Send + Sync {
    async fn validate(&self) -> Result<(), BrookletError>;
    async fn save(&self, canonical_url: &str, title: &str) -> Result<(), BrookletError>;
}

#[async_trait]
pub trait Repository: Send + Sync {
    /// Body lookup for deliberate activation. List queries return summaries.
    async fn cached_entry(
        &self,
        account_id: i64,
        entry_id: i64,
    ) -> Result<Option<Entry>, BrookletError> {
        Ok(self
            .unread_entries(account_id)
            .await?
            .into_iter()
            .find(|entry| entry.id == entry_id))
    }

    async fn account(&self) -> Result<Option<Account>, BrookletError>;
    async fn save_account(&self, account: &Account) -> Result<(), BrookletError>;
    async fn delete_account(&self, account_id: i64) -> Result<(), BrookletError>;
    async fn replace_unread_snapshot(
        &self,
        account_id: i64,
        entries: &[Entry],
    ) -> Result<(), BrookletError>;
    async fn unread_entries(&self, account_id: i64) -> Result<Vec<Entry>, BrookletError>;
    async fn set_read_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        read: bool,
    ) -> Result<(), BrookletError>;
    async fn pending_mutations(
        &self,
        account_id: i64,
    ) -> Result<Vec<PendingMutation>, BrookletError>;
    async fn acknowledge_mutation(&self, mutation: &PendingMutation) -> Result<(), BrookletError>;
    async fn entries_for_view(
        &self,
        account_id: i64,
        view: &str,
    ) -> Result<Vec<Entry>, BrookletError> {
        let _ = (account_id, view);
        Ok(Vec::new())
    }
    async fn search_entries(
        &self,
        account_id: i64,
        query: &str,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
    ) -> Result<Vec<Entry>, BrookletError> {
        let _ = (account_id, query, feed_id, category_id, read);
        Ok(Vec::new())
    }
    async fn categories_cached(&self, account_id: i64) -> Result<Vec<Category>, BrookletError> {
        let _ = account_id;
        Ok(Vec::new())
    }
    async fn feeds_cached(
        &self,
        account_id: i64,
        category_id: Option<i64>,
    ) -> Result<Vec<Feed>, BrookletError> {
        let _ = (account_id, category_id);
        Ok(Vec::new())
    }
    async fn merge_metadata(
        &self,
        account_id: i64,
        categories: &[Category],
        feeds: &[Feed],
    ) -> Result<(), BrookletError> {
        let _ = (account_id, categories, feeds);
        Ok(())
    }
    async fn sync_cursor(&self, account_id: i64) -> Result<Option<i64>, BrookletError> {
        let _ = account_id;
        Ok(None)
    }
    async fn merge_changed_page(
        &self,
        account_id: i64,
        entries: &[Entry],
        removed_ids: &[EntryId],
    ) -> Result<(), BrookletError> {
        let _ = removed_ids;
        self.replace_unread_snapshot(account_id, entries).await
    }
    async fn complete_sync(
        &self,
        account_id: i64,
        cursor: i64,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, cursor, now_ms);
        Ok(())
    }
    async fn record_sync_error(
        &self,
        account_id: i64,
        error: &str,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, error, now_ms);
        Ok(())
    }
    /// Delivery success clears only delivery failures, preserving refresh errors.
    async fn record_delivery_error(
        &self,
        account_id: i64,
        error: Option<&str>,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, error);
        Ok(())
    }
    async fn sync_status(&self, account_id: i64) -> Result<SyncStatus, BrookletError> {
        let _ = account_id;
        Ok(SyncStatus::default())
    }
    async fn set_starred_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        starred: bool,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, entry_id, starred);
        Ok(())
    }
    async fn set_read_many_local(
        &self,
        account_id: i64,
        entry_ids: &[EntryId],
        read: bool,
    ) -> Result<(), BrookletError> {
        for entry_id in entry_ids {
            self.set_read_local(account_id, *entry_id, read).await?;
        }
        Ok(())
    }
    async fn save_reader_position(
        &self,
        account_id: i64,
        position: &ReaderPosition,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, position);
        Ok(())
    }
    async fn queue_karakeep(&self, delivery: &KarakeepDelivery) -> Result<(), BrookletError> {
        let _ = delivery;
        Ok(())
    }
    async fn unfinished_karakeep(
        &self,
        account_id: i64,
    ) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        self.pending_karakeep(account_id).await
    }
    async fn recover_karakeep(
        &self,
        account_id: i64,
        delivery_id: i64,
        route: Option<crate::model::KarakeepRoute>,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, delivery_id, route);
        Err(BrookletError::InvalidSetup("delivery recovery support"))
    }
    async fn pending_karakeep(
        &self,
        account_id: i64,
    ) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        let _ = account_id;
        Ok(Vec::new())
    }
    async fn finish_karakeep(
        &self,
        delivery_id: i64,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let _ = (delivery_id, error, now_ms);
        Ok(())
    }
    async fn defer_karakeep(&self, delivery_id: i64, error: &str) -> Result<(), BrookletError> {
        let _ = (delivery_id, error);
        Ok(())
    }
    async fn karakeep_config(
        &self,
        account_id: i64,
    ) -> Result<Option<KarakeepConfig>, BrookletError> {
        let _ = account_id;
        Ok(None)
    }
    async fn save_karakeep_config(
        &self,
        account_id: i64,
        config: &KarakeepConfig,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, config);
        Ok(())
    }
    async fn storage_policy(&self, account_id: i64) -> Result<StoragePolicy, BrookletError> {
        let _ = account_id;
        Ok(StoragePolicy::default())
    }
    async fn save_storage_policy(
        &self,
        account_id: i64,
        policy: &StoragePolicy,
    ) -> Result<(), BrookletError> {
        let _ = (account_id, policy);
        Ok(())
    }
    async fn apply_retention(&self, account_id: i64, now_ms: i64) -> Result<usize, BrookletError> {
        let _ = (account_id, now_ms);
        Ok(0)
    }
    async fn reader_position(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<ReaderPosition>, BrookletError>;
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn load_miniflux_token(&self, account_id: i64) -> Result<Option<String>, BrookletError>;
    async fn store_miniflux_token(&self, account_id: i64, token: &str)
    -> Result<(), BrookletError>;
    async fn delete_account_secrets(&self, account_id: i64) -> Result<(), BrookletError>;
    async fn load_karakeep_key(&self, _account_id: i64) -> Result<Option<String>, BrookletError> {
        Ok(None)
    }
    async fn delete_karakeep_key(&self, _account_id: i64) -> Result<(), BrookletError> {
        Ok(())
    }
    async fn store_karakeep_key(&self, _account_id: i64, _key: &str) -> Result<(), BrookletError> {
        Ok(())
    }
}

pub trait Clock: Send + Sync {
    fn now_epoch_millis(&self) -> i64;
}
