use crate::config::env_parse;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: Args) -> Result<()> {
    let root = PathBuf::from(env_parse(
        "DISK_QUARANTINE_ROOT",
        String::from("/media/.cleanup-quarantine"),
    ));
    cleanup(&root, args.dry_run, chrono::Utc::now())
}

fn cleanup(root: &Path, dry_run: bool, now: chrono::DateTime<chrono::Utc>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let root = root.canonicalize()?;
    let cutoff = now - chrono::Duration::days(30);
    let mut removed = 0;
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("disk-cleanup-")
        {
            continue;
        }
        let journal = entry.path().join("moves.json");
        if !journal.is_file() || journal.symlink_metadata()?.file_type().is_symlink() {
            continue;
        }
        let manifest: Value = serde_json::from_slice(&std::fs::read(&journal)?)?;
        let created = chrono::DateTime::parse_from_rfc3339(
            manifest["created_at"]
                .as_str()
                .context("missing quarantine timestamp")?,
        )?;
        if created >= cutoff {
            continue;
        }
        let mut records = manifest["records"]
            .as_array()
            .context("missing quarantine records")?
            .clone();
        let outcomes = entry.path().join("outcomes.jsonl");
        if outcomes.is_file() && !outcomes.symlink_metadata()?.file_type().is_symlink() {
            let text = std::fs::read_to_string(&outcomes)?;
            for line in text
                .split_inclusive('\n')
                .filter(|line| line.ends_with('\n'))
            {
                let record: Value = serde_json::from_str(line)?;
                if let Some(existing) = records.iter_mut().find(|r| {
                    r["source_path"] == record["source_path"]
                        && r["quarantine_path"] == record["quarantine_path"]
                }) {
                    *existing = record;
                }
            }
        }
        for record in &records {
            if record["action"] != "quarantined" {
                continue;
            }
            let Some(at) = record["quarantined_at"].as_str() else {
                continue;
            };
            if chrono::DateTime::parse_from_rfc3339(at)? >= cutoff {
                continue;
            }
            let path = Path::new(
                record["quarantine_path"]
                    .as_str()
                    .context("missing quarantine path")?,
            );
            let Ok(md) = path.symlink_metadata() else {
                continue;
            };
            if !md.is_file()
                || !path
                    .canonicalize()?
                    .starts_with(entry.path().canonicalize()?)
            {
                continue;
            }
            if Some(md.len()) != record["size"].as_u64()
                || Some(md.modified()?.duration_since(UNIX_EPOCH)?.as_nanos())
                    != record["mtime_ns"].as_u64().map(u128::from)
            {
                continue;
            }
            if !dry_run {
                std::fs::remove_file(path)?;
            }
            removed += 1;
        }
    }
    println!(
        "{}",
        json!({"quarantine_files_expired":removed,"dry_run":dry_run,"retention_days":30})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_uses_move_time_and_preserves_unlisted_or_changed_files() {
        let root = std::env::temp_dir().join(format!("quarantine-test-{}", uuid::Uuid::new_v4()));
        let run = root.join("disk-cleanup-test");
        std::fs::create_dir_all(&run).unwrap();
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(31)).to_rfc3339();
        let mut records = Vec::new();
        for (name, at) in [
            ("expired.flac", old.clone()),
            ("recent.flac", now.to_rfc3339()),
            ("changed.flac", old.clone()),
        ] {
            let path = run.join(name);
            std::fs::write(&path, b"audio").unwrap();
            let md = path.metadata().unwrap();
            records.push(json!({"action":"quarantined","quarantine_path":path,"size":md.len(),"mtime_ns":md.modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos(),"quarantined_at":at}));
        }
        std::fs::write(run.join("unlisted.flac"), b"keep").unwrap();
        std::fs::write(run.join("changed.flac"), b"new contents").unwrap();
        std::fs::write(
            run.join("moves.json"),
            serde_json::to_vec(&json!({"created_at":old,"records":records})).unwrap(),
        )
        .unwrap();
        let mut pending: Value =
            serde_json::from_slice(&std::fs::read(run.join("moves.json")).unwrap()).unwrap();
        let mut outcomes = String::new();
        for record in pending["records"].as_array_mut().unwrap() {
            outcomes.push_str(&serde_json::to_string(record).unwrap());
            outcomes.push('\n');
            record["action"] = json!("pending");
        }
        outcomes.push_str("{incomplete final entry");
        std::fs::write(run.join("outcomes.jsonl"), outcomes).unwrap();
        std::fs::write(
            run.join("moves.json"),
            serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        cleanup(&root, true, now).unwrap();
        assert!(run.join("expired.flac").exists());
        cleanup(&root, false, now).unwrap();
        assert!(!run.join("expired.flac").exists());
        for name in ["recent.flac", "changed.flac", "unlisted.flac", "moves.json"] {
            assert!(run.join(name).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
