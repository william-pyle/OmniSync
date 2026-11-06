//! Explorer reads a committed snapshot without filesystem or network work.
//! Two banks keep the previous snapshot readable while the next one is prepared.
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{fence, AtomicU64, Ordering},
        Mutex,
    },
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Security::SECURITY_ATTRIBUTES,
        System::{Memory::*, SystemInformation::GetTickCount64},
    },
};

const HEADER: usize = 16;
const WIDTH: usize = 5;
const CAPACITY: usize = 262144;
const WORDS: usize = HEADER + 2 * WIDTH * CAPACITY;
const MAGIC: u64 = 0x4F4D4E4953594E02;
const TTL: u64 = 15000;
fn time() -> u64 {
    unsafe { GetTickCount64() }
}
fn hash(path: &Path) -> [u64; 4] {
    let digest = Sha256::digest(crate::filesystem::key(path));
    std::array::from_fn(|i| u64::from_le_bytes(digest[i * 8..i * 8 + 8].try_into().unwrap()))
}
fn related(a: &str, b: &str) -> bool {
    fn within(a: &str, b: &str) -> bool {
        a == b || a.strip_prefix(b).is_some_and(|tail| tail.starts_with('\\'))
    }
    within(a, b) || within(b, a)
}
#[derive(Default)]
struct Writer {
    epoch: u64,
    all_dirty: Option<u64>,
    dirty: BTreeMap<String, u64>,
    // Normalized path -> (original path, effective state, sorted table index).
    published: BTreeMap<String, (PathBuf, u32, usize)>,
    changes: BTreeMap<String, PathBuf>,
}
pub struct Cache {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    writable: bool,
    writer: Mutex<Writer>,
}
// Shared words are aligned atomics; only this process's writer mutex publishes.
unsafe impl Send for Cache {}
unsafe impl Sync for Cache {}
impl Cache {
    pub fn isolated() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        Self::open_named(
            true,
            crate::wide(format!(
                "Local\\OmniSync-diagnostic-{}-{stamp}",
                std::process::id()
            )),
        )
    }
    pub fn open(writable: bool) -> Result<Self> {
        Self::open_named(
            writable,
            crate::wide(format!(
                "Local\\OmniSync-status-v2-{}",
                crate::security::sid()?
            )),
        )
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
            let cache = Self {
                handle,
                view,
                writable,
                writer: Mutex::new(Writer::default()),
            };
            if writable {
                cache.word(3).store(0, Ordering::SeqCst);
                cache.word(0).store(MAGIC, Ordering::Release);
            }
            Ok(cache)
        }
    }
    fn word(&self, index: usize) -> &AtomicU64 {
        assert!(index < WORDS);
        unsafe { &*self.view.Value.cast::<AtomicU64>().add(index) }
    }
    fn offset(bank: usize, index: usize) -> usize {
        HEADER + bank * CAPACITY * WIDTH + index * WIDTH
    }
    pub fn epoch(&self) -> u64 {
        self.writer.lock().unwrap().epoch
    }
    /// Hard stop. A heartbeat cannot revive an invalidated table.
    pub fn invalidate(&self) {
        if self.writable {
            let mut writer = self.writer.lock().unwrap();
            self.word(3).store(0, Ordering::SeqCst);
            for (key, (path, _, _)) in writer.published.clone() {
                writer.changes.insert(key, path);
            }
        }
    }
    pub fn heartbeat(&self) {
        if self.writable {
            let _guard = self.writer.lock().unwrap();
            if self.word(3).load(Ordering::Acquire) == 1 {
                self.word(2).store(time(), Ordering::Release);
            }
        }
    }
    /// Filesystem uncertainty changes affected green badges to Pending, never blanks everything.
    pub fn mark_pending(&self, paths: Option<&[PathBuf]>) {
        if !self.writable {
            return;
        }
        let mut writer = self.writer.lock().unwrap();
        writer.epoch += 1;
        let epoch = writer.epoch;
        if let Some(paths) = paths {
            for path in paths {
                writer.dirty.insert(crate::filesystem::key(path), epoch);
            }
        } else {
            writer.all_dirty = Some(epoch);
        }
        if writer.dirty.len() > 256 {
            writer.dirty.clear();
            writer.all_dirty = Some(epoch);
        }
        let dirty = writer.dirty.clone();
        let all = writer.all_dirty.is_some();
        let bank = (self.word(1).load(Ordering::Acquire) & 1) as usize;
        let mut changes = vec![];
        for (key, (path, state, index)) in &mut writer.published {
            if *state == 1 && (all || dirty.keys().any(|p| related(key, p))) {
                *state = 3;
                self.word(Self::offset(bank, *index) + 4)
                    .store(3, Ordering::Release);
                changes.push((key.clone(), path.clone()));
            }
        }
        writer.changes.extend(changes);
    }
    pub fn publish(&self, entries: &[(PathBuf, u32)]) -> Result<()> {
        self.publish_scanned(entries, None)
    }
    /// Only a completed scan may clear changes observed before that scan began.
    pub fn publish_scanned(
        &self,
        entries: &[(PathBuf, u32)],
        scanned_through: Option<u64>,
    ) -> Result<()> {
        if !self.writable {
            bail!("Read-only status cache");
        }
        if entries.len() > CAPACITY {
            self.invalidate();
            bail!("Explorer cache capacity exceeded; app status remains available");
        }
        let mut values: Vec<_> = entries
            .iter()
            .map(|(p, s)| (hash(p), p.clone(), crate::filesystem::key(p), *s))
            .collect();
        values.sort_unstable_by_key(|v| v.0);
        values.dedup_by_key(|v| v.0);
        let mut writer = self.writer.lock().unwrap();
        if let Some(epoch) = scanned_through {
            writer.dirty.retain(|_, at| *at > epoch);
            if writer.all_dirty.is_some_and(|at| at <= epoch) {
                writer.all_dirty = None;
            }
        }
        let generation = self.word(1).load(Ordering::Acquire).wrapping_add(1);
        let bank = (generation & 1) as usize;
        let previous = self.word(4 + bank).load(Ordering::Acquire) & !1;
        self.word(4 + bank)
            .store(previous.wrapping_add(1), Ordering::SeqCst);
        let mut published = BTreeMap::new();
        for (index, (digest, path, key, state)) in values.into_iter().enumerate() {
            let state = if state == 1
                && (writer.all_dirty.is_some() || writer.dirty.keys().any(|p| related(&key, p)))
            {
                3
            } else {
                state
            };
            for (i, word) in digest.into_iter().enumerate() {
                self.word(Self::offset(bank, index) + i)
                    .store(word, Ordering::Relaxed);
            }
            self.word(Self::offset(bank, index) + 4)
                .store(state as u64, Ordering::Relaxed);
            if writer.published.get(&key).map(|(_, s, _)| *s) != Some(state) {
                writer.changes.insert(key.clone(), path.clone());
            }
            published.insert(key, (path, state, index));
        }
        let removed: Vec<_> = writer
            .published
            .iter()
            .filter(|(key, _)| !published.contains_key(*key))
            .map(|(key, (path, _, _))| (key.clone(), path.clone()))
            .collect();
        writer.changes.extend(removed);
        self.word(6 + bank)
            .store(published.len() as u64, Ordering::Release);
        self.word(4 + bank)
            .store(previous.wrapping_add(2), Ordering::Release);
        self.word(1).store(generation, Ordering::Release);
        self.word(2).store(time(), Ordering::Release);
        self.word(3).store(1, Ordering::Release);
        writer.published = published;
        Ok(())
    }
    pub fn drain_changes(&self, limit: usize) -> Vec<PathBuf> {
        let mut writer = self.writer.lock().unwrap();
        (0..limit)
            .filter_map(|_| writer.changes.pop_first().map(|(_, p)| p))
            .collect()
    }
    pub fn lookup(&self, path: &Path) -> Option<u32> {
        let key = hash(path);
        for _ in 0..8 {
            if self.word(0).load(Ordering::Acquire) != MAGIC
                || self.word(3).load(Ordering::Acquire) != 1
            {
                return None;
            }
            let stamp = self.word(2).load(Ordering::Acquire);
            let now = time();
            if stamp > now || now - stamp > TTL {
                return None;
            }
            let generation = self.word(1).load(Ordering::Acquire);
            let bank = (generation & 1) as usize;
            let seq = self.word(4 + bank).load(Ordering::Acquire);
            if seq & 1 != 0 {
                continue;
            }
            let mut low = 0;
            let mut high = self.word(6 + bank).load(Ordering::Acquire) as usize;
            if high > CAPACITY {
                return None;
            }
            let mut found = None;
            while low < high {
                let mid = low + (high - low) / 2;
                let value: [u64; 4] = std::array::from_fn(|i| {
                    self.word(Self::offset(bank, mid) + i)
                        .load(Ordering::Relaxed)
                });
                match value.cmp(&key) {
                    std::cmp::Ordering::Less => low = mid + 1,
                    std::cmp::Ordering::Greater => high = mid,
                    std::cmp::Ordering::Equal => {
                        found = Some(
                            self.word(Self::offset(bank, mid) + 4)
                                .load(Ordering::Acquire) as u32,
                        );
                        break;
                    }
                }
            }
            // Keep all table reads ahead of the second sequence check.
            fence(Ordering::AcqRel);
            if self.word(4 + bank).load(Ordering::Acquire) == seq
                && self.word(3).load(Ordering::Acquire) == 1
            {
                return found.filter(|s| (1..=5).contains(s));
            }
        }
        None
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
    fn heartbeat_keeps_idle_snapshot_and_cannot_revive_shutdown() {
        let cache = Cache::isolated().unwrap();
        let p = PathBuf::from(r"C:\test\file.txt");
        cache.publish(&[(p.clone(), 1)]).unwrap();
        cache
            .word(2)
            .store(time().saturating_sub(TTL + 1), Ordering::Release);
        assert_eq!(cache.lookup(&p), None);
        cache.heartbeat();
        assert_eq!(cache.lookup(&p), Some(1));
        cache.invalidate();
        cache.heartbeat();
        assert_eq!(cache.lookup(&p), None);
    }
    #[test]
    fn pending_change_preserves_other_files_and_survives_old_scan() {
        let c = Cache::isolated().unwrap();
        let a = PathBuf::from(r"C:\test\a.txt");
        let b = PathBuf::from(r"C:\test\b.txt");
        let parent = PathBuf::from(r"C:\test");
        let entries = vec![(a.clone(), 1), (b.clone(), 1), (parent.clone(), 1)];
        c.publish(&entries).unwrap();
        c.drain_changes(10);
        let before = c.epoch();
        c.mark_pending(Some(std::slice::from_ref(&a)));
        assert_eq!(c.lookup(&a), Some(3));
        assert_eq!(c.lookup(&parent), Some(3));
        assert_eq!(c.lookup(&b), Some(1));
        c.publish_scanned(&entries, Some(before)).unwrap();
        assert_eq!(c.lookup(&a), Some(3));
        c.publish_scanned(&entries, Some(c.epoch())).unwrap();
        assert_eq!(c.lookup(&a), Some(1));
        assert_eq!(c.drain_changes(10).len(), 2);
        c.publish(&entries).unwrap();
        assert!(c.drain_changes(10).is_empty());
    }
    #[test]
    fn readers_keep_committed_table_during_publication() {
        let name = crate::wide(format!(
            "Local\\OmniSync-concurrency-{}-{}",
            std::process::id(),
            time()
        ));
        let writer = std::sync::Arc::new(Cache::open_named(true, name.clone()).unwrap());
        let reader = Cache::open_named(false, name).unwrap();
        let p = PathBuf::from(r"C:\test\stable.txt");
        writer.publish(&[(p.clone(), 1)]).unwrap();
        let w = writer.clone();
        let other = p.clone();
        let task = std::thread::spawn(move || {
            for _ in 0..100 {
                w.publish(&[(other.clone(), 1)]).unwrap();
            }
        });
        for _ in 0..10000 {
            assert_eq!(reader.lookup(&p), Some(1));
        }
        task.join().unwrap();
    }
    #[test]
    fn malformed_shared_count_is_rejected() {
        let c = Cache::isolated().unwrap();
        c.publish(&[]).unwrap();
        let bank = (c.word(1).load(Ordering::Acquire) & 1) as usize;
        c.word(6 + bank)
            .store((CAPACITY + 1) as u64, Ordering::Release);
        assert_eq!(c.lookup(Path::new(r"C:\a")), None);
    }
}
