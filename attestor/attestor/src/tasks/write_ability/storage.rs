//! Per-attestor durable state. Sharing a configured base directory must never share progress or
//! acknowledgements between signers. A file lock also excludes duplicate processes for one signer.

use super::{cursor::CursorStore, pending};
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
pub(super) struct StateDirectory {
    path: PathBuf,
    // Never unlink the lock file: replacing its inode would allow two owners. Dropping the last
    // store (including an in-flight blocking operation) closes the file and releases ownership.
    _lock: File,
}

impl StateDirectory {
    pub(super) fn open(
        base: &Path,
        chain_key: u64,
        account_id: [u8; 32],
        start_block: Option<u64>,
    ) -> Result<Arc<Self>> {
        super::cursor::ensure_writable(base)?;
        let path = base.join(format!("write-ability-{}", hex::encode(account_id)));
        std::fs::create_dir_all(&path)?;
        File::open(base)?.sync_all()?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.join(format!(".chain-{chain_key}.lock")))?;
        lock.try_lock_exclusive().with_context(|| {
            format!(
                "write-ability state {} for chain {chain_key} is already in use or cannot be locked; \
                 run only one process per attestor identity and chain",
                path.display()
            )
        })?;
        let directory = Arc::new(Self { path, _lock: lock });
        let cursor = CursorStore::for_all_outboxes(directory.path(), chain_key, start_block);
        if cursor.load()?.is_none() {
            let legacy_cursor = CursorStore::for_all_outboxes(base, chain_key, start_block);
            let had_cursor = legacy_cursor.load()?.is_some();
            let had_pending =
                pending::Store::new(directory.path(), chain_key).import_legacy(base)?;
            if had_cursor || had_pending {
                // Shared legacy progress may belong to a different signer, whose acknowledgement
                // may already have deleted our work. Preserve pending records, but replay the
                // configured history. Saving the cursor last makes interrupted imports retryable.
                cursor.save(cursor.scan_floor())?;
                tracing::warn!(
                    path = %directory.path.display(),
                    chain_key,
                    replay_floor = cursor.scan_floor(),
                    "migrated shared write-ability state; replaying configured history for this attestor"
                );
            }
        }
        Ok(directory)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::super::listener::IndexedMessage;
    use super::*;
    use alloy::primitives::{Address, B256};

    struct TempDirectory(PathBuf);
    impl TempDirectory {
        fn new(tag: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "attestor-state-{tag}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn message(id: u8) -> IndexedMessage {
        IndexedMessage {
            message_id: B256::repeat_byte(id),
            outbox: Address::repeat_byte(2),
            destination_chain_key: B256::repeat_byte(3),
        }
    }

