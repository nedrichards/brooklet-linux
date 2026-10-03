use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    api::miniflux::{ReqwestMinifluxApi, ServerIdentity},
    error::BrookletError,
    model::Account,
    services::{
        traits::{MinifluxApi, Repository, SecretStore},
        url_policy::service_url,
    },
};

pub const PRIMARY_ACCOUNT_ID: i64 = 1;

pub struct SetupRequest {
    pub server_url: String,
    pub token: String,
}

#[async_trait]
pub trait IdentityValidator: Send + Sync {
    async fn validate(
        &self,
        server_url: &str,
        token: String,
    ) -> Result<ServerIdentity, BrookletError>;
}

pub struct MinifluxIdentityValidator;

#[async_trait]
impl IdentityValidator for MinifluxIdentityValidator {
    async fn validate(
        &self,
        server_url: &str,
        token: String,
    ) -> Result<ServerIdentity, BrookletError> {
        ReqwestMinifluxApi::new(server_url, token)?.validate().await
    }
}

#[async_trait]
pub trait SetupService: Send + Sync {
    async fn existing_account(&self) -> Result<Option<Account>, BrookletError>;
    async fn configure(&self, request: SetupRequest) -> Result<Account, BrookletError>;
    async fn reconnect(&self, token: String) -> Result<Account, BrookletError>;
}

pub struct AccountSetupService {
    validator: Arc<dyn IdentityValidator>,
    repository: Arc<dyn Repository>,
    secrets: Arc<dyn SecretStore>,
}

impl AccountSetupService {
    pub fn new(
        validator: Arc<dyn IdentityValidator>,
        repository: Arc<dyn Repository>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            validator,
            repository,
            secrets,
        }
    }
}

#[async_trait]
impl SetupService for AccountSetupService {
    async fn existing_account(&self) -> Result<Option<Account>, BrookletError> {
        self.repository.account().await
    }

    async fn configure(&self, request: SetupRequest) -> Result<Account, BrookletError> {
        if self.repository.account().await?.is_some() {
            return Err(BrookletError::AccountAlreadyConfigured);
        }
        let token = request.token.trim().to_owned();
        if token.is_empty() {
            return Err(BrookletError::InvalidSetup("an API token"));
        }
        let mut server_url = service_url(&request.server_url)?;
        if server_url.path().len() > 1 {
            let path = server_url.path().trim_end_matches('/').to_owned();
            server_url.set_path(&path);
        }
        let server_url = server_url.as_str().trim_end_matches('/').to_owned();
        let identity = self.validator.validate(&server_url, token.clone()).await?;
        let account = Account {
            id: PRIMARY_ACCOUNT_ID,
            server_url,
            username: identity.user.username,
            server_version: identity.version.version,
        };

        self.secrets
            .store_miniflux_token(account.id, &token)
            .await?;
        if let Err(error) = self.repository.save_account(&account).await {
            let _ = self.secrets.delete_account_secrets(account.id).await;
            return Err(error);
        }
        Ok(account)
    }

    async fn reconnect(&self, token: String) -> Result<Account, BrookletError> {
        let token = token.trim().to_owned();
        if token.is_empty() {
            return Err(BrookletError::InvalidSetup("an API token"));
        }
        let account = self
            .repository
            .account()
            .await?
            .ok_or(BrookletError::InvalidSetup("a configured Miniflux account"))?;
        let identity = self
            .validator
            .validate(&account.server_url, token.clone())
            .await?;
        if identity.user.username != account.username {
            return Err(BrookletError::AccountMismatch);
        }
        // Repair only the credential: no account metadata write or deletion is
        // needed, so a missing old secret can be repaired without loading it and
        // cached articles, reader positions, pending work and Karakeep stay intact.
        self.secrets
            .store_miniflux_token(account.id, &token)
            .await?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::api::miniflux::{UserDto, VersionDto};
    use crate::model::ReaderPosition;

    use super::*;

    struct FakeValidator {
        result: Result<ServerIdentity, &'static str>,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl IdentityValidator for FakeValidator {
        async fn validate(
            &self,
            server_url: &str,
            _token: String,
        ) -> Result<ServerIdentity, BrookletError> {
            self.calls.lock().unwrap().push(server_url.to_owned());
            self.result
                .clone()
                .map_err(|error| BrookletError::SecretStore(error.into()))
        }
    }

    #[derive(Default)]
    struct MemoryRepository(Mutex<Option<Account>>);

    #[async_trait]
    impl Repository for MemoryRepository {
        async fn account(&self) -> Result<Option<Account>, BrookletError> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn save_account(&self, account: &Account) -> Result<(), BrookletError> {
            *self.0.lock().unwrap() = Some(account.clone());
            Ok(())
        }

        async fn delete_account(&self, _account_id: i64) -> Result<(), BrookletError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }

        async fn replace_unread_snapshot(
            &self,
            _account_id: i64,
            _entries: &[crate::model::Entry],
        ) -> Result<(), BrookletError> {
            Ok(())
        }

        async fn unread_entries(
            &self,
            _account_id: i64,
        ) -> Result<Vec<crate::model::Entry>, BrookletError> {
            Ok(Vec::new())
        }

        async fn set_read_local(&self, _: i64, _: i64, _: bool) -> Result<(), BrookletError> {
            Ok(())
        }

        async fn pending_mutations(
            &self,
            _: i64,
        ) -> Result<Vec<crate::model::PendingMutation>, BrookletError> {
            Ok(Vec::new())
        }

        async fn acknowledge_mutation(
            &self,
            _: &crate::model::PendingMutation,
        ) -> Result<(), BrookletError> {
            Ok(())
        }

        async fn reader_position(
            &self,
            _entry_id: i64,
        ) -> Result<Option<ReaderPosition>, BrookletError> {
            Ok(None)
        }
    }

