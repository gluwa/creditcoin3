//! Durable messages waiting for gossip handoff. Scanning persists each message before delivery
//! into memory; only a successful gossip publish removes it. Replays are bounded and idempotent.

use super::listener::IndexedMessage;
use alloy::primitives::B256;
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;
use write_ability::envelope::MessageVote;

#[derive(Debug)]
pub(super) struct PersistenceError(anyhow::Error);
impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "persist pending message before advancing scan: {:#}",
            self.0
        )
    }
}
impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Clone, Debug)]
pub struct Key {
    domain: B256,
    message: B256,
}
impl From<&IndexedMessage> for Key {
    fn from(message: &IndexedMessage) -> Self {
        Self {
            domain: message.destination_chain_key,
            message: message.message_id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PendingVote {
    pub vote: MessageVote,
    pub key: Key,
}
impl std::ops::Deref for PendingVote {
    type Target = MessageVote;
    fn deref(&self) -> &MessageVote {
        &self.vote
    }
}

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
    // Owned inside blocking closures: cancelling an async caller cannot let a later write
    // overtake an unfinished disk operation. Reads and acknowledgements use the same lock.
    io: Arc<Semaphore>,
    ack_tx: mpsc::Sender<Key>,
    ack_rx: Arc<parking_lot::Mutex<Option<mpsc::Receiver<Key>>>>,
}
impl Store {
    pub fn new(dir: &std::path::Path, chain_key: u64) -> Self {
        let (ack_tx, ack_rx) = mpsc::channel(100);
        Self {
            dir: dir.join(format!("pending-messages-{chain_key}")),
            io: Arc::new(Semaphore::new(1)),
            ack_tx,
            ack_rx: Arc::new(parking_lot::Mutex::new(Some(ack_rx))),
        }
    }
    fn path(&self, key: &Key) -> PathBuf {
        self.dir
            .join(hex::encode(key.domain))
            .join(format!("{}.json", hex::encode(key.message)))
    }
    async fn disk<T: Send + 'static>(
        &self,
        work: impl FnOnce(Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.clone();
        super::cursor::blocking_io(store.io.clone(), move || work(store)).await
    }
    pub async fn save(&self, message: IndexedMessage) -> Result<()> {
        self.disk(move |store| {
            let path = store.path(&Key::from(&message));
            if path.exists() {
                return Ok(());
            }
            let domain = path
                .parent()
                .expect("pending message has a domain directory");
            std::fs::create_dir_all(domain)?;
            // Persist the domain directory entry before a cursor can reference any of its files.
            std::fs::File::open(&store.dir)?.sync_all()?;
            if let Some(parent) = store.dir.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
            super::cursor::atomic_write(&path, &serde_json::to_vec(&message)?)
        })
        .await
        .map_err(|err| anyhow::Error::new(PersistenceError(err)))
    }
    pub async fn batch(&self, domain: B256) -> Result<Vec<IndexedMessage>> {
        self.disk(move |store| {
            let dir = store.dir.join(hex::encode(domain));
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(err) => return Err(err.into()),
            };
            let mut messages = Vec::new();
            for entry in entries {
                let path = entry?.path();
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                let message: IndexedMessage = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("invalid pending message {}", path.display()))?;
                anyhow::ensure!(
                    store.path(&Key::from(&message)) == path,
                    "pending-message identity mismatch"
                );
                messages.push(message);
                if messages.len() == 100 {
                    break;
                }
            }
            Ok(messages)
        })
        .await
    }
    pub fn acknowledge(&self, key: Key) {
        // A full queue retains the file for replay. Disk latency never blocks the swarm.
        if self.ack_tx.try_send(key).is_err() {
            tracing::warn!("pending-message acknowledgement deferred; message will replay");
        }
    }
    pub async fn remove(&self, key: Key) -> Result<()> {
        self.disk(move |store| {
            let path = store.path(&key);
            match std::fs::remove_file(&path) {
                Ok(()) => std::fs::File::open(path.parent().unwrap())?.sync_all()?,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            Ok(())
        })
        .await
    }
    pub async fn run_acknowledgements(&self, token: CancellationToken) -> Result<()> {
        let mut rx = self
            .ack_rx
            .lock()
            .take()
            .context("pending acknowledgements already running")?;
        loop {
            let key = tokio::select! {
                () = token.cancelled() => return Ok(()),
                key = rx.recv() => key.context("pending acknowledgement channel closed")?,
            };
            tokio::select! {
                () = token.cancelled() => return Ok(()),
                result = self.remove(key) => if let Err(err) = result {
                    tracing::warn!(%err, "pending message retained after acknowledgement write failure");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pending-{tag}-{}-{nonce}", std::process::id()))
    }
    fn message() -> IndexedMessage {
        IndexedMessage {
            message_id: B256::repeat_byte(1),
            outbox: alloy::primitives::Address::repeat_byte(2),
            destination_chain_key: B256::repeat_byte(3),
        }
    }
    #[tokio::test]
    async fn pending_work_survives_a_cursor_beyond_the_restart_window() {
        let dir = dir("restart");
        let original = message();
        let store = Store::new(&dir, 7);
        store.save(original.clone()).await.unwrap();
        let cursor = super::super::cursor::CursorStore::for_all_outboxes(&dir, 7, None);
        cursor.save_async(2000).await.unwrap();
        drop(store);
        let restarted = Store::new(&dir, 7);
        assert_eq!(cursor.load_async().await.unwrap(), Some(2000));
        assert_eq!(
            restarted
                .batch(original.destination_chain_key)
                .await
                .unwrap()[0]
                .message_id,
            original.message_id
        );
        restarted.remove((&original).into()).await.unwrap();
        assert!(restarted
            .batch(original.destination_chain_key)
            .await
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn full_ack_queue_retains_work_and_rekey_does_not_delete_it() {
        let dir = dir("ack");
        let store = Store::new(&dir, 7);
        let original = message();
        store.save(original.clone()).await.unwrap();
        let mut rekeyed = original.clone();
        rekeyed.destination_chain_key = B256::repeat_byte(4);
        store.save(rekeyed.clone()).await.unwrap();
        for _ in 0..101 {
            store.acknowledge((&original).into());
        }
        assert_eq!(
            store
                .batch(original.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
        store.remove((&original).into()).await.unwrap();
        assert_eq!(
            store
                .batch(rekeyed.destination_chain_key)
                .await
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn corrupt_pending_record_and_write_failure_fail_closed() {
        let dir = dir("corrupt");
        let store = Store::new(&dir, 7);
        let original = message();
        store.save(original.clone()).await.unwrap();
        std::fs::write(store.path(&(&original).into()), b"bad json").unwrap();
        assert!(store.batch(original.destination_chain_key).await.is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::write(&dir, b"not a directory").unwrap();
        assert!(store.save(original).await.is_err());
        std::fs::remove_file(dir).unwrap();
    }
}
