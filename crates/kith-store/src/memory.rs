//! In-memory store for tests.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::{Key, Namespace, Result, Secrecy, Store, format::checked_length};

/// A [`Store`] held in memory. Nothing survives the process.
#[derive(Debug, Default)]
pub struct MemoryStore {
    values: Mutex<BTreeMap<Key, Vec<u8>>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Locks the map, recovering from poisoning.
    ///
    /// Every critical section is a single `BTreeMap` call, which either
    /// completes or leaves the map unchanged, so a panic elsewhere in a
    /// holder's thread cannot leave a half-applied update behind. Recovering
    /// is therefore sound and keeps one panicking test from cascading.
    fn lock(&self) -> MutexGuard<'_, BTreeMap<Key, Vec<u8>>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store for MemoryStore {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>> {
        Ok(self.lock().get(key).cloned())
    }

    fn put_with(&self, key: &Key, value: &[u8], _secrecy: Secrecy) -> Result<()> {
        checked_length(value)?;
        self.lock().insert(key.clone(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &Key) -> Result<()> {
        self.lock().remove(key);
        Ok(())
    }

    fn list(&self, namespace: &Namespace) -> Result<Vec<Key>> {
        Ok(self
            .lock()
            .keys()
            .filter(|key| key.namespace == *namespace)
            .cloned()
            .collect())
    }
}
