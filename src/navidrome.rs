use crate::config::env_required;
use anyhow::{bail, Result};
use serde_json::Value;
use std::time::Duration;

pub async fn rescan() -> Result<Value> {
    let base = env_required("NAVIDROME_URL")?;
    let user = env_required("NAVIDROME_USER")?;
    let password = env_required("NAVIDROME_PASSWORD")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let call = |method: &str| {
        client
            .post(format!("{}/rest/{method}", base.trim_end_matches('/')))
            .form(&[
                ("u", user.as_str()),
                ("p", password.as_str()),
                ("v", "1.16.1"),
                ("c", "media-maintenance"),
                ("f", "json"),
                ("fullScan", "true"),
            ])
    };
    let mut value: Value = call("startScan")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3600);
    loop {
        let response = &value["subsonic-response"];
        if response["status"] != "ok" {
            bail!("Navidrome rescan failed: {}", response["error"]);
        }
        if response["scanStatus"]["scanning"] == false {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("Navidrome scan timed out");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        value = call("getScanStatus")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
    }
}
