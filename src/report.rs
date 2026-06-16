use std::{path::Path, time::SystemTime};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::fs;
use uuid::Uuid;

pub fn now_iso() -> String {
    let now: DateTime<Utc> = SystemTime::now().into();
    now.to_rfc3339()
}

pub fn run_id(prefix: &str) -> String {
    format!(
        "{}-{}-{}",
        prefix,
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        Uuid::new_v4().simple()
    )
}

pub async fn write_json_report<T: Serialize>(
    dir: &Path,
    prefix: &str,
    run_id: &str,
    value: &T,
) -> Result<std::path::PathBuf> {
    fs::create_dir_all(dir)
        .await
        .with_context(|| format!("create report dir {}", dir.display()))?;
    let path = dir.join(format!("{prefix}-{run_id}.json"));
    fs::write(&path, serde_json::to_vec_pretty(value)?)
        .await
        .with_context(|| format!("write report {}", path.display()))?;
    Ok(path)
}
