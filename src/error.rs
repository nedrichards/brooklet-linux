use crate::model::FailureKind;

#[derive(Debug, thiserror::Error)]
pub enum BrookletError {
    #[error("invalid service URL: {0}")]
    InvalidServiceUrl(String),
    #[error("service request failed with HTTP {status}")]
    Http { status: u16, kind: FailureKind },
    #[error("service transport failed")]
    Transport {
        kind: FailureKind,
        #[source]
        source: reqwest::Error,
    },
    #[error("Miniflux {found} is unsupported; Brooklet requires {required} or newer")]
    UnsupportedServer {
        found: String,
        required: &'static str,
    },
    #[error("database operation failed")]
    Database(#[source] rusqlite::Error),
    #[error("local storage could not be prepared")]
    Storage(#[source] std::io::Error),
    #[error("secure secret storage is unavailable: {0}")]
    SecretStore(String),
    #[error("setup information is incomplete: {0}")]
    InvalidSetup(&'static str),
    #[error(
        "This token belongs to a different Miniflux user. Enter a token for the connected account."
    )]
    AccountMismatch,
    #[error("An account is already connected. Use Reconnect to replace its API token.")]
    AccountAlreadyConfigured,
    #[error(
        "Timed out checking refreshed feeds. Cached articles remain available; try Sync again."
    )]
    RefreshFollowUpTimeout,
}

impl BrookletError {
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::InvalidServiceUrl(_) => FailureKind::MalformedRequest,
            Self::Http { kind, .. } | Self::Transport { kind, .. } => *kind,
            Self::UnsupportedServer { .. } => FailureKind::UnsupportedServer,
            Self::Database(_)
            | Self::Storage(_)
            | Self::SecretStore(_)
            | Self::RefreshFollowUpTimeout => FailureKind::Retryable,
            Self::InvalidSetup(_) | Self::AccountMismatch | Self::AccountAlreadyConfigured => {
                FailureKind::MalformedRequest
            }
        }
    }

    pub fn setup_message(&self) -> String {
        match self {
            Self::AccountMismatch | Self::AccountAlreadyConfigured | Self::RefreshFollowUpTimeout => self.to_string(),
            Self::InvalidServiceUrl(detail) => format!("Check the Miniflux address: {detail}."),
            Self::InvalidSetup(detail) => format!("Please enter {detail}."),
            Self::Http { status: 401 | 403, .. } => {
                "Miniflux rejected this API token. Check the token and try again.".into()
            }
            Self::Http { status, .. } => {
                format!("Miniflux returned HTTP {status}. Check the address and server configuration.")
            }
            Self::Transport { kind: FailureKind::Certificate, .. } => {
                "The server certificate could not be verified. Check the address and certificate configuration.".into()
            }
            Self::Transport { .. } => {
                "Brooklet could not reach Miniflux. Check your connection and server address.".into()
            }
            Self::UnsupportedServer { found, required } => format!(
                "This server runs Miniflux {found}. Brooklet requires {required} or newer."
            ),
            Self::SecretStore(_) => {
                "The account was verified, but the token could not be saved securely. Check that your system secret service is available.".into()
            }
            Self::Database(_) | Self::Storage(_) => {
                "The account was verified, but Brooklet could not save its local configuration.".into()
            }
        }
    }

    pub fn karakeep_message(&self) -> String {
        match self {
            Self::Http { status: 401 | 403, .. } => "Karakeep rejected the API key. Update it in Preferences, then retry the delivery.".into(),
            Self::Http { status, .. } => format!("Karakeep returned HTTP {status}. Check the endpoint or Miniflux integration, then retry."),
            Self::InvalidServiceUrl(detail) => format!("Check the Karakeep endpoint: {detail}."),
            Self::InvalidSetup(detail) => format!("Please enter {detail}."),
            Self::Transport { kind: FailureKind::MalformedRequest, .. } => "The endpoint did not return a Karakeep bookmarks response. Check its address.".into(),
            Self::Transport { .. } => "Karakeep could not be reached. Check the connection and endpoint, then retry.".into(),
            Self::SecretStore(_) => "The Karakeep key could not be accessed or saved securely. Check your system secret service.".into(),
            _ => "Brooklet could not update local Karakeep delivery data.".into(),
        }
    }

    pub fn sync_message(&self) -> String {
        match self {
            Self::Http {
                status: 401 | 403, ..
            } => {
                "Miniflux rejected the saved API token. Reconnect your account to continue.".into()
            }
            Self::Transport {
                kind: FailureKind::Certificate,
                ..
            } => {
                "The Miniflux certificate could not be verified. Cached articles remain available."
                    .into()
            }
            Self::Transport { .. } => {
                "Miniflux is unreachable. Showing cached articles; try again when you are online."
                    .into()
            }
            Self::UnsupportedServer { found, required } => {
                format!("This server runs Miniflux {found}; Brooklet requires {required} or newer.")
            }
            Self::SecretStore(_) => {
                "The saved Miniflux token is unavailable. Reconnect your account to continue."
                    .into()
            }
            Self::Database(_) | Self::Storage(_) => {
                "Brooklet could not update its local article cache.".into()
            }
            Self::RefreshFollowUpTimeout => self.to_string(),
            Self::InvalidServiceUrl(_)
            | Self::InvalidSetup(_)
            | Self::Http { .. }
            | Self::AccountMismatch
            | Self::AccountAlreadyConfigured => self.setup_message(),
        }
    }
}

impl From<rusqlite::Error> for BrookletError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for BrookletError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error)
    }
}
