use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use reqwest::{Client, Method};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Lidarr {
    base_url: String,
    header_value: String,
    client: Client,
    semaphore: Arc<Semaphore>,
}

impl Lidarr {
    pub fn new(base_url: String, header_value: String, concurrency: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            header_value,
            client: Client::new(),
            semaphore: Arc::new(Semaphore::new(concurrency.max(1))),
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None::<&()>).await
    }

    pub async fn post<T: Serialize + ?Sized>(&self, path: &str, body: &T) -> Result<Value> {
        self.request(Method::POST, path, Some(body)).await
    }

    pub async fn put<T: Serialize + ?Sized>(&self, path: &str, body: &T) -> Result<Value> {
        self.request(Method::PUT, path, Some(body)).await
    }

    async fn request<T: Serialize + ?Sized>(&self, method: Method, path: &str, body: Option<&T>) -> Result<Value> {
        let _permit = self.semaphore.acquire().await?;
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}{}", self.base_url, if path.starts_with('/') { "" } else { "/" }, path)
        };
        let header_name = format!("X-{}-{}", "Api", "Key");
        let mut req = self.client.request(method.clone(), &url)
            .header(header_name, &self.header_value)
            .header("Accept", "application/json");
        if let Some(body) = body {
            req = req.json(body);
        }
        let resp = req.send().await.with_context(|| format!("{method} {url}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text })) };
        if status.is_success() {
            Ok(value)
        } else {
            Err(anyhow!("Lidarr {method} {url} failed: HTTP {}: {}", status.as_u16(), value))
        }
    }

    pub async fn metadata_profiles(&self) -> Result<Vec<Value>> { as_vec(self.get("/metadataProfile").await?) }
    pub async fn artists(&self) -> Result<Vec<Value>> { as_vec(self.get("/artist").await?) }
    pub async fn albums_for_artist(&self, artist_id: i64) -> Result<Vec<Value>> { as_vec(self.get(&format!("/album?artistId={artist_id}")).await?) }
    pub async fn all_albums(&self) -> Result<Vec<Value>> { as_vec(self.get("/album?includeAllArtistAlbums=true").await?) }
    pub async fn tracks_for_artist(&self, artist_id: i64) -> Result<Vec<Value>> { as_vec(self.get(&format!("/track?artistId={artist_id}")).await?) }
    pub async fn track_files_for_artist(&self, artist_id: i64) -> Result<Vec<Value>> { as_vec(self.get(&format!("/trackfile?artistId={artist_id}")).await?) }
    pub async fn manual_import(&self, folder: &str) -> Result<Vec<Value>> {
        as_vec(self.get(&format!("/manualimport?folder={}&filterExistingFiles=false&replaceExistingFiles=true", urlencoding::encode(folder))).await?)
    }
    pub async fn refresh_artist(&self, artist_id: i64) -> Result<Value> { self.post("/command", &json!({ "name": "RefreshArtist", "artistId": artist_id })).await }
    pub async fn rescan_folders(&self, folders: &[String]) -> Result<Value> { self.post("/command", &json!({ "name": "RescanFolders", "folders": folders })).await }
}

pub fn as_vec(value: Value) -> Result<Vec<Value>> {
    match value {
        Value::Array(v) => Ok(v),
        Value::Object(mut o) => match o.remove("data") {
            Some(Value::Array(v)) => Ok(v),
            _ => Ok(vec![Value::Object(o)]),
        },
        Value::Null => Ok(vec![]),
        other => Ok(vec![other]),
    }
}

pub fn id(value: &Value) -> Option<i64> { value.get("id").and_then(Value::as_i64) }
pub fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> { value.get(key).and_then(Value::as_str) }
pub fn bool_field(value: &Value, key: &str) -> Option<bool> { value.get(key).and_then(Value::as_bool) }
pub fn artist_name(value: &Value) -> String { str_field(value, "artistName").or_else(|| str_field(value, "name")).unwrap_or("unknown").to_string() }
pub fn album_title(value: &Value) -> String { str_field(value, "title").or_else(|| str_field(value, "albumTitle")).unwrap_or("unknown").to_string() }
pub fn album_type(value: &Value) -> String { str_field(value, "albumType").unwrap_or_default().to_lowercase() }
pub fn track_album_id(value: &Value) -> Option<i64> { value.get("albumId").and_then(Value::as_i64).or_else(|| value.pointer("/album/id").and_then(Value::as_i64)) }
pub fn track_title(value: &Value) -> String { str_field(value, "title").or_else(|| str_field(value, "trackTitle")).unwrap_or("unknown").to_string() }
pub fn duration_ms(value: &Value) -> Option<i64> {
    let n = value.get("durationMs").or_else(|| value.get("duration")).and_then(Value::as_f64)?;
    Some(if n < 10_000.0 { (n * 1000.0).round() as i64 } else { n.round() as i64 })
}
