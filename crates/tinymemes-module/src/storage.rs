//! Owner-local collection preparation and state migration.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tinymemes_bus::storage::{
    LEASES_COLLECTION, LEGACY_MARKERS_COLLECTION, LEGACY_STATE_COLLECTION, MARKERS_COLLECTION,
    MARKERS_STEP, MIGRATIONS_COLLECTION, OWNER, STATE_COLLECTION, STATE_STEP, STATE_VERSION,
    collections, migrate_state_body,
};
use tinystoragedrivers_api::{DocumentStore, DocumentStoreExt, Storage};
use tinystoragedrivers_types::{
    Capability, CollectionSpec, ErrorKind, ExpectedDocument, Key, Precondition, ReadCheck,
    StorageError, Version, WriteOp, WriteOptions,
};
use tokio::sync::Mutex;

const LEASE_KEY: &str = "owner_migrations";
const LEASE_TTL_MS: i64 = 30_000;
const MAX_CAS_ATTEMPTS: usize = 8;
static NEXT_HOLDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Progress {
    owner: String,
    step: String,
    from: u32,
    to: u32,
    cursor: Option<String>,
    written: u64,
    done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LeaseRecord {
    holder: String,
    epoch: u64,
    expires_ms: i64,
}

/// Ensure this owner’s declared collections and migrate its legacy state.
///
/// This operates on the storage handle already bound by the host. It does not
/// choose a scope or compose another owner’s migrations.
///
/// # Errors
/// Returns a port error if collection preparation or migration fails.
pub async fn prepare(storage: &Storage) -> tinystoragedrivers_types::Result<()> {
    for collection in collections() {
        storage.documents().ensure_collection(&collection).await?;
    }
    with_owner_lease(storage).await
}

async fn migrate_state(
    storage: &Storage,
    lease: &OwnerLease,
) -> tinystoragedrivers_types::Result<()> {
    let docs = storage.documents();
    let key = Key::new(format!("{OWNER}/{STATE_COLLECTION}/{STATE_STEP}"))?;
    if docs
        .get_as::<Progress>(MIGRATIONS_COLLECTION, key.as_str())
        .await?
        .is_some_and(|progress| progress.doc.done)
    {
        return Ok(());
    }

    let legacy = docs
        .query_all(
            LEGACY_STATE_COLLECTION,
            &tinystoragedrivers_types::Query::all(),
        )
        .await?;
    for source in legacy {
        lease.renew().await?;
        let target_body = migrate_state_body(&source.id, &source.doc);
        match docs.get(STATE_COLLECTION, &source.id).await? {
            Some(target)
                if target.doc == target_body
                    && target.meta.schema == STATE_VERSION
                    && target.meta.expires_ms == source.meta.expires_ms => {}
            Some(_) => {
                return Err(StorageError::conflict(format!(
                    "{} migration found a different target document `{}`; source kept",
                    STATE_STEP, source.id
                )));
            }
            None => {
                let options = match source.meta.expires_ms {
                    Some(expires_ms) => WriteOptions::new()
                        .schema(STATE_VERSION)
                        .expires_ms(expires_ms),
                    None => WriteOptions::new().schema(STATE_VERSION),
                };
                docs.put_with(
                    STATE_COLLECTION,
                    &source.id,
                    target_body,
                    Precondition::Absent,
                    options,
                )
                .await?;
            }
        }
        lease.renew().await?;
        docs.delete(
            LEGACY_STATE_COLLECTION,
            &source.id,
            Precondition::Version(source.version),
        )
        .await?;
    }

    // Re-read before completion so a source written while the migration ran
    // cannot be hidden by the done marker.
    if !docs
        .query_all(
            LEGACY_STATE_COLLECTION,
            &tinystoragedrivers_types::Query::all(),
        )
        .await?
        .is_empty()
    {
        return Err(StorageError::conflict(
            "legacy memes state changed while migration was running",
        ));
    }
    lease.mark_done(&key, STATE_STEP, 1, STATE_VERSION).await
}

async fn migrate_markers(
    storage: &Storage,
    lease: &OwnerLease,
) -> tinystoragedrivers_types::Result<()> {
    let docs = storage.documents();
    let key = Key::new(format!("{OWNER}/{MARKERS_COLLECTION}/{MARKERS_STEP}"))?;
    if docs
        .get_as::<Progress>(MIGRATIONS_COLLECTION, key.as_str())
        .await?
        .is_some_and(|progress| progress.doc.done)
    {
        return Ok(());
    }

    let legacy = docs
        .query_all(
            LEGACY_MARKERS_COLLECTION,
            &tinystoragedrivers_types::Query::all(),
        )
        .await?;
    for source in legacy {
        lease.renew().await?;
        match docs.get(MARKERS_COLLECTION, &source.id).await? {
            Some(target)
                if target.doc == source.doc
                    && target.meta.schema == source.meta.schema
                    && target.meta.expires_ms == source.meta.expires_ms => {}
            Some(_) => {
                return Err(StorageError::conflict(format!(
                    "{} migration found a different target document `{}`; source kept",
                    MARKERS_STEP, source.id
                )));
            }
            None => {
                let options = match source.meta.expires_ms {
                    Some(expires_ms) => WriteOptions::new()
                        .schema(source.meta.schema)
                        .expires_ms(expires_ms),
                    None => WriteOptions::new().schema(source.meta.schema),
                };
                docs.put_with(
                    MARKERS_COLLECTION,
                    &source.id,
                    source.doc.clone(),
                    Precondition::Absent,
                    options,
                )
                .await?;
            }
        }

        let target = docs
            .get(MARKERS_COLLECTION, &source.id)
            .await?
            .ok_or_else(|| StorageError::conflict("marker migration target disappeared"))?;
        if target.doc != source.doc
            || target.meta.schema != source.meta.schema
            || target.meta.expires_ms != source.meta.expires_ms
        {
            return Err(StorageError::conflict(format!(
                "{} migration target `{}` does not match source; source kept",
                MARKERS_STEP, source.id
            )));
        }
        lease.renew().await?;
        docs.delete(
            LEGACY_MARKERS_COLLECTION,
            &source.id,
            Precondition::Version(source.version),
        )
        .await?;
    }

    if !docs
        .query_all(
            LEGACY_MARKERS_COLLECTION,
            &tinystoragedrivers_types::Query::all(),
        )
        .await?
        .is_empty()
    {
        return Err(StorageError::conflict(
            "legacy remix markers changed while migration was running",
        ));
    }
    lease.mark_done(&key, MARKERS_STEP, 1, 1).await
}

async fn with_owner_lease(storage: &Storage) -> tinystoragedrivers_types::Result<()> {
    let lease = OwnerLease::acquire(storage).await?;
    let result = async {
        migrate_state(storage, &lease).await?;
        migrate_markers(storage, &lease).await
    }
    .await;
    let final_check = if result.is_ok() {
        lease.renew().await
    } else {
        Ok(())
    };
    let _ = lease.release().await;
    match (result, final_check) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

#[derive(Clone, Debug)]
struct OwnerLease {
    docs: Arc<dyn DocumentStore>,
    holder: String,
    epoch: u64,
    version: Arc<Mutex<Version>>,
}

impl OwnerLease {
    async fn acquire(storage: &Storage) -> tinystoragedrivers_types::Result<Self> {
        let docs = Arc::clone(storage.documents());
        docs.ensure_collection(&CollectionSpec::new(LEASES_COLLECTION))
            .await?;
        let seq = NEXT_HOLDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let holder = format!("{}-{}-{seq}", std::process::id(), now_ms());
        let id = Key::new(format!("{OWNER}/{LEASE_KEY}"))?;
        loop {
            let old = docs
                .get_as::<LeaseRecord>(LEASES_COLLECTION, id.as_str())
                .await?;
            let now = now_ms();
            let (epoch, condition) = match old {
                Some(old) if old.doc.expires_ms > now => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
                Some(old) => (
                    old.doc.epoch.checked_add(1).ok_or_else(|| {
                        StorageError::conflict("memes migration lease epoch exhausted")
                    })?,
                    Precondition::Version(old.version),
                ),
                None => (1, Precondition::Absent),
            };
            let expires_ms = now.saturating_add(LEASE_TTL_MS);
            match docs
                .put_as(
                    LEASES_COLLECTION,
                    id.as_str(),
                    &LeaseRecord {
                        holder: holder.clone(),
                        epoch,
                        expires_ms,
                    },
                    condition,
                )
                .await
            {
                Ok(version) => {
                    let current = docs
                        .get_as::<LeaseRecord>(LEASES_COLLECTION, id.as_str())
                        .await?
                        .ok_or_else(|| {
                            StorageError::conflict("memes migration lease disappeared")
                        })?;
                    if current.doc.holder != holder || current.doc.epoch != epoch {
                        return Err(StorageError::conflict(
                            "memes migration lease was taken over",
                        ));
                    }
                    return Ok(Self {
                        docs,
                        holder,
                        epoch,
                        version: Arc::new(Mutex::new(current.version.max(version))),
                    });
                }
                Err(error) if error.kind() == ErrorKind::Conflict => continue,
                Err(error) => return Err(error),
            }
        }
    }

    async fn renew(&self) -> tinystoragedrivers_types::Result<()> {
        let id = Key::new(format!("{OWNER}/{LEASE_KEY}"))?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let current = self
                .docs
                .get_as::<LeaseRecord>(LEASES_COLLECTION, id.as_str())
                .await?
                .ok_or_else(|| StorageError::conflict("memes migration lease disappeared"))?;
            if current.doc.holder != self.holder
                || current.doc.epoch != self.epoch
                || current.doc.expires_ms <= now_ms()
            {
                return Err(StorageError::conflict("memes migration lease was lost"));
            }
            let version = match self
                .docs
                .put_as(
                    LEASES_COLLECTION,
                    id.as_str(),
                    &LeaseRecord {
                        holder: self.holder.clone(),
                        epoch: self.epoch,
                        expires_ms: now_ms().saturating_add(LEASE_TTL_MS),
                    },
                    Precondition::Version(current.version),
                )
                .await
            {
                Ok(version) => version,
                Err(error) if error.kind() == ErrorKind::Conflict => continue,
                Err(error) => return Err(error),
            };
            *self.version.lock().await = version;
            return Ok(());
        }
        Err(StorageError::conflict(
            "memes migration lease changed repeatedly",
        ))
    }

    async fn mark_done(
        &self,
        key: &Key,
        step: &str,
        from: u32,
        to: u32,
    ) -> tinystoragedrivers_types::Result<()> {
        let progress = Progress {
            owner: OWNER.to_owned(),
            step: step.to_owned(),
            from,
            to,
            cursor: None,
            written: 0,
            done: true,
        };
        let body = serde_json::to_value(progress).map_err(|error| {
            StorageError::serialization(format!("encode memes migration progress: {error}"))
        })?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            self.renew().await?;
            let expected = *self.version.lock().await;
            if self
                .docs
                .capabilities()
                .contains(Capability::ValidatedBatch)
            {
                match self
                    .docs
                    .atomic_batch_checked(
                        vec![ReadCheck {
                            collection: LEASES_COLLECTION.to_owned(),
                            id: Key::new(format!("{OWNER}/{LEASE_KEY}"))?
                                .as_str()
                                .to_owned(),
                            expected: ExpectedDocument::Version(expected),
                        }],
                        vec![WriteOp::Put {
                            collection: MIGRATIONS_COLLECTION.to_owned(),
                            id: key.as_str().to_owned(),
                            doc: body.clone(),
                            precondition: Precondition::None,
                        }],
                    )
                    .await
                {
                    Ok(_) => return Ok(()),
                    Err(error) if error.kind() == ErrorKind::Conflict => continue,
                    Err(error) => return Err(error),
                }
            } else {
                self.renew().await?;
                self.docs
                    .put(
                        MIGRATIONS_COLLECTION,
                        key.as_str(),
                        body.clone(),
                        Precondition::None,
                    )
                    .await?;
                return Ok(());
            }
        }
        Err(StorageError::conflict(
            "memes migration lease changed before completion",
        ))
    }

    async fn release(&self) -> tinystoragedrivers_types::Result<()> {
        let id = Key::new(format!("{OWNER}/{LEASE_KEY}"))?;
        let Some(current) = self
            .docs
            .get_as::<LeaseRecord>(LEASES_COLLECTION, id.as_str())
            .await?
        else {
            return Ok(());
        };
        if current.doc.holder != self.holder || current.doc.epoch != self.epoch {
            return Ok(());
        }
        self.docs
            .put_as(
                LEASES_COLLECTION,
                id.as_str(),
                &LeaseRecord {
                    holder: self.holder.clone(),
                    epoch: self.epoch,
                    expires_ms: now_ms(),
                },
                Precondition::Version(current.version),
            )
            .await?;
        Ok(())
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
