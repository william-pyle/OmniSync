//! Machine-wide Explorer registration. Removal is limited to reviewed overlay keys.
use crate::integration::{BADGES, CLSIDS, COMMAND_CLSIDS, OVERLAY_KEY};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use windows::Win32::{
    Security::*,
    System::Registry::{RegGetKeySecurity, RegSetKeySecurity, HKEY},
};
use winreg::{enums::*, RegKey, RegValue};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Value {
    pub kind: u32,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tree {
    pub values: BTreeMap<String, Value>,
    pub children: BTreeMap<String, Tree>,
    pub security: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub name: String,
    pub clsid: String,
    pub dll: String,
    pub eligible: bool,
    pub reason: String,
    pub digest: String,
    pub tree: Tree,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub id: String,
    pub entries: Vec<Registration>,
    pub removed: Vec<String>,
    pub restored: Vec<String>,
    pub complete: bool,
}
fn snapshot(key: &RegKey) -> Result<Tree> {
    let mut values = BTreeMap::new();
    for value in key.enum_values() {
        let (name, value) = value?;
        values.insert(
            name,
            Value {
                kind: value.vtype as u32,
                bytes: value.bytes,
            },
        );
    }
    let mut children = BTreeMap::new();
    for name in key.enum_keys() {
        let name = name?;
        children.insert(
            name.clone(),
            snapshot(&key.open_subkey_with_flags(&name, KEY_READ | KEY_WOW64_64KEY)?)?,
        );
    }
    let mut size = 0;
    unsafe {
        let info =
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION;
        let _ = RegGetKeySecurity(HKEY(key.raw_handle() as *mut _), info, None, &mut size);
        anyhow::ensure!(
            size > 0 && size < 1024 * 1024,
            "Invalid registry security descriptor"
        );
        let mut bytes = vec![0u8; size as usize];
        RegGetKeySecurity(
            HKEY(key.raw_handle() as *mut _),
            info,
            Some(PSECURITY_DESCRIPTOR(bytes.as_mut_ptr().cast())),
            &mut size,
        )
        .ok()?;
        Ok(Tree {
            values,
            children,
            security: bytes,
        })
    }
}
fn value_type(kind: u32) -> Result<winreg::enums::RegType> {
    use winreg::enums::RegType::*;
    Ok(match kind {
        0 => REG_NONE,
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        3 => REG_BINARY,
        4 => REG_DWORD,
        5 => REG_DWORD_BIG_ENDIAN,
        6 => REG_LINK,
        7 => REG_MULTI_SZ,
        8 => REG_RESOURCE_LIST,
        9 => REG_FULL_RESOURCE_DESCRIPTOR,
        10 => REG_RESOURCE_REQUIREMENTS_LIST,
        11 => REG_QWORD,
        _ => anyhow::bail!("Unsupported registry value type"),
    })
}
fn restore_tree(key: &RegKey, tree: &Tree) -> Result<()> {
    for (name, value) in &tree.values {
        key.set_raw_value(
            name,
            &RegValue {
                bytes: value.bytes.clone(),
                vtype: value_type(value.kind)?,
            },
        )?;
    }
    for (name, child) in &tree.children {
        restore_tree(
            &key.create_subkey_with_flags(name, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?
                .0,
            child,
        )?;
    }
    unsafe {
        RegSetKeySecurity(
            HKEY(key.raw_handle() as *mut _),
            DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR(tree.security.as_ptr() as *mut _),
        )
        .ok()?;
    }
    Ok(())
}
fn digest(tree: &Tree) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(tree)?)))
}
pub fn inventory() -> Result<Vec<Registration>> {
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let root = match machine.open_subkey_with_flags(OVERLAY_KEY, KEY_READ | KEY_WOW64_64KEY) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut result = vec![];
    for name in root.enum_keys() {
        let name = name?;
        let key = root.open_subkey_with_flags(&name, KEY_READ | KEY_WOW64_64KEY)?;
        let clsid: String = key.get_value("").unwrap_or_default();
        let dll = machine
            .open_subkey_with_flags(
                format!(r"SOFTWARE\Classes\CLSID\{clsid}\InprocServer32"),
                KEY_READ | KEY_WOW64_64KEY,
            )
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .unwrap_or_default();
        let own = CLSIDS.contains(&clsid.as_str());
        let protected = dll.is_empty()
            || dll.to_lowercase().contains("%systemroot%")
            || dll.to_lowercase().contains("%windir%")
            || std::env::var("WINDIR").ok().is_some_and(|w| {
                crate::filesystem::contains(Path::new(&w), Path::new(dll.trim_matches('"')))
            });
        let eligible = !own && !protected;
        let reason = if own {
            "OmniSync registration"
        } else if protected {
            "Protected or unresolved provider; cannot be taken over"
        } else {
            "May be disabled after explicit review; its sync application keeps running"
        };
        let tree = snapshot(&key)?;
        result.push(Registration {
            name,
            clsid,
            dll,
            eligible,
            reason: reason.into(),
            digest: digest(&tree)?,
            tree,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
pub fn receipt_directory() -> Result<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os("ProgramData").context("ProgramData is unavailable")?)
            .join("OmniSync")
            .join("OverlayBackups");
    crate::security::private_directory(&path, true)?;
    Ok(path)
}
fn save_receipt(path: &Path, receipt: &Receipt) -> Result<()> {
    let temp = path.with_extension("tmp");
    crate::filesystem::write_durable(&temp, &serde_json::to_vec_pretty(receipt)?)?;
    crate::filesystem::atomic_replace(&temp, path)
}
pub fn take_over(selected: &[(String, String)]) -> Result<PathBuf> {
    anyhow::ensure!(
        !selected.is_empty() && selected.len() <= 64,
        "Select between 1 and 64 overlay registrations"
    );
    let all = inventory()?;
    let mut entries = vec![];
    for (name, expected) in selected {
        let item = all
            .iter()
            .find(|i| &i.name == name)
            .context("Selected registration disappeared")?;
        anyhow::ensure!(
            item.eligible && &item.digest == expected,
            "Selected provider changed or is protected; review again"
        );
        entries.push(item.clone());
    }
    let directory = receipt_directory()?;
    let id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        std::process::id()
    );
    let path = directory.join(format!("{id}.json"));
    let mut receipt = Receipt {
        version: 1,
        id,
        entries,
        removed: vec![],
        restored: vec![],
        complete: false,
    };
    save_receipt(&path, &receipt)?;
    let root = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(OVERLAY_KEY, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?;
    let result = (|| -> Result<()> {
        for entry in &receipt.entries {
            let current =
                snapshot(&root.open_subkey_with_flags(&entry.name, KEY_READ | KEY_WOW64_64KEY)?)?;
            anyhow::ensure!(
                digest(&current)? == entry.digest,
                "A provider registration changed during review"
            );
            root.delete_subkey_all(&entry.name)?;
            receipt.removed.push(entry.name.clone());
            save_receipt(&path, &receipt)?;
        }
        receipt.complete = true;
        save_receipt(&path, &receipt)
    })();
    if let Err(error) = result {
        let rollback = restore_receipt(&path);
        return Err(error.context(format!(
            "Takeover stopped. Rollback: {}. Recovery receipt: {}",
            if rollback.is_ok() {
                "completed"
            } else {
                "needs review"
            },
            path.display()
        )));
    }
    Ok(path)
}
pub fn restore_receipt(path: &Path) -> Result<Vec<String>> {
    let directory = receipt_directory()?;
    let path = std::fs::canonicalize(path)?;
    anyhow::ensure!(
        crate::filesystem::contains(&directory, &path)
            && path.extension().is_some_and(|s| s == "json"),
        "Choose a receipt from the protected OverlayBackups folder"
    );
    let mut receipt: Receipt = serde_json::from_slice(&std::fs::read(&path)?)?;
    anyhow::ensure!(receipt.version == 1, "Unsupported receipt version");
    let root = RegKey::predef(HKEY_LOCAL_MACHINE)
        .create_subkey_with_flags(OVERLAY_KEY, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?
        .0;
    let mut skipped = vec![];
    for entry in receipt.entries.clone() {
        anyhow::ensure!(
            !entry.name.contains(['\\', '/']) && digest(&entry.tree)? == entry.digest,
            "Receipt integrity failed"
        );
        if receipt.restored.contains(&entry.name) {
            continue;
        }
        if root
            .open_subkey_with_flags(&entry.name, KEY_READ | KEY_WOW64_64KEY)
            .is_ok()
        {
            skipped.push(entry.name);
            continue;
        }
        let key = root
            .create_subkey_with_flags(&entry.name, KEY_ALL_ACCESS | KEY_WOW64_64KEY)?
            .0;
        restore_tree(&key, &entry.tree)?;
        receipt.restored.push(entry.name);
        save_receipt(&path, &receipt)?;
    }
    Ok(skipped)
}
pub fn register(directory: &Path, overlays: bool, context: bool) -> Result<()> {
    let directory = std::fs::canonicalize(directory)?;
    let dll = directory.join("omnisync_shell.dll");
    anyhow::ensure!(
        dll.is_file() && directory.join("omnisync.exe").is_file(),
        "Installed OmniSync components are missing"
    );
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    for (i, clsid) in CLSIDS.iter().chain(COMMAND_CLSIDS.iter()).enumerate() {
        if i < 5 && !overlays || i >= 5 && !context {
            continue;
        }
        let key = machine
            .create_subkey_with_flags(
                format!(r"SOFTWARE\Classes\CLSID\{clsid}\InprocServer32"),
                KEY_WRITE | KEY_WOW64_64KEY,
            )?
            .0;
        key.set_value("", &crate::filesystem::display(&dll))?;
        key.set_value("ThreadingModel", &"Apartment")?;
    }
    if overlays {
        for (i, clsid) in CLSIDS.iter().enumerate() {
            machine
                .create_subkey_with_flags(
                    format!("{OVERLAY_KEY}\\OmniSync {} {}", i + 1, BADGES[i]),
                    KEY_WRITE | KEY_WOW64_64KEY,
                )?
                .0
                .set_value("", clsid)?;
        }
    }
    if context {
        for class in ["*", "Directory"] {
            for (i, clsid) in COMMAND_CLSIDS.iter().enumerate() {
                let key = machine
                    .create_subkey_with_flags(
                        format!(r"SOFTWARE\Classes\{class}\shell\OmniSync{i}"),
                        KEY_WRITE | KEY_WOW64_64KEY,
                    )?
                    .0;
                key.set_value("ExplorerCommandHandler", clsid)?;
                key.set_value(
                    "MUIVerb",
                    &[
                        "Back up with OmniSync…",
                        "Exclude from OmniSync…",
                        "Show OmniSync status",
                    ][i],
                )?;
                key.set_value(
                    "Icon",
                    &crate::filesystem::display(&directory.join("icons/app.ico")),
                )?;
                key.set_value("MultiSelectModel", &"Single")?;
            }
        }
    }
    Ok(())
}
pub fn unregister(directory: &Path) -> Result<()> {
    let expected = crate::filesystem::display(&directory.join("omnisync_shell.dll"));
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    for (i, clsid) in CLSIDS.iter().chain(COMMAND_CLSIDS.iter()).enumerate() {
        let path = format!(r"SOFTWARE\Classes\CLSID\{clsid}");
        let ours = machine
            .open_subkey_with_flags(
                format!(r"{path}\InprocServer32"),
                KEY_READ | KEY_WOW64_64KEY,
            )
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(&expected));
        if !ours {
            continue;
        }
        if i < 5 {
            let overlay = format!("{OVERLAY_KEY}\\OmniSync {} {}", i + 1, BADGES[i]);
            if machine
                .open_subkey_with_flags(&overlay, KEY_READ | KEY_WOW64_64KEY)
                .ok()
                .and_then(|k| k.get_value::<String, _>("").ok())
                .as_deref()
                == Some(clsid)
            {
                let _ = machine.delete_subkey_all(&overlay);
            }
        } else {
            for class in ["*", "Directory"] {
                let path = format!(r"SOFTWARE\Classes\{class}\shell\OmniSync{}", i - 5);
                if machine
                    .open_subkey_with_flags(&path, KEY_READ | KEY_WOW64_64KEY)
                    .ok()
                    .and_then(|k| k.get_value::<String, _>("ExplorerCommandHandler").ok())
                    .as_deref()
                    == Some(clsid)
                {
                    let _ = machine.delete_subkey_all(path);
                }
            }
        }
        machine.delete_subkey_all(&path)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn types_roundtrip_without_coercion() {
        for kind in 0..=11 {
            assert_eq!(value_type(kind).unwrap() as u32, kind);
        }
        assert!(value_type(12).is_err());
    }
    #[test]
    fn digest_covers_bytes_and_type() {
        let mut tree = Tree {
            values: BTreeMap::new(),
            children: BTreeMap::new(),
            security: vec![1, 2],
        };
        let first = digest(&tree).unwrap();
        tree.values.insert(
            "".into(),
            Value {
                kind: 1,
                bytes: vec![0, 0],
            },
        );
        assert_ne!(first, digest(&tree).unwrap());
        let before = digest(&tree).unwrap();
        tree.values.get_mut("").unwrap().kind = 3;
        assert_ne!(before, digest(&tree).unwrap());
    }
}
