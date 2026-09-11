use crate::config::{env_parse, env_required};
use anyhow::{bail, Context, Result};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Stage {
    Integrity,
    Navidrome,
    Audiomuse,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Integrity => "integrity",
            Self::Navidrome => "navidrome",
            Self::Audiomuse => "audiomuse",
        }
    }
}

#[derive(Debug, Args)]
pub struct RefreshArgs {
    #[arg(value_enum)]
    stage: Stage,
    #[arg(long)]
    navidrome_scanned: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    run_id: String,
    changed_at: String,
    #[serde(default)]
    cleaning_done: bool,
    #[serde(default)]
    cleaning_job: Option<String>,
    #[serde(default)]
    analysis_job: Option<String>,
}

fn marker(dir: &Path, stage: Stage) -> PathBuf {
    dir.join(format!("pending-{}.json", stage.name()))
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().context("missing parent directory")?)?.sync_all()?;
    Ok(())
}

pub fn mark_changed(dir: &Path, run_id: &str, changed: bool) -> Result<()> {
    if !changed {
        return Ok(());
    }
    let stages = [
        (
            Stage::Integrity,
            std::env::var("AUDIO_INTEGRITY_URL").is_ok(),
        ),
        (
            Stage::Navidrome,
            env_parse("NAVIDROME_EXTERNAL_RESCAN", false),
        ),
        (Stage::Audiomuse, std::env::var("AUDIOMUSE_URL").is_ok()),
    ];
    for (stage, enabled) in stages {
        if enabled {
            mark_stage(dir, run_id, stage)?;
        }
    }
    Ok(())
}

fn mark_stage(dir: &Path, run_id: &str, stage: Stage) -> Result<()> {
    fs::create_dir_all(dir)?;
    save(
        &marker(dir, stage),
        &Pending {
            run_id: run_id.into(),
            changed_at: crate::report::now_iso(),
            cleaning_done: false,
            cleaning_job: None,
            analysis_job: None,
        },
    )
}

pub async fn run(args: RefreshArgs) -> Result<()> {
    let dir = PathBuf::from(env_parse("REPORT_DIR", String::from("/data/reports")));
    let path = marker(&dir, args.stage);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "{}",
                json!({"stage":args.stage.name(),"status":"unchanged"})
            );
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let mut pending: Pending = serde_json::from_slice(&bytes)?;
    let response = match args.stage {
        Stage::Integrity => crate::integrity::refresh(&pending.changed_at).await?,
        Stage::Navidrome => {
            if !args.navidrome_scanned {
                bail!("Confirm a successful external scan with --navidrome-scanned");
            }
            json!({"status":"completed"})
        }
        Stage::Audiomuse => {
            if marker(&dir, Stage::Navidrome).exists() || marker(&dir, Stage::Integrity).exists() {
                bail!("Integrity and Navidrome refresh must finish before AudioMuse");
            }
            let client = audiomuse_client()?;
            let base = env_required("AUDIOMUSE_URL")?;
            let result = advance_audiomuse(&client, base.trim_end_matches('/'), &mut pending).await;
            save(&path, &pending)?;
            if !result? {
                println!(
                    "{}",
                    json!({"stage":"audiomuse","status":"pending","cleaning_job":pending.cleaning_job,"analysis_job":pending.analysis_job})
                );
                return Ok(());
            }
            json!({"status":"completed","cleaning_job":pending.cleaning_job,"analysis_job":pending.analysis_job})
        }
    };
    let report = json!({"stage":args.stage.name(),"run_id":pending.run_id,"changed_at":pending.changed_at,"response":response});
    save(
        &dir.join(format!(
            "refresh-{}-{}.json",
            args.stage.name(),
            pending.run_id
        )),
        &report,
    )?;
    fs::remove_file(&path)?;
    fs::File::open(&dir)?.sync_all()?;
    println!("{report}");
    Ok(())
}

fn audiomuse_client() -> Result<reqwest::Client> {
    let token = env_required("AUDIOMUSE_TOKEN")?;
    let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?;
    header.set_sensitive(true);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, header);
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(60))
        .build()?)
}

enum JobState {
    Running,
    Finished,
    Failed(String),
}

async fn job_finished(client: &reqwest::Client, base: &str, id: &str) -> Result<JobState> {
    let status: Value = client
        .get(format!("{base}/api/status/{}", urlencoding::encode(id)))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    match status["state"].as_str() {
        Some("SUCCESS" | "FINISHED" | "finished") => Ok(JobState::Finished),
        Some("NEW" | "PENDING" | "STARTED" | "PROGRESS" | "RUNNING" | "QUEUED" | "queued") => {
            Ok(JobState::Running)
        }
        _ => Ok(JobState::Failed(format!(
            "AudioMuse job {id} did not succeed: {}",
            status["state"]
        ))),
    }
}

