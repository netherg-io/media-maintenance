use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistCache {
    pub artist_id: i64,
    pub fingerprint: String,
    pub processed_at: String,
    pub status: String,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let store = Self {
            path: path.to_path_buf(),
            lock: Arc::new(Mutex::new(())),
        };
        if !store.path.exists() {
            store.write_all(&HashMap::new()).await?;
        }
        Ok(store)
    }

    pub async fn artist_cache(&self) -> Result<HashMap<i64, ArtistCache>> {
        self.read_all().await
    }

    pub async fn upsert_artist(&self, row: &ArtistCache) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut rows = self.read_all_unlocked().await?;
        rows.insert(row.artist_id, row.clone());
        self.write_all_unlocked(&rows).await
    }

    async fn read_all(&self) -> Result<HashMap<i64, ArtistCache>> {
        let _guard = self.lock.lock().await;
        self.read_all_unlocked().await
    }

    async fn read_all_unlocked(&self) -> Result<HashMap<i64, ArtistCache>> {
        let data = fs::read(&self.path).await.unwrap_or_default();
        if data.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(serde_json::from_slice(&data)?)
    }

    async fn write_all(&self, rows: &HashMap<i64, ArtistCache>) -> Result<()> {
        let _guard = self.lock.lock().await;
        self.write_all_unlocked(rows).await
    }

    async fn write_all_unlocked(&self, rows: &HashMap<i64, ArtistCache>) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(rows)?).await?;
        fs::rename(tmp, &self.path).await?;
        Ok(())
    }
}
