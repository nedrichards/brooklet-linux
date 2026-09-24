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
}

impl BrookletError {
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::InvalidServiceUrl(_) => FailureKind::MalformedRequest,
            Self::Http { kind, .. } | Self::Transport { kind, .. } => *kind,
            Self::UnsupportedServer { .. } => FailureKind::UnsupportedServer,
            Self::Database(_) | Self::Storage(_) | Self::SecretStore(_) => FailureKind::Retryable,
            Self::InvalidSetup(_) => FailureKind::MalformedRequest,
        }
    }

    pub fn setup_message(&self) -> String {
        match self {
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
            Self::InvalidServiceUrl(_) | Self::InvalidSetup(_) | Self::Http { .. } => {
                self.setup_message()
            }
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
