use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    error::BrookletError,
    model::{
        Account, Category, DeliveryState, Entry, EntryId, Feed, KarakeepConfig, KarakeepDelivery,
        KarakeepRoute, MutationField, PendingMutation, ReaderPosition, StoragePolicy, SyncStatus,
    },
    services::traits::Repository,
};

const MIGRATIONS: &[&str] = &[
    r#"
        CREATE TABLE accounts (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            server_url      TEXT NOT NULL,
            username        TEXT NOT NULL,
            server_version  TEXT NOT NULL,
            created_at_ms   INTEGER NOT NULL DEFAULT 0
        );
    "#,
    r#"
        CREATE TABLE entries (
            account_id       INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            id               INTEGER NOT NULL,
            feed_id          INTEGER NOT NULL,
            feed_title       TEXT NOT NULL,
            category_title   TEXT NOT NULL,
            title            TEXT NOT NULL,
            url              TEXT NOT NULL,
            author           TEXT,
            published_at_ms  INTEGER NOT NULL,
            html             TEXT NOT NULL,
            read             INTEGER NOT NULL CHECK (read IN (0, 1)),
            starred          INTEGER NOT NULL CHECK (starred IN (0, 1)),
            reading_minutes  INTEGER NOT NULL,
            PRIMARY KEY (account_id, id)
        );
        CREATE INDEX entries_inbox
            ON entries (account_id, read, published_at_ms DESC);
    "#,
    r#"
        CREATE TABLE pending_mutations (
            account_id    INTEGER NOT NULL,
            entry_id      INTEGER NOT NULL,
            field         TEXT NOT NULL CHECK (field IN ('read', 'starred')),
            desired       INTEGER NOT NULL CHECK (desired IN (0, 1)),
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY (account_id, entry_id, field),
            FOREIGN KEY (account_id, entry_id)
                REFERENCES entries(account_id, id) ON DELETE CASCADE
        );
    "#,
    r#"
        ALTER TABLE entries ADD COLUMN changed_at_ms INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE entries ADD COLUMN last_opened_at_ms INTEGER;
        CREATE TABLE categories (
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            id INTEGER NOT NULL,
            title TEXT NOT NULL,
            PRIMARY KEY(account_id, id)
        );
        CREATE TABLE feeds (
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            id INTEGER NOT NULL,
            category_id INTEGER NOT NULL,
            title TEXT NOT NULL,
            site_url TEXT NOT NULL,
            feed_url TEXT NOT NULL,
            PRIMARY KEY(account_id, id)
        );
        CREATE TABLE reader_positions (
            account_id INTEGER NOT NULL,
            entry_id INTEGER NOT NULL,
            block_index INTEGER NOT NULL,
            offset_px INTEGER NOT NULL,
            PRIMARY KEY(account_id, entry_id),
            FOREIGN KEY(account_id, entry_id) REFERENCES entries(account_id, id) ON DELETE CASCADE
        );
        CREATE TABLE sync_state (
            account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
            cursor_seconds INTEGER,
            last_success_ms INTEGER,
            last_error TEXT
        );
        CREATE TABLE karakeep_config (
            account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
            route TEXT NOT NULL CHECK(route IN ('miniflux', 'direct')),
            direct_endpoint TEXT
        );
        CREATE TABLE karakeep_deliveries (
            id INTEGER PRIMARY KEY,
            account_id INTEGER NOT NULL,
            entry_id INTEGER NOT NULL,
            canonical_url TEXT NOT NULL,
            title TEXT NOT NULL,
            route TEXT NOT NULL CHECK(route IN ('miniflux', 'direct')),
            state TEXT NOT NULL CHECK(state IN ('queued', 'saved', 'needs_attention')),
            last_error TEXT,
            completed_at_ms INTEGER,
            UNIQUE(account_id, canonical_url),
            FOREIGN KEY(account_id, entry_id) REFERENCES entries(account_id, id) ON DELETE CASCADE
        );
        CREATE TABLE storage_policy (
            account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
            retain_read_days INTEGER,
            keep_at_most INTEGER NOT NULL DEFAULT 5000
        );
        CREATE INDEX entries_changed ON entries(account_id, changed_at_ms DESC);
        CREATE INDEX entries_saved ON entries(account_id, starred, published_at_ms DESC);
    "#,
    "ALTER TABLE entries ADD COLUMN content_revision INTEGER NOT NULL DEFAULT 0; CREATE INDEX entries_delivery ON karakeep_deliveries(account_id, entry_id); CREATE INDEX entries_order ON entries(account_id, published_at_ms DESC, id DESC);",
    "ALTER TABLE sync_state ADD COLUMN delivery_error TEXT;",
    "ALTER TABLE entries ADD COLUMN remote_removed INTEGER NOT NULL DEFAULT 0;",
];

pub struct SqliteRepository {
    store: Arc<SqliteStore>,
    access: Arc<tokio::sync::Semaphore>,
}

struct SqliteStore {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BrookletError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::prepare(connection, path.to_path_buf())
    }

    pub fn open_in_memory() -> Result<Self, BrookletError> {
        Self::prepare(Connection::open_in_memory()?, PathBuf::from(":memory:"))
    }

    fn prepare(mut connection: Connection, path: PathBuf) -> Result<Self, BrookletError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        if path != Path::new(":memory:") {
            connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        migrate(&mut connection)?;
        Ok(Self {
            store: Arc::new(SqliteStore {
                connection: Mutex::new(connection),
                path,
            }),
            access: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.store.path
    }

    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&SqliteStore) -> Result<T, BrookletError> + Send + 'static,
    ) -> Result<T, BrookletError> {
        // Acquire before spawning: only one blocking task can use this database,
        // rather than occupying the pool with threads waiting on its mutex.
        let permit = self
            .access
            .clone()
            .acquire_owned()
            .await
            .expect("database remains open");
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(&store)
        })
        .await
        .map_err(|error| BrookletError::Storage(std::io::Error::other(error)))?
    }
}

fn migrate(connection: &mut Connection) -> Result<(), rusqlite::Error> {
    let current: usize = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", index + 1)?;
        transaction.commit()?;
    }
    Ok(())
}

const ENTRY_SELECT: &str = r#"SELECT e.id, e.account_id, e.feed_id, e.feed_title,
    e.category_title, e.title, e.url, e.author, e.published_at_ms, e.html,
    e.read, e.starred, e.reading_minutes, k.state, k.last_error, e.content_revision
    FROM entries e
    LEFT JOIN karakeep_deliveries k ON k.account_id=e.account_id AND k.entry_id=e.id"#;

fn summary_select() -> String {
    ENTRY_SELECT.replace("e.html,", "'' AS html,")
}

fn content_revision(html: &str) -> i64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    html.hash(&mut hash);
    (hash.finish() & i64::MAX as u64) as i64
}