    struct FailingRepository;

    #[async_trait]
    impl Repository for FailingRepository {
        async fn account(&self) -> Result<Option<Account>, BrookletError> {
            Ok(None)
        }

        async fn save_account(&self, _account: &Account) -> Result<(), BrookletError> {
            Err(BrookletError::Storage(std::io::Error::other(
                "simulated write failure",
            )))
        }

        async fn delete_account(&self, _account_id: i64) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn replace_unread_snapshot(
            &self,
            _account_id: i64,
            _entries: &[crate::model::Entry],
        ) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn unread_entries(
            &self,
            _account_id: i64,
        ) -> Result<Vec<crate::model::Entry>, BrookletError> {
            Ok(Vec::new())
        }

        async fn set_read_local(&self, _: i64, _: i64, _: bool) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn pending_mutations(
            &self,
            _: i64,
        ) -> Result<Vec<crate::model::PendingMutation>, BrookletError> {
            unreachable!()
        }

        async fn acknowledge_mutation(
            &self,
            _: &crate::model::PendingMutation,
        ) -> Result<(), BrookletError> {
            unreachable!()
        }

        async fn reader_position(
            &self,
            _entry_id: i64,
        ) -> Result<Option<ReaderPosition>, BrookletError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct MemorySecrets(Mutex<Option<(i64, String)>>);

    #[async_trait]
    impl SecretStore for MemorySecrets {
        async fn load_miniflux_token(
            &self,
            account_id: i64,
        ) -> Result<Option<String>, BrookletError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(id, _)| *id == account_id)
                .map(|(_, token)| token.clone()))
        }

        async fn store_miniflux_token(
            &self,
            account_id: i64,
            token: &str,
        ) -> Result<(), BrookletError> {
            *self.0.lock().unwrap() = Some((account_id, token.to_owned()));
            Ok(())
        }

        async fn delete_account_secrets(&self, _account_id: i64) -> Result<(), BrookletError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    fn identity() -> ServerIdentity {
        ServerIdentity {
            user: UserDto {
                id: 7,
                username: "reader".into(),
                is_admin: false,
            },
            version: VersionDto {
                version: "2.3.2".into(),
                commit: None,
            },
        }
    }

    #[tokio::test]
    async fn setup_validates_then_persists_identity_and_secret_separately() {
        let validator = Arc::new(FakeValidator {
            result: Ok(identity()),
            calls: Mutex::new(Vec::new()),
        });
        let repository = Arc::new(MemoryRepository::default());
        let secrets = Arc::new(MemorySecrets::default());
        let service =
            AccountSetupService::new(validator.clone(), repository.clone(), secrets.clone());

        let account = service
            .configure(SetupRequest {
                server_url: " https://MINIFLUX.example/root/ ".into(),
                token: " secret-token ".into(),
            })
            .await
            .unwrap();

        assert_eq!(account.server_url, "https://miniflux.example/root");
        assert_eq!(account.username, "reader");
        assert_eq!(repository.account().await.unwrap(), Some(account));
        assert_eq!(
            secrets.load_miniflux_token(1).await.unwrap().as_deref(),
            Some("secret-token")
        );
        assert_eq!(
            validator.calls.lock().unwrap().as_slice(),
            ["https://miniflux.example/root"]
        );
    }

    #[tokio::test]
    async fn invalid_input_never_reaches_persistence() {
        let validator = Arc::new(FakeValidator {
            result: Ok(identity()),
            calls: Mutex::new(Vec::new()),
        });
        let repository = Arc::new(MemoryRepository::default());
        let secrets = Arc::new(MemorySecrets::default());
        let service = AccountSetupService::new(validator, repository.clone(), secrets.clone());

        assert!(
            service
                .configure(SetupRequest {
                    server_url: "http://miniflux.example".into(),
                    token: "secret-token".into(),
                })
                .await
                .is_err()
        );
        assert!(repository.account().await.unwrap().is_none());
        assert!(secrets.load_miniflux_token(1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn metadata_failure_removes_newly_stored_secret() {
        let validator = Arc::new(FakeValidator {
            result: Ok(identity()),
            calls: Mutex::new(Vec::new()),
        });
        let secrets = Arc::new(MemorySecrets::default());
        let service =
            AccountSetupService::new(validator, Arc::new(FailingRepository), secrets.clone());

        let result = service
            .configure(SetupRequest {
                server_url: "https://miniflux.example".into(),
                token: "secret-token".into(),
            })
            .await;

        assert!(result.is_err());
        assert!(secrets.load_miniflux_token(1).await.unwrap().is_none());
    }
}
