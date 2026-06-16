use std::{env, path::PathBuf, str::FromStr};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub lidarr_base_url: String,
    pub lidarr_header_value: String,
    pub cache_db: PathBuf,
    pub report_dir: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            lidarr_base_url: env_required("LIDARR_BASE_URL")?
                .trim_end_matches('/')
                .to_string(),
            lidarr_header_value: env_required("LIDARR_HEADER_VALUE")?,
            cache_db: PathBuf::from(env_parse(
                "CACHE_DB",
                String::from("/data/media-maintenance.sqlite"),
            )),
            report_dir: PathBuf::from(env_parse("REPORT_DIR", String::from("/data/reports"))),
        })
    }
}

pub fn env_required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing required env {name}"))
}

pub fn env_parse<T>(name: &str, default: T) -> T
where
    T: FromStr,
{
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
