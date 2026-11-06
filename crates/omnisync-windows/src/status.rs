//! Bounded, per-user shared-memory status lookup. Every word is atomic; an odd
//! generation or expired heartbeat suppresses all overlays instead of stale green.
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Security::SECURITY_ATTRIBUTES,
        System::Memory::*,
    },
};
const HEADER: usize = 8;
const WIDTH: usize = 5;
const CAPACITY: usize = 262144;
const WORDS: usize = HEADER + WIDTH * CAPACITY;
const MAGIC: u64 = 0x4F4D4E4953594E43;
fn time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn hash(path: &Path) -> [u64; 4] {
    let digest = Sha256::digest(crate::filesystem::key(path));
    std::array::from_fn(|i| u64::from_le_bytes(digest[i * 8..i * 8 + 8].try_into().unwrap()))
}
fn name() -> Result<Vec<u16>> {
    Ok(crate::wide(format!(
        "Local\\OmniSync-status-{}",
        crate::security::sid()?
    )))
}
pub struct Cache {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    writable: bool,
}
// The map contains only aligned AtomicU64 words. All access is atomic and bounded.
unsafe impl Send for Cache {}
unsafe impl Sync for Cache {}
impl Cache {
    /// A separate cache for diagnostic tests; never opens the active user's status table.
    pub fn isolated() -> Result<Self> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        Self::open_named(
            true,
            crate::wide(format!(
                "Local\\OmniSync-diagnostic-{}-{suffix}",
                std::process::id()
            )),
        )
    }
    pub fn open(writable: bool) -> Result<Self> {
        Self::open_named(writable, name()?)
    }
    fn open_named(writable: bool, name: Vec<u16>) -> Result<Self> {
        unsafe {
            let handle = if writable {
                let sd = crate::security::Descriptor::from_sddl(&format!(
                    "D:P(A;;GA;;;SY)(A;;GA;;;{})",
                    crate::security::sid()?
                ))?;
                let attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: sd.0 .0,
                    bInheritHandle: false.into(),
                };
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    Some(&attributes),
                    PAGE_READWRITE,
                    0,
                    (WORDS * 8) as u32,
                    PCWSTR(name.as_ptr()),
                )?
            } else {
                OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr()))?
            };
            let view = MapViewOfFile(
                handle,
                if writable {
                    FILE_MAP_ALL_ACCESS
                } else {
                    FILE_MAP_READ
                },
                0,
                0,
                WORDS * 8,
            );
            if view.Value.is_null() {
                let _ = CloseHandle(handle);
                bail!("Status cache unavailable");
            }
            let result = Self {
                handle,
                view,
                writable,
            };
            if writable {
                result.word(0).store(MAGIC, Ordering::Release);
                result.invalidate();
            }
            Ok(result)
        }
    }
    fn word(&self, index: usize) -> &AtomicU64 {
        assert!(index < WORDS);
        unsafe { &*(self.view.Value.cast::<AtomicU64>().add(index)) }
    }
    pub fn invalidate(&self) {
        if self.writable {
            self.word(2).store(0, Ordering::Release);
        }
    }
    pub fn heartbeat(&self) {
        if self.writable {
            self.word(2).store(time(), Ordering::Release);
        }
    }
    pub fn publish(&self, entries: &[(std::path::PathBuf, u32)]) -> Result<()> {
        if !self.writable {
            bail!("Read-only status cache");
        }
        if entries.len() > CAPACITY {
            self.invalidate();
            bail!("Explorer cache capacity exceeded; app status remains available");
        }
        let mut values: Vec<_> = entries.iter().map(|(p, s)| (hash(p), *s)).collect();
        values.sort_unstable_by_key(|v| v.0);
        values.dedup_by_key(|v| v.0);
        let previous = self.word(1).load(Ordering::Acquire) & !1;
        self.word(1).store(previous + 1, Ordering::Release);
        self.word(2).store(0, Ordering::Release);
        for (i, (key, status)) in values.iter().enumerate() {
            for (j, v) in key.iter().enumerate() {
                self.word(HEADER + i * WIDTH + j)
                    .store(*v, Ordering::Relaxed);
            }
            self.word(HEADER + i * WIDTH + 4)
                .store(*status as u64, Ordering::Relaxed);
        }
        self.word(3).store(values.len() as u64, Ordering::Release);
        self.word(1).store(previous + 2, Ordering::Release);
        self.heartbeat();
        Ok(())
    }
    pub fn lookup(&self, path: &Path) -> Option<u32> {
        if self.word(0).load(Ordering::Acquire) != MAGIC {
            return None;
        }
        let seq = self.word(1).load(Ordering::Acquire);
        let stamp = self.word(2).load(Ordering::Acquire);
        let now = time();
        if seq & 1 != 0 || stamp == 0 || now.saturating_sub(stamp) > 15 || stamp > now + 5 {
            return None;
        }
        let key = hash(path);
        let mut low = 0;
        let mut high = self.word(3).load(Ordering::Acquire) as usize;
        if high > CAPACITY {
            return None;
        }
        let mut found = None;
        while low < high {
            let mid = low + (high - low) / 2;
            let value: [u64; 4] = std::array::from_fn(|i| {
                self.word(HEADER + mid * WIDTH + i).load(Ordering::Relaxed)
            });
            match value.cmp(&key) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid,
                std::cmp::Ordering::Equal => {
                    found =
                        Some(self.word(HEADER + mid * WIDTH + 4).load(Ordering::Relaxed) as u32);
                    break;
                }
            }
        }
        if self.word(1).load(Ordering::Acquire) != seq || self.word(2).load(Ordering::Acquire) == 0
        {
            None
        } else {
            found.filter(|s| (1..=5).contains(s))
        }
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_and_invalidated_cache_never_returns_green() {
        let name = crate::wide(format!(
            "Local\\OmniSync-test-cache-{}-{}",
            std::process::id(),
            time()
        ));
        let writer = Cache::open_named(true, name.clone()).unwrap();
        let reader = Cache::open_named(false, name).unwrap();
        let path = std::path::PathBuf::from("C:/Example/Photo.jpg");
        writer.publish(&[(path.clone(), 1)]).unwrap();
        assert_eq!(reader.lookup(&path), Some(1));
        assert_eq!(reader.lookup(Path::new("C:/Example/other.jpg")), None);
        writer.word(2).store(time() - 16, Ordering::Release);
        assert_eq!(reader.lookup(&path), None);
        writer.heartbeat();
        writer.invalidate();
        assert_eq!(reader.lookup(&path), None);
    }
    #[test]
    fn malformed_shared_count_is_rejected() {
        let name = crate::wide(format!(
            "Local\\OmniSync-test-bounds-{}",
            std::process::id()
        ));
        let cache = Cache::open_named(true, name).unwrap();
        cache
            .word(3)
            .store((CAPACITY + 1) as u64, Ordering::Release);
        cache.heartbeat();
        assert_eq!(cache.lookup(Path::new("C:/a")), None);
    }
}
