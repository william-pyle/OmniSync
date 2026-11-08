//! Provider-independent, verified backups. Only the selected destination's
//! OmniSync directory is managed; the provider's cloud-upload status is unknown.
use crate::{model::*, staging};
use anyhow::{Context, Result};
use omnisync_windows::filesystem;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MARKER: &str = ".omnisync-destination.json";

fn guard_directories(path: &Path) -> Result<Vec<File>> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
    let mut current = PathBuf::new();
    let mut handles = vec![];
    for part in path.components() {
        current.push(part);
        if matches!(part, std::path::Component::Prefix(_)) {
            continue;
        }
        if !current.try_exists()? {
            std::fs::create_dir(&current)?;
        }
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&current)?;
        let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of_val(&tag) as u32,
            )?;
        }
        anyhow::ensure!(
            file.metadata()?.is_dir() && tag.ReparseTag & 0x20000000 == 0,
            "Destination contains a link or non-directory"
        );
        handles.push(file);
    }
    Ok(handles)
}

fn component(value: &str) -> Result<()> {
    let path = filesystem::safe_relative(value)?;
    anyhow::ensure!(path.components().count() == 1, "Invalid backup identifier");
    Ok(())
}

// Cloud placeholders are allowed here; links/junctions that redirect names are not.
pub fn check_path(path: &Path) -> Result<()> {
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        if matches!(part, std::path::Component::Prefix(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 => {
                let file = OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                    .open(&current)?;
                let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
                unsafe {
                    GetFileInformationByHandleEx(
                        HANDLE(file.as_raw_handle()),
                        FileAttributeTagInfo,
                        (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                        std::mem::size_of_val(&tag) as u32,
                    )?;
                }
                anyhow::ensure!(
                    tag.ReparseTag & 0x20000000 == 0,
                    "Backup destination cannot contain symbolic links or junctions"
                );
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Resolve a selection without initializing a destination or changing settings.
pub fn validate(path: &Path, config: &Config, state: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        path.is_absolute(),
        "Choose an absolute folder or network share path"
    );
    check_path(path)?;
    let path = std::fs::canonicalize(path)
        .context("Backup destination is unavailable. Connect the drive or network share first.")?;
    anyhow::ensure!(path.is_dir(), "Choose an existing destination folder");
    let state = std::fs::canonicalize(state).unwrap_or_else(|_| state.to_path_buf());
    for other in
        std::iter::once(state.as_path()).chain(config.roots.iter().map(|r| Path::new(&r.path)))
    {
        let resolved = std::fs::canonicalize(other).unwrap_or_else(|_| other.to_path_buf());
        anyhow::ensure!(!filesystem::contains(&resolved, &path) && !filesystem::contains(&path, &resolved),
            "The backup destination must be separate from all source folders and OmniSync's app data");
    }
    Ok(path)
}

pub fn select(path: &Path, config: &Config, state: &Path) -> Result<Destination> {
    let path = validate(path, config, state)?;
    let marker = path.join("OmniSync").join(MARKER);
    check_path(&marker)?;
    let identity = if marker.exists() {
        read_marker(&marker)?
    } else {
        id()
    };
    Ok(Destination {
        path: filesystem::display(&path),
        id: identity,
    })
}
fn read_marker(path: &Path) -> Result<String> {
    anyhow::ensure!(path.metadata()?.len() <= 1024, "Invalid destination marker");
    let value: serde_json::Value = serde_json::from_reader(File::open(path)?)?;
    let identity = value["id"].as_str().context("Invalid destination marker")?;
    uuid::Uuid::parse_str(identity).context("Invalid destination identity")?;
    anyhow::ensure!(
        value["schemaVersion"] == 1,
        "Unsupported destination format"
    );
    Ok(identity.into())
}

/// This is called only after the user applies a destination review.
pub fn initialize(destination: &Destination, config: &Config, state: &Path) -> Result<()> {
    let path = validate(Path::new(&destination.path), config, state)?.join("OmniSync");
    check_path(&path)?;
    let _handles = guard_directories(&path)?;
    let marker = path.join(MARKER);
    if !marker.exists() {
        let partial = path.join(format!(".{}.partial", id()));
        let result = (|| -> Result<()> {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)?;
            output.write_all(&serde_json::to_vec(
                &serde_json::json!({"schemaVersion":1,"id":destination.id}),
            )?)?;
            output.sync_all()?;
            drop(output);
            filesystem::move_new(&partial, &marker)
        })();
        let _ = std::fs::remove_file(partial);
        result?;
    }
    anyhow::ensure!(
        read_marker(&marker)? == destination.id,
        "Destination changed since review; select it again"
    );
    let probe = path.join(format!(".{}.probe", id()));
    let result = (|| -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        file.sync_all()?;
        Ok(())
    })();
    let _ = std::fs::remove_file(probe);
    result.context("Destination is not writable")
}

pub fn base(config: &Config) -> Result<PathBuf> {
    let destination = config
        .destination
        .as_ref()
        .context("Choose a backup destination to begin copying")?;
    let base = Path::new(&destination.path).join("OmniSync");
    check_path(&base)?;
    let marker = base.join(MARKER);
    check_path(&marker)?;
    anyhow::ensure!(read_marker(&marker).context("Destination unavailable or its identity marker is missing. Reconnect it or choose a destination again.")? == destination.id,
        "A different backup drive is mounted here. Choose the intended destination again.");
    Ok(std::fs::canonicalize(base)?)
}
pub fn reference(config: &Config, relative: &str) -> Result<String> {
    filesystem::safe_relative(relative)?;
    Ok(format!(
        "folder:{}:{relative}",
        config.destination.as_ref().context("No destination")?.id
    ))
}
pub fn resolve(config: &Config, reference: &str) -> Result<PathBuf> {
    let prefix = format!(
        "folder:{}:",
        config
            .destination
            .as_ref()
            .context("Choose a destination")?
            .id
    );
    let relative = reference.strip_prefix(&prefix).context(
        "This version belongs to another destination; choose that destination to restore it",
    )?;
    let path = base(config)?.join(filesystem::safe_relative(relative)?);
    check_path(&path)?;
    Ok(path)
}
pub fn latest_relative(config: &Config, root: &Root, item: &Item) -> Result<String> {
    component(&config.device_id)?;
    component(&root.id)?;
    // The stable ID prevents equally named sources from colliding. Labels are display-only.
    let name: String = root
        .label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let relative = format!(
        "{}/files/{}-{}/{}",
        config.device_id, name, root.id, item.relative_path
    );
    filesystem::safe_relative(&relative)?;
    Ok(relative)
}

/// Temp file, fsync, read-back hash, then rename on the destination filesystem.
/// Source files and existing snapshots are never removed by a failed transfer.
pub fn copy_verified(
    source: &Path,
    target: &Path,
    sha: &str,
    size: u64,
    replace: bool,
    mut checkpoint: impl FnMut(u64) -> Result<()>,
) -> Result<()> {
    check_path(target)?;
    let parent = target.parent().context("Invalid destination path")?;
    let _handles = guard_directories(parent)?;
    let partial = parent.join(format!(".omnisync-{}.partial", id()));
    let result = (|| -> Result<()> {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total = 0;
        loop {
            checkpoint(total)?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            total += count as u64;
            anyhow::ensure!(total <= size, "Snapshot size changed");
        }
        output.sync_all()?;
        drop(output);
        let (actual, _, length) = staging::hashes(File::open(&partial)?)?;
        anyhow::ensure!(
            actual == sha && length == size,
            "Copied bytes failed SHA-256 verification"
        );
        checkpoint(total)?;
        check_path(target)?;
        if replace {
            filesystem::atomic_replace(&partial, target)?;
        } else {
            filesystem::move_new(&partial, target)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&partial);
    result
}
pub fn matches(config: &Config, version: &Version) -> Result<bool> {
    let path = resolve(config, &version.remote_id)?;
    if !path.try_exists()? {
        return Ok(false);
    }
    let (sha, _, size) = staging::hashes(File::open(path)?)?;
    Ok(sha == version.sha256 && size == version.size)
}

pub fn save_catalog(config: &Config, catalog: &Catalog, state: &Path) -> Result<String> {
    component(&config.device_id)?;
    let relative = format!(
        "{}/catalogs/{}-{}.json",
        config.device_id,
        catalog.committed_at,
        id()
    );
    let reference = reference(config, &relative)?;
    let path = resolve(config, &reference)?;
    let bytes = serde_json::to_vec_pretty(catalog)?;
    let stage = state.join(format!("catalog-{}.tmp", id()));
    let result = (|| -> Result<()> {
        filesystem::write_durable(&stage, &bytes)?;
        let (sha, _, size) = staging::hashes(bytes.as_slice())?;
        copy_verified(&stage, &path, &sha, size, false, |_| Ok(()))
    })();
    let _ = std::fs::remove_file(stage);
    result?;
    Ok(reference)
}
pub fn catalogs(config: &Config) -> Result<Vec<serde_json::Value>> {
    let base = base(config)?;
    let mut result = vec![];
    for entry in std::fs::read_dir(&base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path().join("catalogs");
        check_path(&dir)?;
        if !dir.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(dir)? {
            let file = file?;
            if !file.file_type()?.is_file() || file.path().extension().is_none_or(|x| x != "json") {
                continue;
            }
            let relative = file
                .path()
                .strip_prefix(&base)?
                .to_string_lossy()
                .replace('\\', "/");
            result.push(serde_json::json!({"id":reference(config, &relative)?,"name":relative}));
        }
    }
    result.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
    Ok(result)
}
pub fn read_catalog(config: &Config, reference: &str) -> Result<Catalog> {
    let path = resolve(config, reference)?;
    anyhow::ensure!(
        path.metadata()?.len() <= 64 * 1024 * 1024,
        "Catalog exceeds 64 MiB"
    );
    let catalog: Catalog = serde_json::from_reader(File::open(path)?)?;
    anyhow::ensure!(catalog.schema_version == 2, "Not a folder-backup catalog");
    for item in &catalog.items {
        filesystem::safe_relative(&item.relative_path)?;
        for version in item.current.iter().chain(item.history.iter()) {
            resolve(config, &version.remote_id)?;
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_copy_and_hash_failure_keep_existing_target() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source");
        let target = t.path().join("target");
        std::fs::write(&source, b"new contents").unwrap();
        std::fs::write(&target, b"original").unwrap();
        let (sha, _, size) = staging::hashes(File::open(&source).unwrap()).unwrap();
        assert!(
            copy_verified(&source, &target, &sha, size, true, |_| anyhow::bail!(
                "Paused"
            ))
            .is_err()
        );
        assert!(copy_verified(&source, &target, &"0".repeat(64), size, true, |_| Ok(())).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 2);
    }
    #[test]
    fn overlaps_traversal_and_wrong_marker_are_rejected() {
        let t = tempfile::tempdir().unwrap();
        let state = t.path().join("state");
        let source = t.path().join("source");
        let dest = t.path().join("destination");
        for p in [&state, &source, &dest] {
            std::fs::create_dir(p).unwrap();
        }
        let mut c = Config::default();
        c.roots.push(Root {
            id: "root".into(),
            label: "Source".into(),
            path: filesystem::display(&source),
            volume_identity: String::new(),
            enabled: true,
            is_file: false,
        });
        assert!(validate(&source, &c, &state).is_err());
        assert!(validate(t.path(), &c, &state).is_err());
        let d = select(&dest, &c, &state).unwrap();
        initialize(&d, &c, &state).unwrap();
        c.destination = Some(d.clone());
        for reference in [
            format!("folder:{}:../escape", d.id),
            format!("folder:{}:C:/escape", d.id),
            "legacy-google-id".into(),
        ] {
            assert!(resolve(&c, &reference).is_err());
        }
        c.destination.as_mut().unwrap().id = id();
        assert!(base(&c).is_err());
    }
}
