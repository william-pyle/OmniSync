use crate::model::*;
use anyhow::{bail, Context, Result};
use md5::Md5;
use omnisync_windows::filesystem;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::UNIX_EPOCH,
};
pub fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}
pub fn hashes(mut reader: impl Read) -> Result<(String, String, u64)> {
    let mut sha = Sha256::new();
    let mut md5 = Md5::new();
    let mut bytes = [0u8; 1024 * 1024];
    let mut size = 0;
    loop {
        let n = reader.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        sha.update(&bytes[..n]);
        md5.update(&bytes[..n]);
        size += n as u64;
    }
    Ok((
        hex::encode(sha.finalize()),
        hex::encode(md5.finalize()),
        size,
    ))
}
pub fn capture(root: &Root, item: &Item, state: &Path, config: &Config) -> Result<Job> {
    let (_, identity, _) = filesystem::validate_root(Path::new(&root.path), state)?;
    anyhow::ensure!(
        identity == root.volume_identity,
        "Backup root identity changed before capture"
    );
    let source = if root.is_file {
        std::path::PathBuf::from(&root.path)
    } else {
        Path::new(&root.path).join(filesystem::safe_relative(&item.relative_path)?)
    };
    let mut input = filesystem::locked_source(&source, Path::new(&root.path))?;
    let before = input.metadata()?;
    let stage_dir = state.join("staging");
    std::fs::create_dir_all(&stage_dir)?;
    let used = std::fs::read_dir(&stage_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum::<u64>();
    if before.len() > config.staging_budget_bytes.saturating_sub(used)
        || filesystem::free_space(&stage_dir)?
            < before.len().saturating_add(config.free_space_reserve_bytes)
    {
        bail!("Staging space needed. Increase the budget or free disk space.");
    }
    let job_id = id();
    let stage = stage_dir.join(format!("{job_id}.blob"));
    let partial = stage.with_extension("partial");
    let result = (|| -> Result<Job> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        let mut sha = Sha256::new();
        let mut md5 = Md5::new();
        let mut bytes = vec![0; 1024 * 1024];
        let mut size = 0;
        loop {
            let n = input
                .read(&mut bytes)
                .context("Source changed or became unreadable")?;
            if n == 0 {
                break;
            }
            output.write_all(&bytes[..n])?;
            sha.update(&bytes[..n]);
            md5.update(&bytes[..n]);
            size += n as u64;
        }
        output.sync_all()?;
        drop(output);
        let after = input.metadata()?;
        if before.len() != after.len()
            || modified(&before) != modified(&after)
            || size != before.len()
        {
            bail!("Source changed during snapshot capture");
        }
        std::fs::rename(&partial, &stage)?;
        Ok(Job {
            id: job_id,
            item_id: item.id.clone(),
            generation: item.generation,
            config_revision: config.revision,
            stage: stage.to_string_lossy().into(),
            size,
            sha256: hex::encode(sha.finalize()),
            md5: hex::encode(md5.finalize()),
            target_id: None,
            parent_id: None,
            history_id: None,
            history_saved: false,
            session: None,
            offset: 0,
            phase: "staged".into(),
            attempts: 0,
            retry_at: 0,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}
pub fn verify_stage(job: &Job, state: &Path) -> Result<()> {
    let path = Path::new(&job.stage);
    anyhow::ensure!(
        filesystem::contains(&state.join("staging"), path),
        "Invalid staging reference"
    );
    let (sha, _, size) = hashes(File::open(path)?)?;
    anyhow::ensure!(
        sha == job.sha256 && size == job.size,
        "Staging snapshot integrity failed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Root, Item, Config) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("example.txt"), b"first contents").unwrap();
        let (path, identity, _) =
            filesystem::validate_root(&source, &temp.path().join("state")).unwrap();
        let root = Root {
            id: "root".into(),
            label: "Example".into(),
            path: filesystem::display(&path),
            volume_identity: identity,
            enabled: true,
            is_file: false,
        };
        let item = Item {
            id: "item".into(),
            root_id: "root".into(),
            relative_path: "example.txt".into(),
            size: 14,
            modified: 0,
            generation: 1,
            status: Status::Pending,
            reason: String::new(),
            winning_rule: None,
            current: None,
            history: vec![],
            last_seen: 0,
        };
        let config = Config {
            free_space_reserve_bytes: 1024 * 1024,
            ..Default::default()
        };
        (temp, root, item, config)
    }
    #[test]
    fn snapshot_is_immutable_and_corruption_is_rejected() {
        let (temp, root, item, config) = fixture();
        let state = temp.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let job = capture(&root, &item, &state, &config).unwrap();
        std::fs::write(Path::new(&root.path).join("example.txt"), b"second version").unwrap();
        assert_eq!(std::fs::read(&job.stage).unwrap(), b"first contents");
        verify_stage(&job, &state).unwrap();
        std::fs::write(&job.stage, b"tampered").unwrap();
        assert!(verify_stage(&job, &state).is_err());
    }
    #[test]
    fn insufficient_budget_creates_no_snapshot() {
        let (temp, root, item, mut config) = fixture();
        let state = temp.path().join("state");
        std::fs::create_dir(&state).unwrap();
        config.staging_budget_bytes = 1;
        assert!(capture(&root, &item, &state, &config).is_err());
        assert_eq!(std::fs::read_dir(state.join("staging")).unwrap().count(), 0);
    }
    #[test]
    fn active_writer_prevents_capture() {
        let (temp, root, item, config) = fixture();
        let state = temp.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let _writer = OpenOptions::new()
            .write(true)
            .open(Path::new(&root.path).join("example.txt"))
            .unwrap();
        assert!(capture(&root, &item, &state, &config).is_err());
    }
    #[test]
    fn empty_hashes_match_standard_vectors() {
        let (sha, md5, size) = hashes(&b""[..]).unwrap();
        assert_eq!(size, 0);
        assert_eq!(
            sha,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(md5, "d41d8cd98f00b204e9800998ecf8427e");
    }
}
