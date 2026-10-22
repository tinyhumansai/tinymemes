//! Persistence port for the engine's learned state.

use crate::{MemeIndex, SlangIndex};
use std::collections::VecDeque;

/// Remixed-message ids remembered (oldest dropped first).
pub const REMIXED_CAP: usize = 4000;

/// The engine's learned-state persistence interface. Hosts provide the store;
/// TinyMemes does not select a storage engine or a fallback.
pub trait IndexStore: Send + Sync {
    /// The slang index, or an empty one.
    fn load_slang(&self) -> SlangIndex;
    /// The meme index, or an empty one.
    fn load_memes(&self) -> MemeIndex;
    /// Remixed ids, oldest first, at most [`REMIXED_CAP`].
    fn load_remixed(&self) -> VecDeque<String>;
    /// Persist the slang index.
    fn save_slang(&self, index: &SlangIndex) -> std::io::Result<()>;
    /// Persist the meme index.
    fn save_memes(&self, index: &MemeIndex) -> std::io::Result<()>;
    /// Persist the remixed ids (capped to [`REMIXED_CAP`] by the store).
    fn save_remixed(&self, ids: &VecDeque<String>) -> std::io::Result<()>;
}

/// Drop the oldest ids beyond [`REMIXED_CAP`].
pub fn cap_remixed(ids: &mut VecDeque<String>) {
    while ids.len() > REMIXED_CAP {
        ids.pop_front();
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
