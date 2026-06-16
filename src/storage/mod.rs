use std::{collections::HashMap, path::Path};

use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

#[derive(Debug, Clone)]
pub struct ArtistCache {
    pub artist_id: i64,
    pub fingerprint: String,
    pub processed_at: String,
    pub status: String,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "create table if not exists artist_cache(
                artist_id integer primary key,
                fingerprint text not null,
                processed_at text not null,
                status text not null
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn artist_cache(&self) -> Result<HashMap<i64, ArtistCache>> {
        let rows = sqlx::query_as::<_, (i64, String, String, String)>(
            "select artist_id, fingerprint, processed_at, status from artist_cache",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(artist_id, fingerprint, processed_at, status)| {
                (artist_id, ArtistCache { artist_id, fingerprint, processed_at, status })
            })
            .collect())
    }

    pub async fn upsert_artist(&self, row: &ArtistCache) -> Result<()> {
        sqlx::query(
            "insert into artist_cache(artist_id, fingerprint, processed_at, status)
             values (?, ?, ?, ?)
             on conflict(artist_id) do update set
             fingerprint=excluded.fingerprint,
             processed_at=excluded.processed_at,
             status=excluded.status",
        )
        .bind(row.artist_id)
        .bind(&row.fingerprint)
        .bind(&row.processed_at)
        .bind(&row.status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
