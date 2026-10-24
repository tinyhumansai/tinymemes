//! The module's learned state, held as scoped documents the host stores.
//!
//! The engine's persistence port is [`IndexStore`]. Its methods are
//! synchronous, while the documents live behind async host callbacks, so a
//! request preloads the documents once ([`Documents::fetch`]), serves the
//! port's loads from that snapshot, collects the port's saves, and sends them
//! back with [`Documents::flush`]. The host stores opaque JSON text per
//! [`IndexKind`]; it never parses the ring, the cap or the index formats.

use std::collections::VecDeque;
use std::io;
use std::sync::{Mutex, PoisonError};

use tinymemes::store::cap_remixed;
use tinymemes::{IndexPolicy, IndexStore, MemeIndex, MemeIndexPolicy, SlangIndex};
use tinymemes_bus::IndexKind;

use crate::host::SharedHost;

/// Serializes the read-modify-write of the remixed ring across requests, so two
/// concurrent remixes cannot drop each other's id.
static RING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Default)]
struct Pending {
    slang: Option<String>,
    memes: Option<String>,
    remixed: Option<String>,
}

/// A request's snapshot of the three documents, as an [`IndexStore`].
pub(crate) struct Documents {
    slang: Option<String>,
    memes: Option<String>,
    remixed: Option<String>,
    pending: Mutex<Pending>,
}

async fn fetch_one(host: &SharedHost, kind: IndexKind) -> Option<String> {
    match host.load_index(kind).await {
        Ok(document) => document,
        Err(error) => {
            tracing::debug!(
                ?kind,
                ?error,
                "[tinymemes] index document unavailable; starting fresh"
            );
            None
        }
    }
}

impl Documents {
    /// Load every document the engine needs. A missing or unreadable document
    /// is a fresh start, as with the file-backed default.
    pub(crate) async fn fetch(host: &SharedHost) -> Self {
        let (slang, memes, remixed) = tokio::join!(
            fetch_one(host, IndexKind::Slang),
            fetch_one(host, IndexKind::Memes),
            fetch_one(host, IndexKind::Remixed),
        );
        Self {
            slang,
            memes,
            remixed,
            pending: Mutex::default(),
        }
    }

    /// A snapshot with fixed contents, no host involved.
    #[cfg(test)]
    pub(crate) fn from_parts(
        slang: Option<String>,
        memes: Option<String>,
        remixed: Option<String>,
    ) -> Self {
        Self {
            slang,
            memes,
            remixed,
            pending: Mutex::default(),
        }
    }

    /// Send every document the port saved since the last flush to the host.
    /// Best effort: a lost save only loses usage counts or a remixed marker.
    pub(crate) async fn flush(&self, host: &SharedHost) {
        let pending =
            std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        for (kind, json) in [
            (IndexKind::Slang, pending.slang),
            (IndexKind::Memes, pending.memes),
            (IndexKind::Remixed, pending.remixed),
        ] {
            if let Some(json) = json
                && let Err(error) = host.save_index(kind, json).await
            {
                tracing::debug!(?kind, ?error, "[tinymemes] index save failed");
            }
        }
    }

    fn set(&self, write: impl FnOnce(&mut Pending)) {
        write(&mut self.pending.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// Ids of messages delivered remixed, oldest first.
    pub(crate) fn remixed_ids(&self) -> VecDeque<String> {
        self.load_remixed()
    }

    /// Remember `id` as delivered remixed. The ring is re-read from the host
    /// inside a process-wide lock, so a concurrent remix's id is kept.
    pub(crate) async fn mark_remixed(host: &SharedHost, id: String) {
        let _guard = RING.lock().await;
        let fresh = Self {
            slang: None,
            memes: None,
            remixed: fetch_one(host, IndexKind::Remixed).await,
            pending: Mutex::default(),
        };
        let mut ring = fresh.load_remixed();
        if ring.contains(&id) {
            return;
        }
        ring.push_back(id);
        if fresh.save_remixed(&ring).is_ok() {
            fresh.flush(host).await;
        }
    }
}

impl IndexStore for Documents {
    fn load_slang(&self) -> SlangIndex {
        match &self.slang {
            Some(json) => SlangIndex::from_json(json, IndexPolicy::default()).unwrap_or_else(|e| {
                tracing::warn!("[tinymemes] slang index unreadable, starting fresh: {e}");
                SlangIndex::new(IndexPolicy::default())
            }),
            None => SlangIndex::new(IndexPolicy::default()),
        }
    }

    fn load_memes(&self) -> MemeIndex {
        let policy = MemeIndexPolicy::default();
        match &self.memes {
            Some(json) => MemeIndex::from_json(json, policy).unwrap_or_else(|e| {
                tracing::warn!("[tinymemes] meme index unreadable, starting fresh: {e}");
                MemeIndex::new(policy)
            }),
            None => MemeIndex::new(policy),
        }
    }

    fn load_remixed(&self) -> VecDeque<String> {
        let mut ids: VecDeque<String> = self
            .remixed
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        cap_remixed(&mut ids);
        ids
    }

    fn save_slang(&self, index: &SlangIndex) -> io::Result<()> {
        self.set(|p| p.slang = Some(index.to_json()));
        Ok(())
    }

    fn save_memes(&self, index: &MemeIndex) -> io::Result<()> {
        self.set(|p| p.memes = Some(index.to_json()));
        Ok(())
    }

    fn save_remixed(&self, ids: &VecDeque<String>) -> io::Result<()> {
        let mut capped = ids.clone();
        cap_remixed(&mut capped);
        let json = serde_json::to_string(&capped).map_err(io::Error::other)?;
        self.set(|p| p.remixed = Some(json));
        Ok(())
    }
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
