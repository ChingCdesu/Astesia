use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use uuid::Uuid;

use super::{CredentialVault, CredentialVaultError, CredentialVaultErrorCode};

#[derive(Default)]
pub struct MemoryCredentialVault {
    secrets: Mutex<HashMap<String, (Vec<u8>, String)>>,
    failure: Mutex<Option<CredentialVaultErrorCode>>,
}

impl MemoryCredentialVault {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn fail_with(&self, code: Option<CredentialVaultErrorCode>) {
        *self.failure.lock().expect("failure lock") = code;
    }

    pub fn contains_secret(&self, secret: &str) -> bool {
        self.secrets
            .lock()
            .expect("secrets lock")
            .values()
            .any(|(_, stored)| stored == secret)
    }

    fn failure(&self, operation: &str) -> Result<(), CredentialVaultError> {
        match *self.failure.lock().expect("failure lock") {
            Some(code) => Err(CredentialVaultError::new(code, operation)),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl CredentialVault for MemoryCredentialVault {
    async fn put(&self, binding: &[u8], secret: &str) -> Result<String, CredentialVaultError> {
        self.failure("保存数据库凭据")?;
        let reference = Uuid::new_v4().to_string();
        self.secrets
            .lock()
            .expect("secrets lock")
            .insert(reference.clone(), (binding.to_vec(), secret.to_string()));
        Ok(reference)
    }

    async fn get(&self, reference: &str, binding: &[u8]) -> Result<String, CredentialVaultError> {
        self.failure("读取数据库凭据")?;
        let secrets = self.secrets.lock().expect("secrets lock");
        let (stored_binding, secret) = secrets.get(reference).ok_or_else(|| {
            CredentialVaultError::new(CredentialVaultErrorCode::Missing, "读取数据库凭据")
        })?;
        if stored_binding != binding {
            return Err(CredentialVaultError::new(
                CredentialVaultErrorCode::Corrupt,
                "读取数据库凭据",
            ));
        }
        Ok(secret.clone())
    }

    async fn delete(&self, reference: &str) -> Result<(), CredentialVaultError> {
        self.failure("删除数据库凭据")?;
        self.secrets.lock().expect("secrets lock").remove(reference);
        Ok(())
    }
}

#[tokio::test]
async fn memory_vault_authenticates_binding() {
    let vault = MemoryCredentialVault::shared();
    let reference = vault.put(b"expected", "secret").await.expect("put");
    assert_eq!(
        vault.get(&reference, b"expected").await.expect("get"),
        "secret"
    );
    let error = vault
        .get(&reference, b"different")
        .await
        .expect_err("different binding must fail");
    assert_eq!(error.code, CredentialVaultErrorCode::Corrupt);
}
