//! Resume metadata, persisted next to the download as `<file>.pincers`.
//! IDM keeps the same information in the registry (`ChunksList::InsertFromReg`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSegment {
    pub start: u64,
    /// First byte not yet downloaded.
    pub pos: u64,
    /// Exclusive end.
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    pub url: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: Vec<SavedSegment>,
}

pub fn state_path(target: &Path) -> PathBuf {
    let mut p = target.as_os_str().to_owned();
    p.push(".pincers");
    PathBuf::from(p)
}

pub fn part_path(target: &Path) -> PathBuf {
    let mut p = target.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

impl SavedState {
    pub fn load(target: &Path) -> Option<Self> {
        let data = std::fs::read(state_path(target)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    /// Atomic save: write a temp file, then rename over the old one.
    pub fn save(&self, target: &Path) -> Result<()> {
        let path = state_path(target);
        let tmp = path.with_extension("pincers.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn remove(target: &Path) {
        let _ = std::fs::remove_file(state_path(target));
    }
}
