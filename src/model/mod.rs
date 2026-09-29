use serde::{Deserialize, Serialize};

pub type AccountId = i64;
pub type EntryId = i64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub server_url: String,
    pub username: String,
    pub server_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    pub id: i64,
    pub category_id: i64,
    pub title: String,
    pub site_url: String,
    pub feed_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: EntryId,
    pub account_id: AccountId,
    pub feed_id: i64,
    pub feed_title: String,
    pub category_title: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub published_at_ms: i64,
    pub html: String,
    /// Opaque content fingerprint; summaries retain it without retaining HTML.
    #[serde(default)]
    pub content_revision: i64,
    pub read: bool,
    pub starred: bool,
    pub reading_minutes: u32,
    pub delivery_state: Option<DeliveryState>,
    pub delivery_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Superscript(Vec<Inline>),
    Subscript(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link { text: Vec<Inline>, url: String },
    Break,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCellLayout {
    pub column: u32,
    pub row_span: u32,
    pub column_span: u32,
    pub header: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentBlock {
    Heading {
        level: u8,
        text: String,
        inline: Vec<Inline>,
    },
    Paragraph {
        text: String,
        inline: Vec<Inline>,
    },
    Quote {
        text: String,
        inline: Vec<Inline>,
    },
    Code {
        text: String,
    },
    ListItem {
        text: String,
        inline: Vec<Inline>,
        ordered: bool,
        ordinal: Option<i64>,
        #[serde(default)]
        depth: u32,
    },
    Caption {
        text: String,
        inline: Vec<Inline>,
    },
    Table {
        rows: Vec<Vec<String>>,
        #[serde(default)]
        inline_rows: Vec<Vec<Vec<Inline>>>,
        #[serde(default)]
        cell_layout: Vec<Vec<TableCellLayout>>,
    },
    Image {
        url: String,
        description: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationField {
    Read,
    Starred,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMutation {
    pub account_id: AccountId,
    pub entry_id: EntryId,
    pub field: MutationField,
    pub desired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    Sending,
    Saved,
    NeedsAttention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KarakeepRoute {
    Miniflux,
    Direct,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReaderPosition {
    pub entry_id: EntryId,
    pub first_visible_block: usize,
    pub offset_px: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KarakeepDelivery {
    pub id: i64,
    pub account_id: AccountId,
    pub entry_id: EntryId,
    pub canonical_url: String,
    pub title: String,
    pub route: KarakeepRoute,
    pub state: DeliveryState,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KarakeepConfig {
    pub route: KarakeepRoute,
    pub direct_endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoragePolicy {
    pub retain_read_days: Option<u32>,
    pub keep_at_most: usize,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            retain_read_days: Some(30),
            keep_at_most: 5_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub running: bool,
    pub queued_mutations: usize,
    pub last_successful_sync_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Retryable,
    Authentication,
    UnsupportedServer,
    Certificate,
    MalformedRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionCandidate {
    pub read: bool,
    pub starred: bool,
    pub pending_mutation: bool,
    pub pending_karakeep: bool,
    pub recently_opened: bool,
    pub published_at_ms: i64,
}

impl RetentionCandidate {
    pub fn can_prune(&self, cutoff_ms: i64) -> bool {
        self.read
            && !self.starred
            && !self.pending_mutation
            && !self.pending_karakeep
            && !self.recently_opened
            && self.published_at_ms < cutoff_ms
    }
}

pub fn incremental_start(last_changed_at_seconds: Option<i64>, overlap_seconds: i64) -> i64 {
    (last_changed_at_seconds.unwrap_or(0) - overlap_seconds).max(0)
}

pub fn classify_http_status(status: u16) -> FailureKind {
    match status {
        401 | 403 => FailureKind::Authentication,
        408 | 429 | 500..=599 => FailureKind::Retryable,
        400 | 404 | 422 => FailureKind::MalformedRequest,
        _ => FailureKind::MalformedRequest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_overlap_is_sixty_seconds_and_never_negative() {
        assert_eq!(incremental_start(Some(1_000), 60), 940);
        assert_eq!(incremental_start(Some(30), 60), 0);
        assert_eq!(incremental_start(None, 60), 0);
    }

    #[test]
    fn retention_protects_valuable_or_pending_entries() {
        let ordinary = RetentionCandidate {
            read: true,
            starred: false,
            pending_mutation: false,
            pending_karakeep: false,
            recently_opened: false,
            published_at_ms: 1,
        };
        assert!(ordinary.can_prune(10));
        for protected in [
            RetentionCandidate {
                read: false,
                ..ordinary.clone()
            },
            RetentionCandidate {
                starred: true,
                ..ordinary.clone()
            },
            RetentionCandidate {
                pending_mutation: true,
                ..ordinary.clone()
            },
            RetentionCandidate {
                pending_karakeep: true,
                ..ordinary.clone()
            },
            RetentionCandidate {
                recently_opened: true,
                ..ordinary.clone()
            },
        ] {
            assert!(!protected.can_prune(10));
        }
    }

    #[test]
    fn retry_classification_matches_the_oracle() {
        for status in [408, 429, 500, 503] {
            assert_eq!(classify_http_status(status), FailureKind::Retryable);
        }
        assert_eq!(classify_http_status(401), FailureKind::Authentication);
        assert_eq!(classify_http_status(422), FailureKind::MalformedRequest);
    }
}
