use async_trait::async_trait;

use crate::{error::BrookletError, services::traits::SecretStore};

pub struct Oo7SecretStore {
    application_id: String,
    keyring: tokio::sync::OnceCell<oo7::Keyring>,
}

impl Oo7SecretStore {
    pub fn new(application_id: impl Into<String>) -> Self {
        Self {
            application_id: application_id.into(),
            keyring: tokio::sync::OnceCell::new(),
        }
    }

    fn attributes<'a>(&'a self, account_id: &'a str) -> [(&'a str, &'a str); 3] {
        [
            ("application", self.application_id.as_str()),
            ("kind", "miniflux-token"),
            ("account-id", account_id),
        ]
    }

    fn karakeep_attributes<'a>(&'a self, account_id: &'a str) -> [(&'a str, &'a str); 3] {
        [
            ("application", self.application_id.as_str()),
            ("kind", "karakeep-key"),
            ("account-id", account_id),
        ]
    }

    async fn keyring(&self) -> Result<&oo7::Keyring, BrookletError> {
        self.keyring
            .get_or_try_init(|| async {
                oo7::Keyring::new()
                    .await
                    .map_err(|error| BrookletError::SecretStore(error.to_string()))
            })
            .await
    }
}

#[async_trait]
impl SecretStore for Oo7SecretStore {
    async fn load_miniflux_token(&self, account_id: i64) -> Result<Option<String>, BrookletError> {
        let account_id = account_id.to_string();
        let attributes = self.attributes(&account_id);
        let keyring = self.keyring().await?;
        let items = keyring
            .search_items(&attributes)
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item
            .secret()
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))?;
        String::from_utf8(secret.as_bytes().to_vec())
            .map(Some)
            .map_err(|_| BrookletError::SecretStore("stored token is not valid text".into()))
    }

    async fn store_miniflux_token(
        &self,
        account_id: i64,
        token: &str,
    ) -> Result<(), BrookletError> {
        let account_id = account_id.to_string();
        let attributes = self.attributes(&account_id);
        self.keyring()
            .await?
            .create_item("Brooklet Miniflux token", &attributes, token, true)
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))
    }

    async fn delete_account_secrets(&self, account_id: i64) -> Result<(), BrookletError> {
        let account_id = account_id.to_string();
        let attributes = self.attributes(&account_id);
        self.keyring()
            .await?
            .delete(&attributes)
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))?;
        self.keyring()
            .await?
            .delete(&self.karakeep_attributes(&account_id))
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))
    }

    async fn load_karakeep_key(&self, account_id: i64) -> Result<Option<String>, BrookletError> {
        let account_id = account_id.to_string();
        let items = self
            .keyring()
            .await?
            .search_items(&self.karakeep_attributes(&account_id))
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item
            .secret()
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))?;
        String::from_utf8(secret.as_bytes().to_vec())
            .map(Some)
            .map_err(|_| BrookletError::SecretStore("stored Karakeep key is not valid text".into()))
    }

    async fn delete_karakeep_key(&self, account_id: i64) -> Result<(), BrookletError> {
        self.keyring()
            .await?
            .delete(&self.karakeep_attributes(&account_id.to_string()))
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))
    }

    async fn store_karakeep_key(&self, account_id: i64, key: &str) -> Result<(), BrookletError> {
        let account_id = account_id.to_string();
        self.keyring()
            .await?
            .create_item(
                "Brooklet Karakeep API key",
                &self.karakeep_attributes(&account_id),
                key,
                true,
            )
            .await
            .map_err(|error| BrookletError::SecretStore(error.to_string()))
    }
}