fn read_entry(row: &rusqlite::Row<'_>) -> Result<Entry, rusqlite::Error> {
    let state: Option<String> = row.get(13)?;
    Ok(Entry {
        id: row.get(0)?,
        account_id: row.get(1)?,
        feed_id: row.get(2)?,
        feed_title: row.get(3)?,
        category_title: row.get(4)?,
        title: row.get(5)?,
        url: row.get(6)?,
        author: row.get(7)?,
        published_at_ms: row.get(8)?,
        html: row.get(9)?,
        content_revision: row.get(15)?,
        read: row.get(10)?,
        starred: row.get(11)?,
        reading_minutes: row.get(12)?,
        delivery_state: state.as_deref().map(|value| match value {
            "saved" => DeliveryState::Saved,
            "needs_attention" => DeliveryState::NeedsAttention,
            _ => DeliveryState::Queued,
        }),
        delivery_error: row.get(14)?,
    })
}

impl SqliteStore {
    fn cached_entry(&self, account_id: i64, entry_id: i64) -> Result<Option<Entry>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection
            .query_row(
                &format!("{ENTRY_SELECT} WHERE e.account_id=?1 AND e.id=?2"),
                params![account_id, entry_id],
                read_entry,
            )
            .optional()
            .map_err(Into::into)
    }

    fn account(&self) -> Result<Option<Account>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection
            .query_row(
                "SELECT id, server_url, username, server_version FROM accounts WHERE id = 1",
                [],
                |row| {
                    Ok(Account {
                        id: row.get(0)?,
                        server_url: row.get(1)?,
                        username: row.get(2)?,
                        server_version: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn save_account(&self, account: &Account) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute(
            r#"INSERT INTO accounts (id, server_url, username, server_version, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, unixepoch('subsec') * 1000)
               ON CONFLICT(id) DO UPDATE SET
                   server_url = excluded.server_url,
                   username = excluded.username,
                   server_version = excluded.server_version"#,
            params![
                account.id,
                account.server_url,
                account.username,
                account.server_version
            ],
        )?;
        Ok(())
    }

    fn delete_account(&self, account_id: i64) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.pragma_update(None, "secure_delete", "ON")?;
        connection.execute("DELETE FROM accounts WHERE id = ?1", [account_id])?;
        // Account-owned records cascade through the database. The checkpoint
        // also discards old pages from the WAL after their logical deletion.
        if self.path != Path::new(":memory:") {
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        }
        Ok(())
    }

    fn replace_unread_snapshot(
        &self,
        account_id: i64,
        entries: &[Entry],
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            r#"UPDATE entries SET read = 1
               WHERE account_id = ?1 AND read = 0
                 AND NOT EXISTS (
                     SELECT 1 FROM pending_mutations AS pending
                     WHERE pending.account_id = entries.account_id
                       AND pending.entry_id = entries.id
                       AND pending.field = 'read'
                 )"#,
            [account_id],
        )?;
        {
            let mut statement = transaction.prepare(
                r#"INSERT INTO entries (
                       account_id, id, feed_id, feed_title, category_title, title, url,
                       author, published_at_ms, html, read, starred, reading_minutes, content_revision
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                   ON CONFLICT(account_id, id) DO UPDATE SET
                       feed_id = excluded.feed_id,
                       feed_title = excluded.feed_title,
                       category_title = excluded.category_title,
                       title = excluded.title,
                       url = excluded.url,
                       author = excluded.author,
                       published_at_ms = excluded.published_at_ms,
                       html = excluded.html, content_revision = excluded.content_revision,
                       read = CASE WHEN EXISTS (
                           SELECT 1 FROM pending_mutations AS pending
                           WHERE pending.account_id = entries.account_id
                             AND pending.entry_id = entries.id
                             AND pending.field = 'read'
                       ) THEN entries.read ELSE excluded.read END,
                       starred = CASE WHEN EXISTS (
                           SELECT 1 FROM pending_mutations AS pending
                           WHERE pending.account_id = entries.account_id
                             AND pending.entry_id = entries.id
                             AND pending.field = 'starred'
                       ) THEN entries.starred ELSE excluded.starred END,
                       reading_minutes = excluded.reading_minutes"#,
            )?;
            for entry in entries {
                statement.execute(params![
                    entry.account_id,
                    entry.id,
                    entry.feed_id,
                    entry.feed_title,
                    entry.category_title,
                    entry.title,
                    entry.url,
                    entry.author,
                    entry.published_at_ms,
                    entry.html,
                    entry.read,
                    entry.starred,
                    entry.reading_minutes,
                    content_revision(&entry.html),
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn unread_entries(&self, account_id: i64) -> Result<Vec<Entry>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let sql = format!(
            "{} WHERE e.account_id=?1 AND e.read=0 ORDER BY e.published_at_ms DESC, e.id DESC",
            summary_select()
        );
        let mut statement = connection.prepare(&sql)?;
        statement
            .query_map([account_id], read_entry)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn entries_for_view(&self, account_id: i64, view: &str) -> Result<Vec<Entry>, BrookletError> {
        let (filter, feed_id) = match view {
            "inbox" | "unread" => ("e.read=0", None),
            "saved" => ("e.starred=1", None),
            "read" => ("e.read=1", None),
            "all" => ("1=1", None),
            _ => match view
                .strip_prefix("feed:")
                .and_then(|value| value.parse::<i64>().ok())
            {
                Some(id) => ("e.feed_id=?2", Some(id)),
                None => return Ok(Vec::new()),
            },
        };
        let sql = format!(
            "{} WHERE e.account_id=?1 AND {filter} ORDER BY e.published_at_ms DESC,e.id DESC",
            summary_select()
        );
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection.prepare(&sql)?;
        let entries = if let Some(feed_id) = feed_id {
            statement
                .query_map(params![account_id, feed_id], read_entry)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([account_id], read_entry)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(entries)
    }

    fn search_entries(
        &self,
        account_id: i64,
        query: &str,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
    ) -> Result<Vec<Entry>, BrookletError> {
        let pattern = format!(
            "%{}%",
            query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let sql = format!(
            "{} LEFT JOIN feeds f ON f.account_id=e.account_id AND f.id=e.feed_id WHERE e.account_id=?1 AND (?2='' OR e.title LIKE ?3 ESCAPE '\\' OR e.feed_title LIKE ?3 ESCAPE '\\' OR COALESCE(e.author,'') LIKE ?3 ESCAPE '\\' OR e.html LIKE ?3 ESCAPE '\\') AND (?4 IS NULL OR e.feed_id=?4) AND (?5 IS NULL OR f.category_id=?5) AND (?6 IS NULL OR e.read=?6) ORDER BY e.published_at_ms DESC LIMIT 500",
            summary_select()
        );
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection.prepare(&sql)?;
        statement
            .query_map(
                params![
                    account_id,
                    query.trim(),
                    pattern,
                    feed_id,
                    category_id,
                    read
                ],
                read_entry,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn set_read_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        read: bool,
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        let updated = transaction.execute(
            "UPDATE entries SET read = ?3, last_opened_at_ms = CASE WHEN ?3 THEN unixepoch('subsec') * 1000 ELSE last_opened_at_ms END WHERE account_id = ?1 AND id = ?2",
            params![account_id, entry_id, read],
        )?;
        if updated > 0 {
            transaction.execute(
                r#"INSERT INTO pending_mutations
                   (account_id, entry_id, field, desired, updated_at_ms)
               VALUES (?1, ?2, 'read', ?3, unixepoch('subsec') * 1000)
               ON CONFLICT(account_id, entry_id, field) DO UPDATE SET
                   desired = excluded.desired,
                   updated_at_ms = excluded.updated_at_ms"#,
                params![account_id, entry_id, read],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn pending_mutations(&self, account_id: i64) -> Result<Vec<PendingMutation>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection.prepare(
            r#"SELECT account_id, entry_id, field, desired
               FROM pending_mutations
               WHERE account_id = ?1
               ORDER BY updated_at_ms, entry_id, field"#,
        )?;
        statement
            .query_map([account_id], |row| {
                let field: String = row.get(2)?;
                Ok(PendingMutation {
                    account_id: row.get(0)?,
                    entry_id: row.get(1)?,
                    field: if field == "read" {
                        MutationField::Read
                    } else {
                        MutationField::Starred
                    },
                    desired: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn acknowledge_mutation(&self, mutation: &PendingMutation) -> Result<(), BrookletError> {
        let field = match mutation.field {
            MutationField::Read => "read",
            MutationField::Starred => "starred",
        };
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute(
            r#"DELETE FROM pending_mutations
               WHERE account_id = ?1 AND entry_id = ?2 AND field = ?3 AND desired = ?4"#,
            params![
                mutation.account_id,
                mutation.entry_id,
                field,
                mutation.desired
            ],
        )?;
        Ok(())
    }

    fn set_starred_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        starred: bool,
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        let updated = transaction.execute(
            "UPDATE entries SET starred=?3 WHERE account_id=?1 AND id=?2",
            params![account_id, entry_id, starred],
        )?;
        if updated > 0 {
            transaction.execute("INSERT INTO pending_mutations(account_id,entry_id,field,desired,updated_at_ms) VALUES(?1,?2,'starred',?3,unixepoch('subsec')*1000) ON CONFLICT(account_id,entry_id,field) DO UPDATE SET desired=excluded.desired,updated_at_ms=excluded.updated_at_ms",params![account_id,entry_id,starred])?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn set_read_many_local(
        &self,
        account_id: i64,
        entry_ids: &[EntryId],
        read: bool,
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        {
            let mut update = transaction.prepare("UPDATE entries SET read=?3,last_opened_at_ms=CASE WHEN ?3 THEN unixepoch('subsec')*1000 ELSE last_opened_at_ms END WHERE account_id=?1 AND id=?2")?;
            let mut pending = transaction.prepare("INSERT INTO pending_mutations(account_id,entry_id,field,desired,updated_at_ms) VALUES(?1,?2,'read',?3,unixepoch('subsec')*1000) ON CONFLICT(account_id,entry_id,field) DO UPDATE SET desired=excluded.desired,updated_at_ms=excluded.updated_at_ms")?;
            for id in entry_ids {
                if update.execute(params![account_id, id, read])? > 0 {
                    pending.execute(params![account_id, id, read])?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn categories_cached(&self, account_id: i64) -> Result<Vec<Category>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection
            .prepare("SELECT id,title FROM categories WHERE account_id=?1 ORDER BY title")?;
        statement
            .query_map([account_id], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    title: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn feeds_cached(
        &self,
        account_id: i64,
        category_id: Option<i64>,
    ) -> Result<Vec<Feed>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection.prepare("SELECT id,category_id,title,site_url,feed_url FROM feeds WHERE account_id=?1 AND (?2 IS NULL OR category_id=?2) ORDER BY title")?;
        statement
            .query_map(params![account_id, category_id], |row| {
                Ok(Feed {
                    id: row.get(0)?,
                    category_id: row.get(1)?,
                    title: row.get(2)?,
                    site_url: row.get(3)?,
                    feed_url: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn merge_metadata(
        &self,
        account_id: i64,
        categories: &[Category],
        feeds: &[Feed],
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM feeds WHERE account_id=?1", [account_id])?;
        transaction.execute("DELETE FROM categories WHERE account_id=?1", [account_id])?;
        {
            let mut category_stmt = transaction
                .prepare("INSERT INTO categories(account_id,id,title) VALUES(?1,?2,?3)")?;
            for category in categories {
                category_stmt.execute(params![account_id, category.id, category.title])?;
            }
            let mut feed_stmt = transaction.prepare("INSERT INTO feeds(account_id,id,category_id,title,site_url,feed_url) VALUES(?1,?2,?3,?4,?5,?6)")?;
            for feed in feeds {
                feed_stmt.execute(params![
                    account_id,
                    feed.id,
                    feed.category_id,
                    feed.title,
                    feed.site_url,
                    feed.feed_url
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn sync_cursor(&self, account_id: i64) -> Result<Option<i64>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection
            .query_row(
                "SELECT cursor_seconds FROM sync_state WHERE account_id=?1",
                [account_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(Into::into)
    }

    fn merge_changed_page(
        &self,
        account_id: i64,
        entries: &[Entry],
        removed_ids: &[EntryId],
    ) -> Result<(), BrookletError> {
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        {
            let mut remove = transaction.prepare("DELETE FROM entries WHERE account_id=?1 AND id=?2 AND NOT EXISTS(SELECT 1 FROM pending_mutations m WHERE m.account_id=entries.account_id AND m.entry_id=entries.id) AND NOT EXISTS(SELECT 1 FROM karakeep_deliveries k WHERE k.account_id=entries.account_id AND k.entry_id=entries.id AND k.state!='saved')")?;
            for id in removed_ids {
                transaction.execute(
                    "UPDATE entries SET remote_removed=1 WHERE account_id=?1 AND id=?2",
                    params![account_id, id],
                )?;
                remove.execute(params![account_id, id])?;
            }
            let mut merge = transaction.prepare("INSERT INTO entries(account_id,id,feed_id,feed_title,category_title,title,url,author,published_at_ms,html,read,starred,reading_minutes,content_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(account_id,id) DO UPDATE SET remote_removed=0,feed_id=excluded.feed_id,feed_title=excluded.feed_title,category_title=excluded.category_title,title=excluded.title,url=excluded.url,author=excluded.author,published_at_ms=excluded.published_at_ms,html=excluded.html,content_revision=excluded.content_revision,read=CASE WHEN EXISTS(SELECT 1 FROM pending_mutations m WHERE m.account_id=entries.account_id AND m.entry_id=entries.id AND m.field='read') THEN entries.read ELSE excluded.read END,starred=CASE WHEN EXISTS(SELECT 1 FROM pending_mutations m WHERE m.account_id=entries.account_id AND m.entry_id=entries.id AND m.field='starred') THEN entries.starred ELSE excluded.starred END,reading_minutes=excluded.reading_minutes")?;
            for entry in entries {
                merge.execute(params![
                    entry.account_id,
                    entry.id,
                    entry.feed_id,
                    entry.feed_title,
                    entry.category_title,
                    entry.title,
                    entry.url,
                    entry.author,
                    entry.published_at_ms,
                    entry.html,
                    entry.read,
                    entry.starred,
                    entry.reading_minutes,
                    content_revision(&entry.html)
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn complete_sync(
        &self,
        account_id: i64,
        cursor: i64,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO sync_state(account_id,cursor_seconds,last_success_ms,last_error) VALUES(?1,?2,?3,NULL) ON CONFLICT(account_id) DO UPDATE SET cursor_seconds=excluded.cursor_seconds,last_success_ms=excluded.last_success_ms,last_error=NULL",params![account_id,cursor,now_ms])?;
        Ok(())
    }

    fn record_sync_error(
        &self,
        account_id: i64,
        error: &str,
        _now_ms: i64,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO sync_state(account_id,last_error) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET last_error=excluded.last_error",params![account_id,error])?;
        Ok(())
    }

    fn record_delivery_error(
        &self,
        account_id: i64,
        error: Option<&str>,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute(
            "INSERT INTO sync_state(account_id,delivery_error) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET delivery_error=excluded.delivery_error",
            params![account_id, error],
        )?;
        Ok(())
    }

    fn sync_status(&self, account_id: i64) -> Result<SyncStatus, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let (last_successful_sync_at_ms, refresh_error, delivery_error) = connection
            .query_row(
                "SELECT last_success_ms,last_error,delivery_error FROM sync_state WHERE account_id=?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
            )
            .optional()?
            .unwrap_or((None, None, None));
        let error = match (&refresh_error, &delivery_error) {
            (Some(refresh), Some(delivery)) => {
                Some(format!("Delivery: {delivery}; Refresh: {refresh}"))
            }
            _ => delivery_error.clone().or_else(|| refresh_error.clone()),
        };
        let (article_changes, queued_karakeep): (usize, usize) = connection.query_row(
            "SELECT (SELECT count(*) FROM pending_mutations WHERE account_id=?1), (SELECT count(*) FROM karakeep_deliveries WHERE account_id=?1 AND state!='saved')",
            [account_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(SyncStatus {
            running: false,
            queued_mutations: article_changes + queued_karakeep,
            queued_karakeep,
            last_successful_sync_at_ms,
            refresh_error,
            delivery_error,
            error,
        })
    }

    fn reader_position(&self, entry_id: EntryId) -> Result<Option<ReaderPosition>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection
            .query_row(
                "SELECT block_index,offset_px FROM reader_positions WHERE entry_id=?1 LIMIT 1",
                [entry_id],
                |row| {
                    Ok(ReaderPosition {
                        entry_id,
                        first_visible_block: row.get(0)?,
                        offset_px: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn save_reader_position(
        &self,
        account_id: i64,
        position: &ReaderPosition,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO reader_positions(account_id,entry_id,block_index,offset_px) VALUES(?1,?2,?3,?4) ON CONFLICT(account_id,entry_id) DO UPDATE SET block_index=excluded.block_index,offset_px=excluded.offset_px",params![account_id,position.entry_id,position.first_visible_block,position.offset_px])?;
        Ok(())
    }

    fn queue_karakeep(&self, delivery: &KarakeepDelivery) -> Result<(), BrookletError> {
        let route = if delivery.route == KarakeepRoute::Direct {
            "direct"
        } else {
            "miniflux"
        };
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO karakeep_deliveries(account_id,entry_id,canonical_url,title,route,state) VALUES(?1,?2,?3,?4,?5,'queued') ON CONFLICT(account_id,canonical_url) DO UPDATE SET entry_id=excluded.entry_id,title=excluded.title,route=excluded.route,state=CASE WHEN karakeep_deliveries.state='saved' THEN 'saved' ELSE 'queued' END,last_error=NULL",params![delivery.account_id,delivery.entry_id,delivery.canonical_url,delivery.title,route])?;
        Ok(())
    }

    fn pending_karakeep(&self, account_id: i64) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        self.karakeep_deliveries(account_id, true)
    }

    fn karakeep_deliveries(
        &self,
        account_id: i64,
        queued_only: bool,
    ) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        let mut statement = connection.prepare("SELECT id,account_id,entry_id,canonical_url,title,route,state,last_error FROM karakeep_deliveries WHERE account_id=?1 AND state!='saved' AND (?2=0 OR state='queued') ORDER BY id")?;
        statement
            .query_map(params![account_id, queued_only], |row| {
                let route: String = row.get(5)?;
                let state: String = row.get(6)?;
                Ok(KarakeepDelivery {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    entry_id: row.get(2)?,
                    canonical_url: row.get(3)?,
                    title: row.get(4)?,
                    route: if route == "direct" {
                        KarakeepRoute::Direct
                    } else {
                        KarakeepRoute::Miniflux
                    },
                    state: if state == "needs_attention" {
                        DeliveryState::NeedsAttention
                    } else {
                        DeliveryState::Queued
                    },
                    error: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn recover_karakeep(
        &self,
        account_id: i64,
        delivery_id: i64,
        route: Option<KarakeepRoute>,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        if let Some(route) = route {
            connection.execute("UPDATE karakeep_deliveries SET state='queued',route=?3,last_error=NULL,completed_at_ms=NULL WHERE account_id=?1 AND id=?2 AND state!='saved'", params![account_id,delivery_id,if route == KarakeepRoute::Direct { "direct" } else { "miniflux" }])?;
        } else {
            connection.execute(
                "DELETE FROM karakeep_deliveries WHERE account_id=?1 AND id=?2 AND state!='saved'",
                params![account_id, delivery_id],
            )?;
        }
        Ok(())
    }

    fn finish_karakeep(
        &self,
        delivery_id: i64,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("UPDATE karakeep_deliveries SET state=CASE WHEN ?2 IS NULL THEN 'saved' ELSE 'needs_attention' END,last_error=?2,completed_at_ms=CASE WHEN ?2 IS NULL THEN ?3 ELSE NULL END WHERE id=?1",params![delivery_id,error,now_ms])?;
        Ok(())
    }

    fn defer_karakeep(&self, delivery_id: i64, error: &str) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute(
            "UPDATE karakeep_deliveries SET state='queued',last_error=?2 WHERE id=?1",
            params![delivery_id, error],
        )?;
        Ok(())
    }

    fn karakeep_config(&self, account_id: i64) -> Result<Option<KarakeepConfig>, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection
            .query_row(
                "SELECT route,direct_endpoint FROM karakeep_config WHERE account_id=?1",
                [account_id],
                |row| {
                    let route: String = row.get(0)?;
                    Ok(KarakeepConfig {
                        route: if route == "direct" {
                            KarakeepRoute::Direct
                        } else {
                            KarakeepRoute::Miniflux
                        },
                        direct_endpoint: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn save_karakeep_config(
        &self,
        account_id: i64,
        config: &KarakeepConfig,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO karakeep_config(account_id,route,direct_endpoint) VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET route=excluded.route,direct_endpoint=excluded.direct_endpoint",params![account_id,if config.route==KarakeepRoute::Direct {"direct"} else {"miniflux"},config.direct_endpoint])?;
        Ok(())
    }

    fn storage_policy(&self, account_id: i64) -> Result<StoragePolicy, BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        Ok(connection
            .query_row(
                "SELECT retain_read_days,keep_at_most FROM storage_policy WHERE account_id=?1",
                [account_id],
                |row| {
                    Ok(StoragePolicy {
                        retain_read_days: row.get(0)?,
                        keep_at_most: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    fn save_storage_policy(
        &self,
        account_id: i64,
        policy: &StoragePolicy,
    ) -> Result<(), BrookletError> {
        let connection = self.connection.lock().expect("SQLite mutex poisoned");
        connection.execute("INSERT INTO storage_policy(account_id,retain_read_days,keep_at_most) VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET retain_read_days=excluded.retain_read_days,keep_at_most=excluded.keep_at_most",params![account_id,policy.retain_read_days,policy.keep_at_most])?;
        Ok(())
    }

    fn apply_retention(&self, account_id: i64, now_ms: i64) -> Result<usize, BrookletError> {
        let policy = self.storage_policy(account_id)?;
        let mut connection = self.connection.lock().expect("SQLite mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM karakeep_deliveries WHERE account_id=?1 AND state='saved' AND completed_at_ms<?2",params![account_id,now_ms-30*86_400_000])?;
        let reconciled = transaction.execute("DELETE FROM entries WHERE account_id=?1 AND remote_removed=1 AND NOT EXISTS(SELECT 1 FROM pending_mutations m WHERE m.account_id=entries.account_id AND m.entry_id=entries.id) AND NOT EXISTS(SELECT 1 FROM karakeep_deliveries k WHERE k.account_id=entries.account_id AND k.entry_id=entries.id AND k.state!='saved')", [account_id])?;
        let Some(days) = policy.retain_read_days else {
            transaction.commit()?;
            return Ok(reconciled);
        };
        let cutoff = now_ms.saturating_sub(i64::from(days) * 86_400_000);
        let eligible = "e.account_id=?1 AND e.read=1 AND e.starred=0 AND (e.last_opened_at_ms IS NULL OR e.last_opened_at_ms<?2) AND NOT EXISTS(SELECT 1 FROM pending_mutations m WHERE m.account_id=e.account_id AND m.entry_id=e.id) AND NOT EXISTS(SELECT 1 FROM karakeep_deliveries k WHERE k.account_id=e.account_id AND k.entry_id=e.id)";
        let expired_sql = format!(
            "DELETE FROM entries WHERE account_id=?1 AND id IN (SELECT e.id FROM entries e WHERE {eligible} AND e.published_at_ms<?2)"
        );
        let expired = transaction.execute(&expired_sql, params![account_id, cutoff])?;
        let overflow_sql = format!(
            "DELETE FROM entries WHERE account_id=?1 AND id IN (SELECT e.id FROM entries e WHERE {eligible} ORDER BY e.published_at_ms DESC, e.id DESC LIMIT -1 OFFSET ?3)"
        );
        let overflow = transaction.execute(
            &overflow_sql,
            params![account_id, cutoff, policy.keep_at_most],
        )?;
        transaction.commit()?;
        let removed = reconciled + expired + overflow;
        Ok(removed)
    }
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn cached_entry(
        &self,
        account_id: i64,
        entry_id: i64,
    ) -> Result<Option<Entry>, BrookletError> {
        self.run(move |store| store.cached_entry(account_id, entry_id))
            .await
    }

    async fn cached_entry_ids(&self, account_id: i64) -> Result<Vec<EntryId>, BrookletError> {
        self.run(move |store| {
            let connection = store.connection.lock().expect("SQLite mutex poisoned");
            let mut statement =
                connection.prepare("SELECT id FROM entries WHERE account_id=?1 ORDER BY id")?;
            Ok(statement
                .query_map([account_id], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    async fn account(&self) -> Result<Option<Account>, BrookletError> {
        self.run(move |store| store.account()).await
    }

    async fn save_account(&self, account: &Account) -> Result<(), BrookletError> {
        let account = account.clone();
        self.run(move |store| store.save_account(&account)).await
    }

    async fn delete_account(&self, account_id: i64) -> Result<(), BrookletError> {
        self.run(move |store| store.delete_account(account_id))
            .await
    }

    async fn replace_unread_snapshot(
        &self,
        account_id: i64,
        entries: &[Entry],
    ) -> Result<(), BrookletError> {
        let entries = entries.to_vec();
        self.run(move |store| store.replace_unread_snapshot(account_id, &entries))
            .await
    }

    async fn unread_entries(&self, account_id: i64) -> Result<Vec<Entry>, BrookletError> {
        self.run(move |store| store.unread_entries(account_id))
            .await
    }

    async fn entries_for_view(
        &self,
        account_id: i64,
        view: &str,
    ) -> Result<Vec<Entry>, BrookletError> {
        let view = view.to_owned();
        self.run(move |store| store.entries_for_view(account_id, &view))
            .await
    }

    async fn search_entries(
        &self,
        account_id: i64,
        query: &str,
        feed_id: Option<i64>,
        category_id: Option<i64>,
        read: Option<bool>,
    ) -> Result<Vec<Entry>, BrookletError> {
        let query = query.to_owned();
        self.run(move |store| store.search_entries(account_id, &query, feed_id, category_id, read))
            .await
    }

    async fn set_read_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        read: bool,
    ) -> Result<(), BrookletError> {
        self.run(move |store| store.set_read_local(account_id, entry_id, read))
            .await
    }

    async fn pending_mutations(
        &self,
        account_id: i64,
    ) -> Result<Vec<PendingMutation>, BrookletError> {
        self.run(move |store| store.pending_mutations(account_id))
            .await
    }

    async fn acknowledge_mutation(&self, mutation: &PendingMutation) -> Result<(), BrookletError> {
        let mutation = mutation.clone();
        self.run(move |store| store.acknowledge_mutation(&mutation))
            .await
    }

    async fn set_starred_local(
        &self,
        account_id: i64,
        entry_id: EntryId,
        starred: bool,
    ) -> Result<(), BrookletError> {
        self.run(move |store| store.set_starred_local(account_id, entry_id, starred))
            .await
    }

    async fn set_read_many_local(
        &self,
        account_id: i64,
        entry_ids: &[EntryId],
        read: bool,
    ) -> Result<(), BrookletError> {
        let entry_ids = entry_ids.to_vec();
        self.run(move |store| store.set_read_many_local(account_id, &entry_ids, read))
            .await
    }

    async fn categories_cached(&self, account_id: i64) -> Result<Vec<Category>, BrookletError> {
        self.run(move |store| store.categories_cached(account_id))
            .await
    }

    async fn feeds_cached(
        &self,
        account_id: i64,
        category_id: Option<i64>,
    ) -> Result<Vec<Feed>, BrookletError> {
        self.run(move |store| store.feeds_cached(account_id, category_id))
            .await
    }

    async fn merge_metadata(
        &self,
        account_id: i64,
        categories: &[Category],
        feeds: &[Feed],
    ) -> Result<(), BrookletError> {
        let categories = categories.to_vec();
        let feeds = feeds.to_vec();
        self.run(move |store| store.merge_metadata(account_id, &categories, &feeds))
            .await
    }

    async fn sync_cursor(&self, account_id: i64) -> Result<Option<i64>, BrookletError> {
        self.run(move |store| store.sync_cursor(account_id)).await
    }

    async fn merge_changed_page(
        &self,
        account_id: i64,
        entries: &[Entry],
        removed_ids: &[EntryId],
    ) -> Result<(), BrookletError> {
        let entries = entries.to_vec();
        let removed_ids = removed_ids.to_vec();
        self.run(move |store| store.merge_changed_page(account_id, &entries, &removed_ids))
            .await
    }

    async fn complete_sync(
        &self,
        account_id: i64,
        cursor: i64,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        self.run(move |store| store.complete_sync(account_id, cursor, now_ms))
            .await
    }

    async fn record_sync_error(
        &self,
        account_id: i64,
        error: &str,
        _now_ms: i64,
    ) -> Result<(), BrookletError> {
        let error = error.to_owned();
        self.run(move |store| store.record_sync_error(account_id, &error, _now_ms))
            .await
    }

    async fn sync_status(&self, account_id: i64) -> Result<SyncStatus, BrookletError> {
        self.run(move |store| store.sync_status(account_id)).await
    }

    async fn record_delivery_error(
        &self,
        account_id: i64,
        error: Option<&str>,
    ) -> Result<(), BrookletError> {
        let error = error.map(str::to_owned);
        self.run(move |store| store.record_delivery_error(account_id, error.as_deref()))
            .await
    }

    async fn reader_position(
        &self,
        entry_id: EntryId,
    ) -> Result<Option<ReaderPosition>, BrookletError> {
        self.run(move |store| store.reader_position(entry_id)).await
    }

    async fn save_reader_position(
        &self,
        account_id: i64,
        position: &ReaderPosition,
    ) -> Result<(), BrookletError> {
        let position = position.clone();
        self.run(move |store| store.save_reader_position(account_id, &position))
            .await
    }

    async fn queue_karakeep(&self, delivery: &KarakeepDelivery) -> Result<(), BrookletError> {
        let delivery = delivery.clone();
        self.run(move |store| store.queue_karakeep(&delivery)).await
    }

    async fn pending_karakeep(
        &self,
        account_id: i64,
    ) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        self.run(move |store| store.pending_karakeep(account_id))
            .await
    }

    async fn unfinished_karakeep(
        &self,
        account_id: i64,
    ) -> Result<Vec<KarakeepDelivery>, BrookletError> {
        self.run(move |store| store.karakeep_deliveries(account_id, false))
            .await
    }
    async fn recover_karakeep(
        &self,
        account_id: i64,
        delivery_id: i64,
        route: Option<KarakeepRoute>,
    ) -> Result<(), BrookletError> {
        self.run(move |store| store.recover_karakeep(account_id, delivery_id, route))
            .await
    }

    async fn finish_karakeep(
        &self,
        delivery_id: i64,
        error: Option<&str>,
        now_ms: i64,
    ) -> Result<(), BrookletError> {
        let error = error.map(str::to_owned);
        self.run(move |store| store.finish_karakeep(delivery_id, error.as_deref(), now_ms))
            .await
    }

    async fn defer_karakeep(&self, delivery_id: i64, error: &str) -> Result<(), BrookletError> {
        let error = error.to_owned();
        self.run(move |store| store.defer_karakeep(delivery_id, &error))
            .await
    }

    async fn karakeep_config(
        &self,
        account_id: i64,
    ) -> Result<Option<KarakeepConfig>, BrookletError> {
        self.run(move |store| store.karakeep_config(account_id))
            .await
    }

    async fn save_karakeep_config(
        &self,
        account_id: i64,
        config: &KarakeepConfig,
    ) -> Result<(), BrookletError> {
        let config = config.clone();
        self.run(move |store| store.save_karakeep_config(account_id, &config))
            .await
    }

    async fn storage_policy(&self, account_id: i64) -> Result<StoragePolicy, BrookletError> {
        self.run(move |store| store.storage_policy(account_id))
            .await
    }

    async fn save_storage_policy(
        &self,
        account_id: i64,
        policy: &StoragePolicy,
    ) -> Result<(), BrookletError> {
        let policy = policy.clone();
        self.run(move |store| store.save_storage_policy(account_id, &policy))
            .await
    }

    async fn apply_retention(&self, account_id: i64, now_ms: i64) -> Result<usize, BrookletError> {
        self.run(move |store| store.apply_retention(account_id, now_ms))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn summaries_omit_bodies_but_preserve_content_change_detection() {
        let repository = repository_with_entry().await;
        let before = repository.unread_entries(1).await.unwrap().remove(0);
        assert!(before.html.is_empty());
        let body = repository.cached_entry(1, 42).await.unwrap().unwrap();
        assert_eq!(body.html, "<p>Story</p>");
        assert!(repository.cached_entry(2, 42).await.unwrap().is_none());
        let changed = Entry {
            html: "<p>Changed body</p>".into(),
            ..body
        };
        repository
            .merge_changed_page(1, &[changed], &[])
            .await
            .unwrap();
        let after = repository
            .entries_for_view(1, "all")
            .await
            .unwrap()
            .remove(0);
        assert!(after.html.is_empty());
        assert_ne!(before.content_revision, after.content_revision);
        assert!(
            repository
                .search_entries(1, "Changed body", None, None, None)
                .await
                .unwrap()[0]
                .html
                .is_empty()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn database_work_does_not_block_the_async_executor() {
        let repository = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let (started, ready) = tokio::sync::oneshot::channel();
        let work = tokio::spawn(async move {
            repository
                .run(move |_| {
                    started.send(()).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    Ok(())
                })
                .await
                .unwrap();
        });
        ready.await.unwrap();
        // A synchronous operation on this single-thread executor would finish
        // before it could deliver the notification and run this continuation.
        assert!(!work.is_finished());
        tokio::task::yield_now().await;
        assert!(!work.is_finished());
        work.await.unwrap();
    }

    #[tokio::test]
    async fn migration_and_account_round_trip_exclude_credentials() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        let account = Account {
            id: 1,
            server_url: "https://miniflux.example".into(),
            username: "reader".into(),
            server_version: "2.3.2".into(),
        };
        repository.save_account(&account).await.unwrap();
        assert_eq!(repository.account().await.unwrap(), Some(account));

        let connection = repository.store.connection.lock().unwrap();
        let mut statement = connection.prepare("PRAGMA table_info(accounts)").unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|name| {
            name.contains("token") || name.contains("secret") || name.contains("credential")
        }));
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as u32);
    }

    #[tokio::test]
    async fn unread_snapshot_is_atomic_and_marks_missing_entries_read() {
        let repository = SqliteRepository::open_in_memory().unwrap();
        repository
            .save_account(&Account {
                id: 1,
                server_url: "https://miniflux.example".into(),
                username: "reader".into(),
                server_version: "2.3.2".into(),
            })
            .await
            .unwrap();
        let entry = |id, title: &str| Entry {
            id,
            account_id: 1,
            feed_id: 9,
            feed_title: "Example feed".into(),
            category_title: "News".into(),
            title: title.into(),
            url: format!("https://example.com/{id}"),
            author: None,
            published_at_ms: id,
            html: "<p>Story</p>".into(),
            content_revision: 0,
            read: false,
            starred: false,
            reading_minutes: 2,
            delivery_state: None,
            delivery_error: None,
        };

        repository
            .replace_unread_snapshot(1, &[entry(1, "Old"), entry(2, "Keep")])
            .await
            .unwrap();
        repository
            .replace_unread_snapshot(1, &[entry(2, "Updated")])
            .await
            .unwrap();

        let unread = repository.unread_entries(1).await.unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].id, 2);
        assert_eq!(unread[0].title, "Updated");
    }

    #[tokio::test]
    async fn local_read_changes_are_atomic_and_coalesce() {
        let repository = repository_with_entry().await;

        repository.set_read_local(1, 42, true).await.unwrap();
        assert!(repository.unread_entries(1).await.unwrap().is_empty());
        repository.set_read_local(1, 42, false).await.unwrap();

        let pending = repository.pending_mutations(1).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].field, MutationField::Read);
        assert!(!pending[0].desired);
        assert_eq!(repository.unread_entries(1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stale_remote_state_does_not_overwrite_pending_local_intent() {
        let repository = repository_with_entry().await;
        repository.set_read_local(1, 42, true).await.unwrap();

        let mut stale = example_entry();
        stale.read = false;
        repository
            .replace_unread_snapshot(1, &[stale])
            .await
            .unwrap();

        assert!(repository.unread_entries(1).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn acknowledgement_cannot_delete_a_newer_opposite_intention() {
        let repository = repository_with_entry().await;
        repository.set_read_local(1, 42, true).await.unwrap();
        let sent = repository.pending_mutations(1).await.unwrap().remove(0);
        repository.set_read_local(1, 42, false).await.unwrap();

        repository.acknowledge_mutation(&sent).await.unwrap();

        let pending = repository.pending_mutations(1).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].desired);
    }

    #[tokio::test]
    async fn error_before_first_sync_has_no_cursor_and_delivery_success_preserves_refresh_error() {
        let repository = repository_with_entry().await;
        repository
            .record_sync_error(1, "refresh failed", 0)
            .await
            .unwrap();
        assert_eq!(repository.sync_cursor(1).await.unwrap(), None);
        repository
            .record_delivery_error(1, Some("delivery failed"))
            .await
            .unwrap();
        let error = repository.sync_status(1).await.unwrap().error.unwrap();
        assert!(error.contains("delivery failed"));
        assert!(error.contains("refresh failed"));
        repository.record_delivery_error(1, None).await.unwrap();
        assert_eq!(
            repository.sync_status(1).await.unwrap().error.as_deref(),
            Some("refresh failed")
        );
        repository.complete_sync(1, 100, 10).await.unwrap();
        assert_eq!(repository.sync_cursor(1).await.unwrap(), Some(100));
        assert_eq!(repository.sync_status(1).await.unwrap().error, None);
    }

    #[tokio::test]
    async fn delivery_status_migration_preserves_existing_account_entries_and_queue() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.db");
        {
            let connection = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..MIGRATIONS.len() - 1] {
                connection.execute_batch(sql).unwrap();
            }
            connection
                .pragma_update(None, "user_version", MIGRATIONS.len() - 1)
                .unwrap();
            connection.execute("INSERT INTO accounts(id,server_url,username,server_version) VALUES(1,'https://miniflux.example','reader','2.3.2')", []).unwrap();
            connection.execute("INSERT INTO entries(account_id,id,feed_id,feed_title,category_title,title,url,published_at_ms,html,read,starred,reading_minutes) VALUES(1,42,7,'Feed','News','Story','https://example.com/42',0,'<p>Story</p>',1,0,1)", []).unwrap();
            connection.execute("INSERT INTO pending_mutations(account_id,entry_id,field,desired,updated_at_ms) VALUES(1,42,'read',1,1234)", []).unwrap();
        }
        let repository = SqliteRepository::open(&path).unwrap();
        assert_eq!(
            repository.account().await.unwrap().unwrap().username,
            "reader"
        );
        assert!(repository.cached_entry(1, 42).await.unwrap().unwrap().read);
        let pending = repository.pending_mutations(1).await.unwrap().remove(0);
        assert!(pending.desired);
        repository
            .record_delivery_error(1, Some("offline"))
            .await
            .unwrap();
        assert_eq!(repository.sync_cursor(1).await.unwrap(), None);
        assert_eq!(
            repository.sync_status(1).await.unwrap().error.as_deref(),
            Some("offline")
        );
        repository.acknowledge_mutation(&pending).await.unwrap();
        assert!(repository.pending_mutations(1).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn changed_merge_preserves_local_star_and_protects_removed_pending_entries() {
        let repository = repository_with_entry().await;
        repository.set_starred_local(1, 42, true).await.unwrap();
        let mut stale = example_entry();
        stale.title = "Updated remotely".into();
        repository
            .merge_changed_page(1, &[stale], &[])
            .await
            .unwrap();
        let cached = repository.entries_for_view(1, "all").await.unwrap();
        assert!(cached[0].starred);
        assert_eq!(cached[0].title, "Updated remotely");

        repository.merge_changed_page(1, &[], &[42]).await.unwrap();
        assert_eq!(
            repository.entries_for_view(1, "all").await.unwrap().len(),
            1
        );
        let sent = repository.pending_mutations(1).await.unwrap().remove(0);
        repository.acknowledge_mutation(&sent).await.unwrap();
        repository.merge_changed_page(1, &[], &[42]).await.unwrap();
        assert!(
            repository
                .entries_for_view(1, "all")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn karakeep_canonical_url_coalesces_and_protects_removed_entries() {
        let repository = repository_with_entry().await;
        let delivery = KarakeepDelivery {
            id: 0,
            account_id: 1,
            entry_id: 42,
            canonical_url: "https://example.com/42".into(),
            title: "Story".into(),
            route: KarakeepRoute::Miniflux,
            state: DeliveryState::Queued,
            error: None,
        };
        repository.queue_karakeep(&delivery).await.unwrap();
        repository.queue_karakeep(&delivery).await.unwrap();
        assert_eq!(repository.pending_karakeep(1).await.unwrap().len(), 1);
        repository.merge_changed_page(1, &[], &[42]).await.unwrap();
        assert_eq!(
            repository.entries_for_view(1, "all").await.unwrap().len(),
            1
        );
        let receipt = repository.pending_karakeep(1).await.unwrap().remove(0);
        repository
            .finish_karakeep(receipt.id, None, 0)
            .await
            .unwrap();
        assert!(repository.pending_karakeep(1).await.unwrap().is_empty());
        repository
            .save_storage_policy(
                1,
                &StoragePolicy {
                    retain_read_days: None,
                    keep_at_most: 5_000,
                },
            )
            .await
            .unwrap();
        repository
            .apply_retention(1, 31 * 86_400_000)
            .await
            .unwrap();
        assert!(
            repository
                .entries_for_view(1, "all")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn karakeep_retry_stays_queued_but_terminal_failure_needs_attention() {
        let repository = repository_with_entry().await;
        let delivery = KarakeepDelivery {
            id: 0,
            account_id: 1,
            entry_id: 42,
            canonical_url: "https://example.com/42".into(),
            title: "Story".into(),
            route: KarakeepRoute::Direct,
            state: DeliveryState::Queued,
            error: None,
        };
        repository.queue_karakeep(&delivery).await.unwrap();
        let id = repository.pending_karakeep(1).await.unwrap()[0].id;
        repository.defer_karakeep(id, "offline").await.unwrap();
        assert_eq!(
            repository.pending_karakeep(1).await.unwrap()[0]
                .error
                .as_deref(),
            Some("offline")
        );
        repository
            .finish_karakeep(id, Some("bad request"), 0)
            .await
            .unwrap();
        assert!(repository.pending_karakeep(1).await.unwrap().is_empty());
        assert_eq!(
            repository.entries_for_view(1, "all").await.unwrap()[0].delivery_state,
            Some(DeliveryState::NeedsAttention)
        );
        repository.queue_karakeep(&delivery).await.unwrap();
        assert_eq!(repository.pending_karakeep(1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reader_position_search_and_retention_are_local() {
        let repository = repository_with_entry().await;
        let position = ReaderPosition {
            entry_id: 42,
            first_visible_block: 3,
            offset_px: 16,
        };
        repository.save_reader_position(1, &position).await.unwrap();
        assert_eq!(
            repository.reader_position(42).await.unwrap(),
            Some(position)
        );
        assert_eq!(
            repository
                .search_entries(1, "Story", None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
        repository
            .save_storage_policy(
                1,
                &StoragePolicy {
                    retain_read_days: Some(30),
                    keep_at_most: 0,
                },
            )
            .await
            .unwrap();
        repository.set_read_local(1, 42, true).await.unwrap();
        let sent = repository.pending_mutations(1).await.unwrap().remove(0);
        assert_eq!(
            repository
                .apply_retention(1, 60 * 86_400_000)
                .await
                .unwrap(),
            0
        );
        repository.acknowledge_mutation(&sent).await.unwrap();
        // Opening the article protects it even though publication is old.
        assert_eq!(
            repository
                .apply_retention(1, 60 * 86_400_000)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn retention_applies_age_and_count_independently() {
        let repository = repository_with_entry().await;
        let day = 86_400_000;
        let now = 100 * day;
        let make = |id, published_at_ms, read, starred| Entry {
            id,
            published_at_ms,
            read,
            starred,
            ..example_entry()
        };
        repository
            .merge_changed_page(
                1,
                &[
                    make(1, day, true, false),
                    make(2, 98 * day, true, false),
                    make(3, 99 * day, true, false),
                    make(4, day, true, true),
                    make(5, day, false, false),
                ],
                &[],
            )
            .await
            .unwrap();
        repository
            .save_storage_policy(
                1,
                &StoragePolicy {
                    retain_read_days: Some(30),
                    keep_at_most: 1,
                },
            )
            .await
            .unwrap();

        assert_eq!(repository.apply_retention(1, now).await.unwrap(), 2);
        let ids = repository
            .entries_for_view(1, "all")
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![3, 5, 4, 42]);
    }

    #[tokio::test]
    async fn metadata_refresh_removes_feeds_and_categories_missing_from_server() {
        let repository = repository_with_entry().await;
        let category = Category {
            id: 7,
            title: "News".into(),
        };
        let feed = Feed {
            id: 9,
            category_id: 7,
            title: "Example".into(),
            site_url: "https://example.com".into(),
            feed_url: "https://example.com/feed".into(),
        };
        repository
            .merge_metadata(1, &[category], &[feed])
            .await
            .unwrap();
        assert_eq!(repository.categories_cached(1).await.unwrap().len(), 1);
        assert_eq!(repository.feeds_cached(1, None).await.unwrap().len(), 1);

        repository.merge_metadata(1, &[], &[]).await.unwrap();
        assert!(repository.categories_cached(1).await.unwrap().is_empty());
        assert!(repository.feeds_cached(1, None).await.unwrap().is_empty());
        assert_eq!(
            repository.entries_for_view(1, "all").await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn search_treats_wildcards_and_backslashes_as_literal_text() {
        let repository = repository_with_entry().await;
        let entry = Entry {
            id: 43,
            title: "100%_\\done".into(),
            ..example_entry()
        };
        repository
            .merge_changed_page(1, &[entry], &[])
            .await
            .unwrap();
        for query in ["%", "_", "\\"] {
            let found = repository
                .search_entries(1, query, None, None, None)
                .await
                .unwrap();
            assert_eq!(found.len(), 1, "query {query}");
            assert_eq!(found[0].id, 43);
        }
    }

    fn example_entry() -> Entry {
        Entry {
            id: 42,
            account_id: 1,
            feed_id: 9,
            feed_title: "Example feed".into(),
            category_title: "News".into(),
            title: "Story".into(),
            url: "https://example.com/42".into(),
            author: None,
            published_at_ms: 42,
            html: "<p>Story</p>".into(),
            content_revision: 0,
            read: false,
            starred: false,
            reading_minutes: 2,
            delivery_state: None,
            delivery_error: None,
        }
    }

    async fn repository_with_entry() -> SqliteRepository {
        let repository = SqliteRepository::open_in_memory().unwrap();
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
            .replace_unread_snapshot(1, &[example_entry()])
            .await
            .unwrap();
        repository
    }

    #[test]
    fn file_database_enables_foreign_keys_and_wal() {
        let directory = tempfile::tempdir().unwrap();
        let repository = SqliteRepository::open(directory.path().join("brooklet.db")).unwrap();
        let connection = repository.store.connection.lock().unwrap();
        let foreign_keys: u32 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        let journal: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
        assert_eq!(journal.to_ascii_lowercase(), "wal");
    }

    #[tokio::test]
    async fn deleting_account_clears_cached_and_pending_data() {
        let repository = repository_with_entry().await;
        repository.set_read_local(1, 42, true).await.unwrap();
        repository
            .save_reader_position(
                1,
                &ReaderPosition {
                    entry_id: 42,
                    first_visible_block: 2,
                    offset_px: 18,
                },
            )
            .await
            .unwrap();
        {
            let connection = repository.store.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO categories (account_id, id, title) VALUES (1, 7, 'News')",
                    [],
                )
                .unwrap();
            connection
                .execute("INSERT INTO feeds (account_id, id, category_id, title, site_url, feed_url) VALUES (1, 8, 7, 'Feed', '', '')", [])
                .unwrap();
        }

        repository.delete_account(1).await.unwrap();

        assert!(repository.account().await.unwrap().is_none());
        let connection = repository.store.connection.lock().unwrap();
        for table in [
            "entries",
            "pending_mutations",
            "reader_positions",
            "categories",
            "feeds",
            "sync_state",
            "karakeep_config",
            "karakeep_deliveries",
            "storage_policy",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} still contains account data");
        }
    }

    #[tokio::test]
    async fn offline_intention_survives_database_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brooklet.db");
        {
            let repository = SqliteRepository::open(&path).unwrap();
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
                .merge_changed_page(1, &[example_entry()], &[])
                .await
                .unwrap();
            repository.set_read_local(1, 42, true).await.unwrap();
        }
        let reopened = SqliteRepository::open(&path).unwrap();
        let pending = reopened.pending_mutations(1).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entry_id, 42);
        assert!(pending[0].desired);
        assert!(reopened.unread_entries(1).await.unwrap().is_empty());
    }
}