async fn enqueue(
    client: &reqwest::Client,
    base: &str,
    kind: &str,
    body: Value,
) -> Result<Option<String>> {
    let response = client
        .post(format!("{base}/api/{kind}/start"))
        .json(&body)
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::CONFLICT {
        return Ok(None);
    }
    let result: Value = response.error_for_status()?.json().await?;
    Ok(Some(
        result["task_id"]
            .as_str()
            .context("AudioMuse did not return a task ID")?
            .into(),
    ))
}

async fn advance_audiomuse(
    client: &reqwest::Client,
    base: &str,
    pending: &mut Pending,
) -> Result<bool> {
    if !pending.cleaning_done {
        if let Some(id) = &pending.cleaning_job {
            match job_finished(client, base, id).await? {
                JobState::Running => return Ok(false),
                JobState::Finished => pending.cleaning_done = true,
                JobState::Failed(error) => {
                    pending.cleaning_job = None;
                    bail!(error);
                }
            }
        } else {
            pending.cleaning_job =
                enqueue(client, base, "cleaning", json!({"clean_catalogue":true})).await?;
            return Ok(false);
        }
    }
    if let Some(id) = &pending.analysis_job {
        return match job_finished(client, base, id).await? {
            JobState::Running => Ok(false),
            JobState::Finished => Ok(true),
            JobState::Failed(error) => {
                pending.analysis_job = None;
                bail!(error);
            }
        };
    }
    pending.analysis_job =
        enqueue(client, base, "analysis", json!({"num_recent_albums":0})).await?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read},
        net::TcpListener,
        thread,
    };

    fn pending() -> Pending {
        Pending {
            run_id: "test".into(),
            changed_at: "2026-09-11T12:00:00Z".into(),
            cleaning_done: false,
            cleaning_job: None,
            analysis_job: None,
        }
    }

    fn mock(responses: Vec<(&'static str, u16, Value)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            for (expected, code, body) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                assert!(first.starts_with(expected), "unexpected request: {first}");
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                }
                let mut input = vec![0; length];
                reader.read_exact(&mut input).unwrap();
                if expected.contains("analysis/start") {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&input).unwrap()["num_recent_albums"],
                        0
                    );
                }
                if expected.contains("cleaning/start") {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&input).unwrap()["clean_catalogue"],
                        true
                    );
                }
                let body = body.to_string();
                write!(socket,"HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        (base, handle)
    }

    #[tokio::test]
    async fn audiomuse_waits_for_cleaning_and_does_not_duplicate_jobs() {
        let (base, worker) = mock(vec![
            ("POST /api/cleaning/start", 202, json!({"task_id":"clean"})),
            ("GET /api/status/clean", 200, json!({"state":"NEW"})),
            ("GET /api/status/clean", 200, json!({"state":"SUCCESS"})),
            (
                "POST /api/analysis/start",
                202,
                json!({"task_id":"analysis"}),
            ),
            ("GET /api/status/analysis", 200, json!({"state":"RUNNING"})),
            ("GET /api/status/analysis", 200, json!({"state":"SUCCESS"})),
        ]);
        let client = reqwest::Client::new();
        let mut state = pending();
        for _ in 0..4 {
            assert!(!advance_audiomuse(&client, &base, &mut state).await.unwrap());
        }
        assert!(advance_audiomuse(&client, &base, &mut state).await.unwrap());
        assert!(state.cleaning_done);
        assert_eq!(state.cleaning_job.as_deref(), Some("clean"));
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn busy_audiomuse_preserves_pending_work_without_cancelling_active_task() {
        let (base, worker) = mock(vec![(
            "POST /api/cleaning/start",
            409,
            json!({"task_id":"user-analysis"}),
        )]);
        let mut state = pending();
        assert!(
            !advance_audiomuse(&reqwest::Client::new(), &base, &mut state)
                .await
                .unwrap()
        );
        assert!(state.cleaning_job.is_none());
        assert!(!state.cleaning_done);
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn failed_analysis_can_retry_without_repeating_successful_cleaning() {
        let (base, worker) = mock(vec![(
            "GET /api/status/analysis",
            200,
            json!({"state":"FAIL"}),
        )]);
        let mut state = pending();
        state.cleaning_done = true;
        state.analysis_job = Some("analysis".into());
        assert!(
            advance_audiomuse(&reqwest::Client::new(), &base, &mut state)
                .await
                .is_err()
        );
        assert!(state.analysis_job.is_none());
        assert!(state.cleaning_done);
        worker.join().unwrap();
    }

    #[test]
    fn unchanged_runs_do_not_create_or_erase_pending_refreshes() {
        let dir = std::env::temp_dir().join(format!("refresh-{}", uuid::Uuid::new_v4()));
        mark_changed(&dir, "noop", false).unwrap();
        assert!(!dir.exists());
        mark_stage(&dir, "changed", Stage::Integrity).unwrap();
        let before = fs::read(marker(&dir, Stage::Integrity)).unwrap();
        mark_changed(&dir, "noop", false).unwrap();
        assert_eq!(before, fs::read(marker(&dir, Stage::Integrity)).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}
