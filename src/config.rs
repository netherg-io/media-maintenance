use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub lidarr_base_url: String,
    pub lidarr_header_value: String,
    pub cache_db: PathBuf,
    pub report_dir: PathBuf,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            lidarr_base_url: std::env::var("LIDARR_BASE_URL").unwrap_or_else(|_| String::from("http://lidarr:8686/api/v1")),
            lidarr_header_value: std::env::var("LIDARR_HEADER_VALUE").unwrap_or_default(),
            cache_db: PathBuf::from(std::env::var("CACHE_DB").unwrap_or_else(|_| String::from("/data/media-maintenance.sqlite"))),
            report_dir: PathBuf::from(std::env::var("REPORT_DIR").unwrap_or_else(|_| String::from("/data/reports"))),
        })
    }
}

pub fn env_parse<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}
