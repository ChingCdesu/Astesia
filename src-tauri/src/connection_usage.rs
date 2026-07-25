use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use fs4::{FileExt, TryLockError};
use ring::digest::{digest, SHA256};

#[derive(Clone, Debug)]
pub(crate) struct ConnectionUsageLocks {
    directory: Arc<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct ConnectionUsageLease {
    _file: File,
}

#[derive(Debug)]
pub(crate) struct ConnectionMutationGuard {
    _file: File,
}

#[derive(Debug)]
pub(crate) enum ConnectionUsageError {
    Contended,
    Io(io::Error),
}

impl std::fmt::Display for ConnectionUsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contended => formatter.write_str("connection usage lock is contended"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ConnectionUsageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contended => None,
            Self::Io(error) => Some(error),
        }
    }
}

impl ConnectionUsageLocks {
    pub(crate) fn for_repository(database_path: &Path) -> Self {
        let mut directory_name = database_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("connections.sqlite3"))
            .to_os_string();
        directory_name.push(".leases");
        Self {
            directory: Arc::new(database_path.with_file_name(directory_name)),
        }
    }

    pub(crate) fn try_acquire_usage(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionUsageLease, ConnectionUsageError> {
        let file = self.open_lock_file(connection_id)?;
        FileExt::try_lock_shared(&file).map_err(map_try_lock_error)?;
        Ok(ConnectionUsageLease { _file: file })
    }

    pub(crate) fn try_acquire_mutation(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionMutationGuard, ConnectionUsageError> {
        let file = self.open_lock_file(connection_id)?;
        FileExt::try_lock(&file).map_err(map_try_lock_error)?;
        Ok(ConnectionMutationGuard { _file: file })
    }

    pub(crate) fn is_in_use(&self, connection_id: &str) -> Result<bool, ConnectionUsageError> {
        match self.try_acquire_mutation(connection_id) {
            Ok(_guard) => Ok(false),
            Err(ConnectionUsageError::Contended) => {
                // A second shared lock distinguishes MCP readers from an
                // exclusive profile mutation. The latter must not be reported
                // to callers as a lingering STDIO session.
                self.try_acquire_usage(connection_id).map(|_probe| true)
            }
            Err(error) => Err(error),
        }
    }

    fn open_lock_file(&self, connection_id: &str) -> Result<File, ConnectionUsageError> {
        std::fs::create_dir_all(self.directory.as_ref()).map_err(ConnectionUsageError::Io)?;
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.lock_path(connection_id))
            .map_err(ConnectionUsageError::Io)
    }

    fn lock_path(&self, connection_id: &str) -> PathBuf {
        self.directory
            .join(format!("{}.lock", sha256_hex(connection_id.as_bytes())))
    }

    #[cfg(test)]
    fn directory(&self) -> &Path {
        self.directory.as_ref()
    }
}

fn map_try_lock_error(error: TryLockError) -> ConnectionUsageError {
    match error {
        TryLockError::WouldBlock => ConnectionUsageError::Contended,
        TryLockError::Error(error) => ConnectionUsageError::Io(error),
    }
}

fn sha256_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let hash = digest(&SHA256, value);
    let mut encoded = String::with_capacity(hash.as_ref().len() * 2);
    for byte in hash.as_ref() {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locks(directory: &tempfile::TempDir) -> ConnectionUsageLocks {
        ConnectionUsageLocks::for_repository(&directory.path().join("connections.sqlite3"))
    }

    #[test]
    fn multiple_usage_leases_coexist_and_block_mutation() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let locks = locks(&directory);
        let first = locks.try_acquire_usage("analytics").expect("first usage");
        let second = locks.try_acquire_usage("analytics").expect("second usage");

        assert!(matches!(
            locks.try_acquire_mutation("analytics"),
            Err(ConnectionUsageError::Contended)
        ));
        assert!(locks.is_in_use("analytics").expect("probe"));

        drop(first);
        assert!(locks.is_in_use("analytics").expect("second still held"));
        drop(second);
        assert!(!locks.is_in_use("analytics").expect("released"));
    }

    #[test]
    fn mutation_guard_blocks_new_usage_non_blockingly() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let locks = locks(&directory);
        let mutation = locks
            .try_acquire_mutation("analytics")
            .expect("mutation guard");

        assert!(matches!(
            locks.try_acquire_usage("analytics"),
            Err(ConnectionUsageError::Contended)
        ));

        assert!(matches!(
            locks.is_in_use("analytics"),
            Err(ConnectionUsageError::Contended)
        ));
        drop(mutation);
        locks
            .try_acquire_usage("analytics")
            .expect("usage after mutation");
    }

    #[test]
    fn connection_ids_are_hashed_and_lock_files_are_retained() {
        let directory = tempfile::TempDir::new().expect("tempdir");
        let locks = locks(&directory);
        let connection_id = "../unsafe/connection";
        let expected_path = locks.lock_path(connection_id);
        {
            let _lease = locks.try_acquire_usage(connection_id).expect("usage lease");
            assert!(expected_path.starts_with(locks.directory()));
            assert_eq!(
                expected_path.file_name().and_then(|name| name.to_str()),
                Some("cca8551a10b58b83f6346486c3acdf4a3e6994338a2a3ff56a1d49021388c5ca.lock")
            );
        }
        assert!(expected_path.exists());
    }
}