    #[tokio::test]
    async fn signers_sharing_a_base_recover_their_own_votes_and_progress() {
        let base = TempDirectory::new("independent");
        let first = pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .unwrap();
        let second = pending::Store::open(&base.0, 7, [2; 32], None)
            .await
            .unwrap();
        let original = message(1);
        first.save(original.clone()).await.unwrap();
        second.save(original.clone()).await.unwrap();
        // A pending vote must survive even when its source block is far beyond restart rewind.
        first.cursor(None).save_async(2_000).await.unwrap();
        second.cursor(None).save_async(5_000).await.unwrap();
        first.remove((&original).into()).await.unwrap();
        assert_eq!(
            second
                .batch(original.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
        drop(first);
        drop(second);

        let first = pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .unwrap();
        let second = pending::Store::open(&base.0, 7, [2; 32], None)
            .await
            .unwrap();
        assert_eq!(first.cursor(None).load_async().await.unwrap(), Some(2_000));
        assert_eq!(second.cursor(None).load_async().await.unwrap(), Some(5_000));
        assert!(first
            .batch(original.destination_chain_key)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            second.batch(original.destination_chain_key).await.unwrap()[0].message_id,
            original.message_id
        );
    }

    #[tokio::test]
    async fn duplicate_identity_is_excluded_until_ownership_is_released() {
        let base = TempDirectory::new("exclusive");
        let owner = pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .unwrap();
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_err());
        // One signer can serve different chains without sharing their progress or queue.
        let other_chain = pending::Store::open(&base.0, 8, [1; 32], None)
            .await
            .unwrap();
        other_chain.cursor(None).save_async(800).await.unwrap();
        assert_eq!(owner.cursor(None).load_async().await.unwrap(), None);
        drop(owner);
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn cloned_cursor_retains_ownership_after_pending_store_is_dropped() {
        let base = TempDirectory::new("cursor-owner");
        let owner = pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .unwrap();
        let cursor = owner.cursor(None);
        let survivor = cursor.clone();
        drop(cursor);
        drop(owner);
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_err());
        survivor.save_async(123).await.unwrap();
        assert_eq!(survivor.load_async().await.unwrap(), Some(123));
        drop(survivor);
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn legacy_migration_preserves_other_signers_and_never_reimports_completed_work() {
        let base = TempDirectory::new("legacy");
        let original = message(1);
        let legacy = pending::Store::new(&base.0, 7);
        legacy.save(original.clone()).await.unwrap();
        CursorStore::for_all_outboxes(&base.0, 7, None)
            .save(9_999)
            .unwrap();

        let migrated = pending::Store::open(&base.0, 7, [1; 32], Some(100))
            .await
            .unwrap();
        assert_eq!(
            migrated.cursor(Some(100)).load_async().await.unwrap(),
            Some(99)
        );
        assert_eq!(
            migrated
                .batch(original.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            legacy
                .batch(original.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            CursorStore::for_all_outboxes(&base.0, 7, None)
                .load()
                .unwrap(),
            Some(9_999)
        );
        migrated.remove((&original).into()).await.unwrap();
        migrated.cursor(None).save_async(12_000).await.unwrap();
        drop(migrated);

        let restarted = pending::Store::open(&base.0, 7, [1; 32], Some(10))
            .await
            .unwrap();
        assert_eq!(
            restarted.cursor(Some(10)).load_async().await.unwrap(),
            Some(12_000)
        );
        assert!(restarted
            .batch(original.destination_chain_key)
            .await
            .unwrap()
            .is_empty());
        // Another signer still needs the untouched legacy queue and its own replay floor.
        let second = pending::Store::open(&base.0, 7, [2; 32], Some(200))
            .await
            .unwrap();
        assert_eq!(
            second.cursor(Some(200)).load_async().await.unwrap(),
            Some(199)
        );
        assert_eq!(
            second
                .batch(original.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn a_fresh_namespace_keeps_the_uninitialized_cursor() {
        let base = TempDirectory::new("fresh");
        let fresh = pending::Store::open(&base.0, 7, [1; 32], Some(100))
            .await
            .unwrap();
        assert_eq!(fresh.cursor(Some(100)).load_async().await.unwrap(), None);
        assert!(fresh
            .batch(message(1).destination_chain_key)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn interrupted_import_retries_before_committing_the_replay_cursor() {
        let base = TempDirectory::new("interrupted");
        let directory = StateDirectory::open(&base.0, 7, [1; 32], Some(50)).unwrap();
        let scoped_path = directory.path().to_owned();
        drop(directory);
        let first = message(1);
        let second = message(2);
        let legacy = pending::Store::new(&base.0, 7);
        legacy.save(first.clone()).await.unwrap();
        legacy.save(second.clone()).await.unwrap();
        CursorStore::for_all_outboxes(&base.0, 7, None)
            .save(9_999)
            .unwrap();
        // Simulate a crash after copying only one record and before the cursor commit.
        pending::Store::new(&scoped_path, 7)
            .save(first.clone())
            .await
            .unwrap();
        assert_eq!(
            CursorStore::for_all_outboxes(&scoped_path, 7, None)
                .load()
                .unwrap(),
            None
        );

        let recovered = pending::Store::open(&base.0, 7, [1; 32], Some(50))
            .await
            .unwrap();
        let recovered_messages = recovered.batch(first.destination_chain_key).await.unwrap();
        assert_eq!(recovered_messages.len(), 2);
        assert!(recovered_messages
            .iter()
            .any(|m| m.message_id == first.message_id));
        assert!(recovered_messages
            .iter()
            .any(|m| m.message_id == second.message_id));
        assert_eq!(
            recovered.cursor(Some(50)).load_async().await.unwrap(),
            Some(49)
        );
        assert_eq!(
            legacy
                .batch(first.destination_chain_key)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn corrupt_legacy_state_does_not_commit_a_migration_cursor() {
        let base = TempDirectory::new("corrupt-legacy");
        let directory = StateDirectory::open(&base.0, 7, [1; 32], None).unwrap();
        let scoped_path = directory.path().to_owned();
        drop(directory);
        let legacy_cursor = CursorStore::for_all_outboxes(&base.0, 7, None);
        std::fs::write(legacy_cursor.path(), b"bad json").unwrap();
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_err());
        legacy_cursor.save(9_999).unwrap();
        let original = message(1);
        let legacy = pending::Store::new(&base.0, 7);
        legacy.save(original.clone()).await.unwrap();
        let domain = base
            .0
            .join("pending-messages-7")
            .join(hex::encode(original.destination_chain_key));
        let record = std::fs::read_dir(domain)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(record, b"bad json").unwrap();
        assert!(pending::Store::open(&base.0, 7, [1; 32], None)
            .await
            .is_err());
        assert_eq!(
            CursorStore::for_all_outboxes(&scoped_path, 7, None)
                .load()
                .unwrap(),
            None
        );
    }
}
