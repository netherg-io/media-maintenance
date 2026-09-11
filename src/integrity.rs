use crate::config::{env_parse, env_required};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, path::Path, time::Duration};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub path: String,
    pub size: u64,
    pub mtime_ns: String,
    pub verdict: String,
    pub authenticity: String,
    pub message: String,
    pub authenticity_message: String,
    pub validator_version: String,
}

#[derive(Deserialize)]
struct Page {
    items: Vec<Evidence>,
    total: usize,
}

pub async fn fetch() -> Result<HashMap<String, Evidence>> {
    let Ok(base) = std::env::var("AUDIO_INTEGRITY_URL") else {
        return Ok(HashMap::new());
    };
    let mut headers = reqwest::header::HeaderMap::new();
    let mut token =
        reqwest::header::HeaderValue::from_str(&env_required("AUDIO_INTEGRITY_TOKEN")?)?;
    token.set_sensitive(true);
    headers.insert("x-integrity-token", token);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(60))
        .build()?;
    let base = base.trim_end_matches('/');
    if env_parse("AUDIO_INTEGRITY_SCAN", true) {
        let response = client
            .post(format!("{base}/api/scans"))
            .json(&serde_json::json!({"mode":"incremental"}))
            .send()
            .await?;
        if response.status() != reqwest::StatusCode::CONFLICT {
            response.error_for_status()?;
        }
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(env_parse("AUDIO_INTEGRITY_TIMEOUT_SECONDS", 21600));
        loop {
            let status: Value = client
                .get(format!("{base}/api/status"))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            match status["phase"].as_str() {
                Some("completed") => break,
                Some("discovering" | "scanning" | "cancelling") => {}
                _ => bail!("Audio Integrity scan did not complete: {}", status["phase"]),
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Audio Integrity scan timed out");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    let summary: Value = client
        .get(format!("{base}/api/summary"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = summary["validatorVersion"]
        .as_str()
        .context("missing validator version")?;
    let mut result = HashMap::new();
    for verdict in ["corrupt", "likely_lossy"] {
        let mut offset = 0;
        loop {
            let page: Page = client
                .get(format!("{base}/api/results"))
                .query(&[
                    ("verdict", verdict.to_owned()),
                    ("limit", "500".into()),
                    ("offset", offset.to_string()),
                ])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if page.items.is_empty() && offset < page.total {
                bail!("Incomplete integrity results");
            }
            offset += page.items.len();
            for item in page.items {
                if item.validator_version == version {
                    result.insert(item.path.clone(), item);
                }
            }
            if offset >= page.total {
                break;
            }
        }
    }
    Ok(result)
}

impl Evidence {
    pub fn matches(&self, path: &Path, size: u64, mtime_ns: u128) -> bool {
        Path::new(&self.path).is_absolute()
            && self.size == size
            && self.mtime_ns.parse::<u128>().ok() == Some(mtime_ns)
            && path.is_file()
    }
    pub fn category(&self) -> Option<&'static str> {
        if self.verdict == "corrupt" {
            Some("audio_corrupt")
        } else if self.verdict == "healthy" && self.authenticity == "likely_lossy" {
            Some("audio_likely_lossy")
        } else {
            None
        }
    }
}
