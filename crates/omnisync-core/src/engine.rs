use crate::{
    drive::{Drive, UploadReply, CHUNK},
    model::*,
    oauth::Auth,
    rules::Matcher,
    staging,
    store::Store,
};
use anyhow::{bail, Context, Result};
use notify::Watcher;
use omnisync_windows::{filesystem, security, status::Cache};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

struct PendingPreview {
    config: Config,
    expires: i64,
    paths: Vec<(String, String)>,
}
mod cleanup;
struct StatusService {
    task: tokio::task::JoinHandle<()>,
    cache: Arc<Cache>,
}
impl Drop for StatusService {
    fn drop(&mut self) {
        self.task.abort();
        self.cache.invalidate();
    }
}
pub struct Engine {
    pub state: PathBuf,
    store: Mutex<Store>,
    pub auth: Auth,
    drive: Drive,
    progress: Mutex<Progress>,
    summary: Mutex<Counts>,
    previews: Mutex<HashMap<String, PendingPreview>>,
    cleanup_previews: Mutex<HashMap<String, cleanup::CleanupPlan>>,
    cleanup_running: AtomicBool,
    wake: tokio::sync::Notify,
    dirty: AtomicBool,
    force: AtomicBool,
    stopping: AtomicBool,
    cache: Arc<Cache>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    watched: Mutex<Vec<String>>,
    scan_lock: Mutex<()>,
    run_lock: tokio::sync::Mutex<()>,
}
impl Engine {
    pub fn open(state: PathBuf) -> Result<Arc<Self>> {
        Self::open_with_cache(state, Cache::open(true)?)
    }
    fn open_with_cache(state: PathBuf, cache: Cache) -> Result<Arc<Self>> {
        security::private_directory(&state, false)?;
        let store = Store::open(&state.join("state.sqlite"))?;
        // 0.1.0 did not persist coverage per item. Republish its known versions once;
        // this does not upload file contents again or change source configuration.
        if store.items()?.iter().any(|i| {
            i.current
                .as_ref()
                .is_some_and(|v| i.cataloged_remote_id.as_deref() != Some(&v.remote_id))
        }) {
            store.set("catalogDirty", &true)?;
        }
        let last_backup = store.setting::<i64>("lastBackup")?;
        let auth = Auth::new(&state)?;
        let drive = Drive::new(auth.clone())?;
        Ok(Arc::new(Self {
            state,
            store: Mutex::new(store),
            auth,
            drive,
            progress: Mutex::new(Progress {
                phase: "starting".into(),
                last_backup,
                ..Default::default()
            }),
            summary: Mutex::new(Counts::default()),
            previews: Mutex::new(HashMap::new()),
            cleanup_previews: Mutex::new(HashMap::new()),
            cleanup_running: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            dirty: AtomicBool::new(true),
            force: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            cache: Arc::new(cache),
            watcher: Mutex::new(None),
            watched: Mutex::new(vec![]),
            scan_lock: Mutex::new(()),
            run_lock: tokio::sync::Mutex::new(()),
        }))
    }
    pub fn config(&self) -> Result<Config> {
        self.store.lock().unwrap().config()
    }
    pub fn items(&self) -> Result<Vec<Item>> {
        self.store.lock().unwrap().items()
    }
    fn account(&self) -> Result<Account> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .setting("account")?
            .unwrap_or_default())
    }
    pub async fn snapshot(self: &Arc<Self>) -> Result<Snapshot> {
        let connected = self.auth.connected().await;
        let configured = self.auth.configured().await;
        let engine = self.clone();
        tokio::task::spawn_blocking(move || engine.snapshot_local(connected, configured)).await?
    }
    fn snapshot_local(&self, connected: bool, oauth_configured: bool) -> Result<Snapshot> {
        let config = self.config()?;
        let mut account = self.account()?;
        account.connected = connected;
        let mut recent = self.display_items()?;
        let mut counts = Counts::default();
        for item in &recent {
            match item.status {
                Status::BackedUp => counts.backed_up += 1,
                Status::Pending | Status::Syncing => counts.pending += 1,
                Status::Issue => counts.issues += 1,
                Status::Excluded => counts.excluded += 1,
                Status::Retained => counts.retained += 1,
            }
        }
        *self.summary.lock().unwrap() = counts.clone();
        recent.sort_by_key(|i| std::cmp::Reverse(i.last_seen));
        recent.truncate(1000);
        let mut progress = self.progress.lock().unwrap().clone();
        if config.paused() {
            progress.phase = "paused".into();
            progress.message = "Backup is paused. Existing cloud copies are retained.".into();
        }
        Ok(Snapshot {
            config,
            account,
            progress,
            counts,
            recent,
            oauth_configured,
            version: env!("CARGO_PKG_VERSION").into(),
        })
    }
    pub fn display_items(&self) -> Result<Vec<Item>> {
        let mut items = self.items()?;
        for item in &mut items {
            if item.status != item.effective_status() {
                item.status = Status::Pending;
                item.reason = "Publishing recovery catalog".into();
            }
        }
        Ok(items)
    }
    pub fn tray_status(&self) -> Result<(bool, String, Counts)> {
        let paused = self.config()?.paused();
        let phase = if paused {
            "paused".into()
        } else {
            self.progress.lock().unwrap().phase.clone()
        };
        Ok((paused, phase, self.summary.lock().unwrap().clone()))
    }
    fn phase(&self, phase: &str, message: &str) {
        let mut p = self.progress.lock().unwrap();
        p.phase = phase.into();
        p.message = message.into();
    }
    pub fn rescan(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        self.cache.mark_pending(None);
        self.wake.notify_one();
    }
    pub fn sync_now(&self) -> Result<()> {
        anyhow::ensure!(
            !self.config()?.paused(),
            "Resume backup before selecting Sync now"
        );
        self.force.store(true, Ordering::SeqCst);
        self.rescan();
        Ok(())
    }
    pub fn pause(&self, seconds: Option<i64>) -> Result<()> {
        let mut s = self.store.lock().unwrap();
        anyhow::ensure!(
            !self.cleanup_running.load(Ordering::SeqCst),
            "Wait for cloud cleanup to finish before changing backup preferences"
        );
        let mut c = s.config()?;
        let expected = c.revision;
        c.revision += 1;
        c.paused_until = seconds.map(|n| if n <= 0 { 0 } else { now() + n });
        s.save_config(&c, expected)?;
        drop(s);
        self.wake.notify_one();
        Ok(())
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.cache.invalidate();
        self.wake.notify_one();
    }
    fn permitted(&self) -> Result<()> {
        anyhow::ensure!(
            !self.cleanup_running.load(Ordering::SeqCst),
            "Backup is yielding at a checkpoint for cloud cleanup"
        );
        anyhow::ensure!(
            !self.dirty.load(Ordering::SeqCst),
            "Checking changed source files before continuing backup"
        );
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "OmniSync is closing");
        anyhow::ensure!(
            !self.config()?.paused(),
            "Backup paused at a safe checkpoint"
        );
        Ok(())
    }
    pub async fn connect(&self) -> Result<()> {
        if self.progress.lock().unwrap().connecting {
            bail!("Sign-in is already open in your browser");
        }
        self.progress.lock().unwrap().connecting = true;
        let result = async {
            self.auth.connect().await?;
            self.refresh_account().await?;
            self.rescan();
            Ok(())
        }
        .await;
        self.progress.lock().unwrap().connecting = false;
        result
    }
    pub async fn disconnect(&self) -> Result<()> {
        self.pause(Some(0))?;
        let _guard = self.run_lock.lock().await;
        self.auth.disconnect().await?;
        self.phase(
            "disconnected",
            "Google Drive is disconnected. Cloud backups are retained.",
        );
        Ok(())
    }
    async fn refresh_account(&self) -> Result<()> {
        let v = self.drive.about().await?;
        let mut account = self.account()?;
        let permission = v["user"]["permissionId"]
            .as_str()
            .context("Drive account identity is missing")?
            .to_string();
        if !account.permission_id.is_empty() && account.permission_id != permission {
            self.auth.disconnect().await?;
            bail!("This profile belongs to another Google account. Reconnect the original account; use a separate Windows user for a separate profile.");
        }
        account.permission_id = permission;
        account.display_name = v["user"]["displayName"]
            .as_str()
            .unwrap_or("Google Drive")
            .into();
        account.email = v["user"]["emailAddress"]
            .as_str()
            .unwrap_or_default()
            .into();
        account.connected = true;
        account.quota_used = v["storageQuota"]["usage"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        account.quota_limit = v["storageQuota"]["limit"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        self.store.lock().unwrap().set("account", &account)?;
        Ok(())
    }
    pub fn preview(&self, mut config: Config, description: String) -> Result<Preview> {
        let current = self.config()?;
        anyhow::ensure!(
            config.revision == current.revision,
            "Configuration changed; open this review again"
        );
        // Device identity and security limits are controlled by the host, never imported from a web view.
        config.device_id = current.device_id;
        config.revision = current.revision + 1;
        anyhow::ensure!(
            config.staging_budget_bytes >= 1024 * 1024
                && config.free_space_reserve_bytes >= 1024 * 1024,
            "Storage limits must be at least 1 MiB"
        );
        anyhow::ensure!(
            config.upload_limit_kib != Some(0),
            "Upload limit must be positive or empty"
        );
        anyhow::ensure!(
            ["system", "light", "dark"].contains(&config.theme.as_str()),
            "Unknown theme"
        );
        let mut paths = vec![];
        let mut ids = HashSet::new();
        for root in &mut config.roots {
            anyhow::ensure!(ids.insert(root.id.clone()), "Duplicate folder ID");
            let (path, identity, is_file) =
                filesystem::validate_root(Path::new(&root.path), &self.state)?;
            for (p, _) in &paths {
                anyhow::ensure!(
                    !filesystem::contains(Path::new(p), &path)
                        && !filesystem::contains(&path, Path::new(p)),
                    "Selected folders must not overlap"
                );
            }
            root.path = filesystem::display(&path);
            root.volume_identity = identity.clone();
            root.is_file = is_file;
            anyhow::ensure!(!root.label.trim().is_empty(), "A folder needs a name");
            paths.push((root.path.clone(), identity));
        }
        for rule in &config.rules {
            anyhow::ensure!(
                rule.scope == "global" || ids.contains(&rule.scope),
                "Rule refers to a missing folder"
            );
        }
        let matcher = Matcher::new(&config.rules)?;
        let mut included = 0;
        let mut excluded = 0;
        let mut bytes = 0;
        let mut examples = vec![];
        let mut warnings = vec![];
        for root in config.roots.iter().filter(|r| r.enabled) {
            walk_root(root, |path, relative| {
                match filesystem::eligible(path) {
                    Ok(()) => {
                        let decision = matcher.decide(&root.id, relative)?;
                        if decision.included {
                            included += 1;
                            bytes += std::fs::metadata(path)?.len();
                        } else {
                            excluded += 1;
                        }
                        if examples.len() < 12 {
                            examples.push(format!(
                                "{} / {} — {}",
                                root.label, relative, decision.reason
                            ));
                        }
                    }
                    Err(e) => {
                        excluded += 1;
                        if warnings.len() < 12 {
                            warnings.push(format!("{relative}: {e}"));
                        }
                    }
                }
                Ok(())
            })?;
        }
        let token = id();
        let preview = Preview {
            token: token.clone(),
            revision: current.revision,
            description,
            included,
            excluded,
            bytes,
            examples,
            warnings,
        };
        let mut pending = self.previews.lock().unwrap();
        pending.retain(|_, p| p.expires > now());
        pending.insert(
            token,
            PendingPreview {
                config,
                expires: now() + 600,
                paths,
            },
        );
        Ok(preview)
    }
    pub fn apply_preview(&self, token: &str) -> Result<()> {
        let preview = self
            .previews
            .lock()
            .unwrap()
            .remove(token)
            .context("Review expired. Preview the change again.")?;
        anyhow::ensure!(preview.expires > now(), "Review expired");
        for (path, expected) in preview.paths {
            let (_, identity, _) = filesystem::validate_root(Path::new(&path), &self.state)?;
            anyhow::ensure!(
                identity == expected,
                "A selected folder was replaced since the preview"
            );
        }
        let mut s = self.store.lock().unwrap();
        anyhow::ensure!(
            !self.cleanup_running.load(Ordering::SeqCst),
            "Wait for cloud cleanup to finish before changing exclusions"
        );
        s.save_config(&preview.config, preview.config.revision - 1)?;
        drop(s);
        self.rescan();
        Ok(())
    }
    pub fn undo_preview(&self) -> Result<Preview> {
        let current = self.config()?;
        let mut previous = self
            .store
            .lock()
            .unwrap()
            .previous_config(current.revision)?
            .context("No earlier configuration is available")?;
        previous.revision = current.revision;
        previous.paused_until = current.paused_until;
        self.preview(previous, "Undo the last saved configuration".into())
    }
    pub fn shell_preview(&self, path: &Path, include: bool) -> Result<Preview> {
        let mut c = self.config()?;
        filesystem::no_reparse(path)?;
        let path = std::fs::canonicalize(path)?;
        let root = c
            .roots
            .iter()
            .find(|r| filesystem::contains(Path::new(&r.path), &path))
            .cloned();
        if let Some(root) = root {
            let same = filesystem::key(Path::new(&root.path)) == filesystem::key(&path);
            if same && !root.is_file {
                c.roots
                    .iter_mut()
                    .find(|r| r.id == root.id)
                    .unwrap()
                    .enabled = include;
            } else {
                let relative = if root.is_file {
                    Path::new(&root.path)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    filesystem::display(&path)
                        .strip_prefix(&format!("{}\\", root.path))
                        .context("Cannot resolve relative path")?
                        .replace('\\', "/")
                };
                let order = c.rules.iter().map(|r| r.order).max().unwrap_or(0) + 1;
                c.rules.push(Rule {
                    id: id(),
                    scope: root.id,
                    kind: if path.is_dir() {
                        RuleKind::Subtree
                    } else {
                        RuleKind::ExactFile
                    },
                    action: if include {
                        Action::Include
                    } else {
                        Action::Exclude
                    },
                    value: relative,
                    order,
                });
            }
        } else {
            anyhow::ensure!(include, "This item is already outside your backup folders");
            c.roots.push(Root {
                id: id(),
                label: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                path: filesystem::display(&path),
                volume_identity: String::new(),
                enabled: true,
                is_file: path.is_file(),
            });
        }
        self.preview(
            c,
            if include {
                "Include selected item"
            } else {
                "Exclude selected item; keep cloud copies"
            }
            .into(),
        )
    }
    fn scan(&self) -> Result<()> {
        let _scan = self.scan_lock.lock().unwrap();
        let scan_epoch = self.cache.epoch();
        self.phase("scanning", "Checking selected folders");
        let config = self.config()?;
        let matcher = Matcher::new(&config.rules)?;
        let epoch = now();
        let mut failures = vec![];
        for root in &config.roots {
            if !root.enabled {
                for mut item in self.items()?.into_iter().filter(|i| i.root_id == root.id) {
                    item.status = Status::Excluded;
                    item.reason = "Folder backup is disabled; cloud copies retained".into();
                    self.store.lock().unwrap().put_item(&item)?;
                }
                continue;
            }
            let result = (|| -> Result<()> {
                let (_, identity, _) =
                    filesystem::validate_root(Path::new(&root.path), &self.state)?;
                anyhow::ensure!(
                    identity == root.volume_identity,
                    "Folder identity changed; select the folder again"
                );
                let mut seen = HashSet::new();
                walk_root(root, |path, relative| {
                    seen.insert(relative.to_lowercase());
                    let meta = std::fs::symlink_metadata(path)?;
                    let changed_at = staging::modified(&meta);
                    let previous = self.store.lock().unwrap().find(&root.id, relative)?;
                    let mut item = previous.unwrap_or(Item {
                        id: id(),
                        root_id: root.id.clone(),
                        relative_path: relative.into(),
                        size: meta.len(),
                        modified: changed_at,
                        generation: 1,
                        status: Status::Pending,
                        reason: String::new(),
                        winning_rule: None,
                        current: None,
                        cataloged_remote_id: None,
                        history: vec![],
                        last_seen: epoch,
                    });
                    if item.size != meta.len() || item.modified != changed_at {
                        item.generation += 1;
                        item.size = meta.len();
                        item.modified = changed_at;
                        item.status = Status::Pending;
                    }
                    item.last_seen = epoch;
                    let decision = matcher.decide(&root.id, relative)?;
                    item.winning_rule = decision.winning_rule;
                    if let Err(error) = filesystem::eligible(path) {
                        item.status = Status::Excluded;
                        item.reason = error.to_string();
                    } else if !decision.included {
                        item.status = Status::Excluded;
                        item.reason = decision.reason;
                    } else if item.status != Status::Issue
                        || !item.reason.starts_with("Cloud conflict:")
                    {
                        // Hash the stable source on every reconciliation; timestamps alone never establish green.
                        match filesystem::locked_source(path, Path::new(&root.path))
                            .and_then(staging::hashes)
                        {
                            Ok((sha, _, size)) => {
                                let same = item
                                    .current
                                    .as_ref()
                                    .is_some_and(|v| v.sha256 == sha && v.size == size);
                                if same {
                                    item.status = Status::BackedUp;
                                    item.reason = "Verified backup matches local content".into();
                                } else {
                                    let staged = self.store.lock().unwrap().job(&item.id)?;
                                    if item.status == Status::BackedUp
                                        || staged.is_some_and(|j| {
                                            j.generation == item.generation && j.sha256 != sha
                                        })
                                    {
                                        item.generation += 1;
                                    }
                                    item.status = Status::Pending;
                                    item.reason = "Waiting for verified backup".into();
                                }
                            }
                            Err(e) => {
                                item.status = Status::Issue;
                                item.reason = e.to_string();
                            }
                        }
                    }
                    self.store.lock().unwrap().put_item(&item)?;
                    Ok(())
                })?;
                for mut item in self.items()?.into_iter().filter(|i| {
                    i.root_id == root.id && !seen.contains(&i.relative_path.to_lowercase())
                }) {
                    item.status = Status::Retained;
                    item.reason = "Local item is absent; cloud versions retained".into();
                    self.store.lock().unwrap().put_item(&item)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                failures.push(format!("{}: {e}", root.label));
                for mut item in self.items()?.into_iter().filter(|i| i.root_id == root.id) {
                    item.status = Status::Issue;
                    item.reason="Backup folder unavailable or scan incomplete; no remote deletion performed".into();
                    self.store.lock().unwrap().put_item(&item)?;
                }
            }
        }
        for mut item in self
            .items()?
            .into_iter()
            .filter(|i| !config.roots.iter().any(|r| r.id == i.root_id))
        {
            item.status = Status::Retained;
            item.reason = "Folder removed from configuration; cloud versions retained".into();
            self.store.lock().unwrap().put_item(&item)?;
        }
        self.store.lock().unwrap().set("scanErrors", &failures)?;
        self.publish_cache_scanned(Some(scan_epoch))?;
        if !failures.is_empty() {
            self.phase("issue", &failures.join("; "));
        }
        Ok(())
    }
    fn publish_cache(&self) -> Result<()> {
        self.publish_cache_scanned(None)
    }
    fn publish_cache_scanned(&self, scanned_through: Option<u64>) -> Result<()> {
        if self.stopping.load(Ordering::SeqCst) {
            return Ok(());
        }
        let config = self.config()?;
        let items = self.items()?;
        let mut paths = vec![];
        let mut folders: HashMap<PathBuf, u32> = HashMap::new();
        let mut summary = Counts::default();
        for item in &items {
            match item.effective_status() {
                Status::BackedUp => summary.backed_up += 1,
                Status::Pending | Status::Syncing => summary.pending += 1,
                Status::Excluded => summary.excluded += 1,
                Status::Issue => summary.issues += 1,
                Status::Retained => summary.retained += 1,
            }
        }
        *self.summary.lock().unwrap() = summary;
        for root in &config.roots {
            for item in items.iter().filter(|i| i.root_id == root.id) {
                let state = item.effective_status().overlay();
                let path = if root.is_file {
                    PathBuf::from(&root.path)
                } else {
                    Path::new(&root.path).join(filesystem::safe_relative(&item.relative_path)?)
                };
                if state == 0 {
                    continue;
                }
                paths.push((path.clone(), state));
                if !root.is_file {
                    let mut parent = path.parent();
                    while let Some(directory) = parent {
                        if !filesystem::contains(Path::new(&root.path), directory) {
                            break;
                        }
                        let current = folders.entry(directory.to_path_buf()).or_insert(0);
                        *current = aggregate(*current, if !root.enabled { 4 } else { state });
                        parent = directory.parent();
                    }
                }
            }
        }
        paths.extend(folders);
        self.cache.publish_scanned(&paths, scanned_through)?;
        Ok(())
    }
    fn update_watchers(self: &Arc<Self>) -> Result<()> {
        let roots = self
            .config()?
            .roots
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| r.path)
            .collect::<Vec<_>>();
        if *self.watched.lock().unwrap() == roots && self.watcher.lock().unwrap().is_some() {
            return Ok(());
        }
        let weak = Arc::downgrade(self);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Some(engine) = weak.upgrade() {
                    match event {
                        Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => {}
                        Ok(event) if !event.paths.is_empty() && !event.need_rescan() => {
                            engine.cache.mark_pending(Some(&event.paths));
                            engine.dirty.store(true, Ordering::SeqCst);
                            engine.wake.notify_one();
                        }
                        _ => engine.rescan(),
                    }
                }
            })?;
        for root in &roots {
            watcher.watch(Path::new(root), notify::RecursiveMode::Recursive)?;
        }
        *self.watcher.lock().unwrap() = Some(watcher);
        *self.watched.lock().unwrap() = roots;
        Ok(())
    }
    pub async fn run(self: Arc<Self>) {
        // This task never scans or waits for a cloud request. Its lifetime follows the engine.
        let weak = Arc::downgrade(&self);
        let status_task = tokio::spawn(async move {
            let mut notification: Option<tokio::task::JoinHandle<()>> = None;
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let Some(engine) = weak.upgrade() else {
                    break;
                };
                if engine.stopping.load(Ordering::SeqCst) {
                    break;
                }
                engine.cache.heartbeat();
                if notification
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                {
                    continue;
                }
                let paths = engine.cache.drain_changes(256);
                if !paths.is_empty() {
                    notification = Some(tokio::task::spawn_blocking(move || {
                        omnisync_windows::integration::refresh_overlays(&paths)
                    }));
                }
            }
        });
        // Cancellation and unexpected worker exit must also stop the heartbeat.
        let _status = StatusService {
            task: status_task,
            cache: self.cache.clone(),
        };
        let mut reconciled = 0;
        let mut remote_checked = 0;
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                break;
            }
            let _guard = self.run_lock.lock().await;
            if self.cleanup_running.load(Ordering::SeqCst) {
                drop(_guard);
                tokio::time::sleep(Duration::from_millis(150)).await;
                continue;
            }
            let forced = self.force.swap(false, Ordering::SeqCst);
            let config = match self.config() {
                Ok(c) => c,
                Err(e) => {
                    self.phase("issue", &e.to_string());
                    break;
                }
            };
            if self.dirty.swap(false, Ordering::SeqCst) || now() - reconciled >= 86400 || forced {
                let e = self.clone();
                match tokio::task::spawn_blocking(move || e.scan()).await {
                    Ok(Ok(())) => {
                        reconciled = now();
                        if let Err(e) = self.update_watchers() {
                            self.phase("issue", &format!("Folder watching unavailable: {e}"));
                            self.dirty.store(true, Ordering::SeqCst);
                        }
                    }
                    Ok(Err(e)) => self.phase("issue", &e.to_string()),
                    Err(_) => self.phase("issue", "Folder scan stopped unexpectedly"),
                }
            }
            if config.paused() {
                self.phase("paused", "Backup is paused");
            } else if !self.auth.connected().await {
                self.phase("disconnected", "Connect Google Drive to begin backup");
            } else if config.pause_on_metered && metered() {
                self.phase(
                    "metered",
                    "Backup waits while this connection is metered or its cost is unknown",
                );
            } else if config.continuous || forced {
                let result = async {
                    if now() - remote_checked >= 300 || forced {
                        self.refresh_account().await?;
                        self.check_remote().await?;
                        remote_checked = now();
                    }
                    self.save_catalog().await?;
                    self.backup_pending().await?;
                    self.save_catalog().await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if let Err(e) = result {
                    self.phase("issue", &e.to_string());
                } else {
                    let items = self.items().unwrap_or_default();
                    let errors = self
                        .store
                        .lock()
                        .unwrap()
                        .setting::<Vec<String>>("scanErrors")
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let issues =
                        items.iter().filter(|i| i.status == Status::Issue).count() + errors.len();
                    let pending = items
                        .iter()
                        .any(|i| i.status == Status::Pending || i.status == Status::Syncing);
                    self.phase(
                        if issues > 0 {
                            "issue"
                        } else if pending {
                            "waiting"
                        } else {
                            "idle"
                        },
                        if issues > 0 {
                            "Some files or folders need attention"
                        } else if pending {
                            "Pending files will retry after their saved backoff"
                        } else {
                            "All selected changes are backed up"
                        },
                    );
                }
            } else {
                self.phase(
                    "idle",
                    "Automatic backup is off. Select Sync now to run backup.",
                );
            }
            let _ = self.publish_cache();
            drop(_guard);
            tokio::select! {_=self.wake.notified()=>tokio::time::sleep(Duration::from_millis(750)).await,_=tokio::time::sleep(Duration::from_secs(5))=>{}}
        }
    }
    async fn check_remote(&self) -> Result<()> {
        for mut item in self.items()?.into_iter().filter(|i| {
            i.current.is_some() && i.status != Status::Retained && i.status != Status::Excluded
        }) {
            self.permitted()?;
            let version = item.current.as_ref().unwrap();
            if !self
                .drive
                .metadata(&version.remote_id)
                .await?
                .is_some_and(|m| m.matches_version(version))
            {
                item.status = Status::Issue;
                item.reason="Cloud conflict: the verified Drive copy changed or was removed. Restore/review it, then acknowledge to create a separate new snapshot.".into();
                self.store.lock().unwrap().put_item(&item)?;
            }
        }
        Ok(())
    }
    pub fn acknowledge_conflict(&self, item_id: &str) -> Result<()> {
        let mut item = self
            .store
            .lock()
            .unwrap()
            .item(item_id)?
            .context("Unknown file")?;
        anyhow::ensure!(
            item.reason.starts_with("Cloud conflict:"),
            "No cloud conflict to acknowledge"
        );
        if let Some(previous) = item.current.take() {
            item.history.push(previous);
        }
        item.status = Status::Pending;
        item.reason = "Cloud conflict acknowledged; a new separate backup will be created".into();
        self.store.lock().unwrap().put_item(&item)?;
        self.rescan();
        Ok(())
    }
    async fn folder(
        &self,
        key: &str,
        name: &str,
        parent: Option<&str>,
        device: &str,
    ) -> Result<String> {
        let existing = self.store.lock().unwrap().remote_path(key)?;
        let id = match existing {
            Some(id) => id,
            None => {
                let id = self.drive.generate_id().await?;
                self.store.lock().unwrap().set_remote_path(key, &id)?;
                id
            }
        };
        self.drive.folder(&id, name, parent, device, key).await?;
        Ok(id)
    }
    async fn destination(&self, config: &Config, root: &Root, relative: &str) -> Result<String> {
        let base = self
            .folder("base", "OmniSync", None, &config.device_id)
            .await?;
        let device = self
            .folder(
                "device",
                &format!("Device {}", &config.device_id[..8]),
                Some(&base),
                &config.device_id,
            )
            .await?;
        let root_folder = self
            .folder(
                &format!("root:{}", root.id),
                &root.label,
                Some(&device),
                &config.device_id,
            )
            .await?;
        let mut parent = self
            .folder(
                &format!("files:{}", root.id),
                "Files",
                Some(&root_folder),
                &config.device_id,
            )
            .await?;
        let parts: Vec<_> = relative.split('/').collect();
        let mut path = String::new();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            path.push_str(part);
            path.push('/');
            parent = self
                .folder(
                    &format!("dir:{}:{path}", root.id),
                    part,
                    Some(&parent),
                    &config.device_id,
                )
                .await?;
        }
        let mut account = self.account()?;
        account.base_id = Some(base);
        account.device_folder_id = Some(device);
        self.store.lock().unwrap().set("account", &account)?;
        Ok(parent)
    }
    async fn backup_pending(self: &Arc<Self>) -> Result<()> {
        let mut checkpoint = std::time::Instant::now();
        let mut completed = 0;
        for mut item in self
            .items()?
            .into_iter()
            .filter(|i| i.status == Status::Pending || i.status == Status::Syncing)
        {
            self.permitted()?;
            if completed > 0
                && (item.size >= CHUNK as u64 || checkpoint.elapsed() >= Duration::from_secs(5))
            {
                self.save_catalog().await?;
                completed = 0;
                checkpoint = std::time::Instant::now();
            }
            let config = self.config()?;
            if item.reason == "Upload verified; publishing recovery catalog" {
                continue;
            }
            let Some(root) = config
                .roots
                .iter()
                .find(|r| r.enabled && r.id == item.root_id)
            else {
                continue;
            };
            if !Matcher::new(&config.rules)?
                .decide(&root.id, &item.relative_path)?
                .included
            {
                continue;
            }
            let existing = self.store.lock().unwrap().job(&item.id)?;
            let mut job = if let Some(job) = existing {
                if job.generation != item.generation {
                    self.discard_job(&job)?;
                    continue;
                }
                if job.retry_at > now() {
                    continue;
                }
                job
            } else {
                let e = self.clone_for_capture(root, &item, &config);
                match tokio::task::spawn_blocking(move || staging::capture(&e.0, &e.1, &e.2, &e.3))
                    .await?
                {
                    Ok(job) => {
                        self.store.lock().unwrap().put_job(&job)?;
                        job
                    }
                    Err(error) => {
                        item.status = Status::Issue;
                        item.reason = error.to_string();
                        self.store.lock().unwrap().put_item(&item)?;
                        continue;
                    }
                }
            };
            item.status = Status::Syncing;
            item.reason = "Uploading an immutable snapshot".into();
            self.store.lock().unwrap().put_item(&item)?;
            self.phase("syncing", "Backing up changes");
            {
                let mut p = self.progress.lock().unwrap();
                p.current_file = item.relative_path.clone();
                p.sent = job.offset;
                p.total = job.size;
            }
            self.publish_cache()?;
            let result = self.upload_item(&config, root, &item, &mut job).await;
            match result {
                Ok(version) => {
                    let mut latest = self
                        .store
                        .lock()
                        .unwrap()
                        .item(&item.id)?
                        .context("File record disappeared")?;
                    if let Some(old) = latest.current.replace(version) {
                        latest.history.push(old);
                    }
                    latest.status = Status::Pending;
                    latest.reason = "Upload verified; publishing recovery catalog".into();
                    self.store.lock().unwrap().commit_verified(&latest)?;
                    self.remove_stage(&job);
                    completed += 1;
                    // Bound progress latency without rewriting a full catalog after every tiny file.
                    if completed >= 8 || checkpoint.elapsed() >= Duration::from_secs(5) {
                        self.save_catalog().await?;
                        completed = 0;
                        checkpoint = std::time::Instant::now();
                    }
                }
                Err(error) => {
                    job.attempts = job.attempts.saturating_add(1);
                    job.retry_at =
                        now() + (5_i64.saturating_mul(2_i64.pow(job.attempts.min(9)))).min(1800);
                    self.store.lock().unwrap().put_job(&job)?;
                    item.status = if error.to_string().starts_with("Cloud conflict:") {
                        Status::Issue
                    } else {
                        Status::Pending
                    };
                    item.reason = error.to_string();
                    self.store.lock().unwrap().put_item(&item)?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    fn clone_for_capture(
        &self,
        root: &Root,
        item: &Item,
        config: &Config,
    ) -> (Root, Item, PathBuf, Config) {
        (
            root.clone(),
            item.clone(),
            self.state.clone(),
            config.clone(),
        )
    }
    fn remove_stage(&self, job: &Job) {
        if filesystem::contains(&self.state.join("staging"), Path::new(&job.stage)) {
            let _ = std::fs::remove_file(&job.stage);
        }
    }
    fn discard_job(&self, job: &Job) -> Result<()> {
        self.store.lock().unwrap().delete_job(&job.item_id)?;
        self.remove_stage(job);
        Ok(())
    }
    async fn upload_item(
        &self,
        config: &Config,
        root: &Root,
        item: &Item,
        job: &mut Job,
    ) -> Result<Version> {
        staging::verify_stage(job, &self.state)?;
        if let Some(version) = &item.current {
            anyhow::ensure!(
                self.drive
                    .metadata(&version.remote_id)
                    .await?
                    .is_some_and(|m| m.matches_version(version)),
                "Cloud conflict: existing backup changed. Review the file before continuing."
            );
        }
        if job.parent_id.is_none() {
            job.parent_id = Some(self.destination(config, root, &item.relative_path).await?);
            self.store.lock().unwrap().put_job(job)?;
        }
        let name = snapshot_name(&item.relative_path, &job.id);
        self.transfer(job, &name, &config.device_id, &item.id, false)
            .await?;
        Ok(Version {
            remote_id: job.target_id.clone().unwrap(),
            sha256: job.sha256.clone(),
            md5: job.md5.clone(),
            size: job.size,
            verified_at: now(),
        })
    }
    async fn transfer(
        &self,
        job: &mut Job,
        name: &str,
        device: &str,
        item: &str,
        catalog: bool,
    ) -> Result<()> {
        if job.target_id.is_none() {
            job.target_id = Some(self.drive.generate_id().await?);
            self.persist_job(job, catalog)?;
        }
        let target = job.target_id.clone().unwrap();
        if let Some(meta) = self.drive.metadata(&target).await? {
            anyhow::ensure!(
                meta.matches(&job.sha256, &job.md5, job.size)
                    && meta.app_properties.get("omniDevice").map(String::as_str) == Some(device)
                    && meta.app_properties.get("omniItem").map(String::as_str) == Some(item),
                "Existing upload ID does not match this snapshot"
            );
            return Ok(());
        }
        if job.size == 0 {
            let meta = self
                .drive
                .empty_file(
                    &target,
                    name,
                    job.parent_id.as_deref().context("Missing upload parent")?,
                    device,
                    item,
                )
                .await?;
            anyhow::ensure!(
                meta.matches(&job.sha256, &job.md5, 0),
                "Empty file verification failed"
            );
            return Ok(());
        }
        let mut completed = false;
        if let Some(encrypted) = &job.session {
            let url = String::from_utf8(security::unprotect(encrypted)?)?;
            match self.drive.upload(&url, 0, job.size, None).await? {
                UploadReply::Progress(offset) => job.offset = offset,
                UploadReply::Complete(meta) => {
                    anyhow::ensure!(
                        meta.matches(&job.sha256, &job.md5, job.size),
                        "Remote checksum mismatch"
                    );
                    completed = true;
                }
                UploadReply::Expired => {
                    job.session = None;
                    job.offset = 0;
                }
            }
            self.persist_job(job, catalog)?;
        }
        if !completed && job.session.is_none() {
            let url = self
                .drive
                .start_upload(
                    &target,
                    false,
                    name,
                    job.parent_id.as_deref().context("Upload parent missing")?,
                    job.size,
                    device,
                    item,
                )
                .await?;
            job.session = Some(security::protect(url.as_bytes())?);
            job.offset = 0;
            self.persist_job(job, catalog)?;
        }
        let mut input = File::open(&job.stage)?;
        while !completed {
            self.permitted()?;
            if !catalog {
                let c = self.config()?;
                let current = self
                    .store
                    .lock()
                    .unwrap()
                    .item(item)?
                    .context("File missing")?;
                anyhow::ensure!(
                    c.roots.iter().any(|r| r.id == current.root_id && r.enabled)
                        && Matcher::new(&c.rules)?
                            .decide(&current.root_id, &current.relative_path)?
                            .included,
                    "File was excluded; upload paused"
                );
            }
            let config = self.config()?;
            anyhow::ensure!(
                !config.pause_on_metered || !metered(),
                "Waiting for an unmetered connection"
            );
            let remaining = job.size.saturating_sub(job.offset);
            let length = remaining.min(CHUNK as u64) as usize;
            input.seek(SeekFrom::Start(job.offset))?;
            let mut buffer = vec![0u8; length];
            input.read_exact(&mut buffer)?;
            let url = String::from_utf8(security::unprotect(job.session.as_ref().unwrap())?)?;
            let start = std::time::Instant::now();
            let previous = job.offset;
            match self
                .drive
                .upload(&url, job.offset, job.size, Some(buffer))
                .await?
            {
                UploadReply::Progress(offset) => {
                    anyhow::ensure!(
                        offset > previous || job.size == 0,
                        "Upload made no progress"
                    );
                    job.offset = offset;
                }
                UploadReply::Complete(meta) => {
                    anyhow::ensure!(
                        meta.matches(&job.sha256, &job.md5, job.size),
                        "Remote checksum mismatch; backup is not marked complete"
                    );
                    job.offset = job.size;
                    completed = true;
                }
                UploadReply::Expired => {
                    job.session = None;
                    job.offset = 0;
                    self.persist_job(job, catalog)?;
                    bail!("Upload session expired; a fresh session will be reconciled on retry");
                }
            }
            self.persist_job(job, catalog)?;
            self.progress.lock().unwrap().sent = job.offset;
            if let Some(kib) = config.upload_limit_kib {
                let target = Duration::from_secs_f64(length as f64 / (kib.max(1) as f64 * 1024.0));
                while start.elapsed() < target {
                    self.permitted()?;
                    tokio::time::sleep(
                        target
                            .saturating_sub(start.elapsed())
                            .min(Duration::from_millis(250)),
                    )
                    .await;
                }
            }
        }
        let meta = self
            .drive
            .metadata(&target)
            .await?
            .context("Uploaded file cannot be verified")?;
        anyhow::ensure!(
            meta.matches(&job.sha256, &job.md5, job.size),
            "Final remote verification failed"
        );
        Ok(())
    }
    fn persist_job(&self, job: &Job, catalog: bool) -> Result<()> {
        if catalog {
            self.store.lock().unwrap().set("catalogJob", &Some(job))
        } else {
            self.store.lock().unwrap().put_job(job)
        }
    }
    async fn save_catalog(self: &Arc<Self>) -> Result<()> {
        if !self
            .store
            .lock()
            .unwrap()
            .setting::<bool>("catalogDirty")?
            .unwrap_or(false)
        {
            return Ok(());
        }
        self.permitted()?;
        let config = self.config()?;
        let parent = self
            .account()?
            .device_folder_id
            .context("Device folder is missing")?;
        let saved = self
            .store
            .lock()
            .unwrap()
            .setting::<Option<Job>>("catalogJob")?
            .flatten();
        let mut job = if let Some(job) = saved {
            job
        } else {
            let catalog = Catalog {
                schema_version: 1,
                device_id: config.device_id.clone(),
                committed_at: now(),
                roots: config
                    .roots
                    .iter()
                    .map(|r| CatalogRoot {
                        id: r.id.clone(),
                        label: r.label.clone(),
                        is_file: r.is_file,
                    })
                    .collect(),
                items: self.items()?,
            };
            let bytes = serde_json::to_vec_pretty(&catalog)?;
            let id = id();
            let path = self.state.join("staging").join(format!("{id}.catalog"));
            std::fs::create_dir_all(path.parent().unwrap())?;
            filesystem::write_durable(&path, &bytes)?;
            let (sha, md5, size) = staging::hashes(bytes.as_slice())?;
            let job = Job {
                id: id.clone(),
                item_id: "catalog".into(),
                generation: 0,
                config_revision: config.revision,
                stage: path.to_string_lossy().into(),
                size,
                sha256: sha,
                md5,
                target_id: None,
                parent_id: Some(parent),
                history_id: None,
                history_saved: false,
                session: None,
                offset: 0,
                phase: "catalog".into(),
                attempts: 0,
                retry_at: 0,
            };
            self.persist_job(&job, true)?;
            job
        };
        let name = format!("OmniSync-catalog-{}.json", job.id);
        self.transfer(&mut job, &name, &config.device_id, "catalog", true)
            .await?;
        self.drive
            .mark_catalog(job.target_id.as_ref().unwrap())
            .await?;
        let catalog: Catalog = serde_json::from_reader(File::open(&job.stage)?)?;
        self.store.lock().unwrap().commit_catalog(
            &catalog,
            job.target_id.as_ref().unwrap(),
            now(),
        )?;
        self.remove_stage(&job);
        self.progress.lock().unwrap().last_backup = Some(now());
        let engine = self.clone();
        tokio::task::spawn_blocking(move || engine.reconcile_cataloged()).await??;
        Ok(())
    }
    fn reconcile_cataloged(&self) -> Result<()> {
        let config = self.config()?;
        let matcher = Matcher::new(&config.rules)?;
        // Recheck only newly uploaded sources, not the entire tree on an async thread.
        for mut item in self.items()?.into_iter().filter(|i| {
            i.status == Status::Pending
                && i.reason == "Upload verified; publishing recovery catalog"
        }) {
            let Some(version) = &item.current else {
                continue;
            };
            if item.cataloged_remote_id.as_deref() != Some(&version.remote_id) {
                continue;
            }
            let Some(root) = config
                .roots
                .iter()
                .find(|r| r.id == item.root_id && r.enabled)
            else {
                continue;
            };
            if !matcher.decide(&root.id, &item.relative_path)?.included {
                continue;
            }
            let path = if root.is_file {
                PathBuf::from(&root.path)
            } else {
                Path::new(&root.path).join(filesystem::safe_relative(&item.relative_path)?)
            };
            let result = (|| -> Result<bool> {
                let (_, identity, _) =
                    filesystem::validate_root(Path::new(&root.path), &self.state)?;
                anyhow::ensure!(identity == root.volume_identity, "Source root changed");
                let (sha, _, size) =
                    staging::hashes(filesystem::locked_source(&path, Path::new(&root.path))?)?;
                Ok(sha == version.sha256 && size == version.size)
            })();
            match result {
                Ok(true) => {
                    item.status = Status::BackedUp;
                    item.reason = "Verified backup matches local content".into();
                }
                Ok(false) => {
                    item.generation += 1;
                    item.reason = "Local content changed; waiting for verified backup".into();
                }
                Err(e) => {
                    item.status = Status::Issue;
                    item.reason = e.to_string();
                }
            }
            self.store.lock().unwrap().put_item(&item)?;
        }
        self.publish_cache()
    }
    pub async fn catalogs(&self) -> Result<Value> {
        Ok(serde_json::to_value(self.drive.list_catalogs().await?)?)
    }
    pub async fn read_catalog(&self, id: &str) -> Result<Catalog> {
        let meta = self
            .drive
            .metadata(id)
            .await?
            .context("Catalog is unavailable")?;
        anyhow::ensure!(
            meta.app_properties.get("omniRole").map(String::as_str) == Some("catalog")
                && meta
                    .size
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(u64::MAX)
                    <= 64 * 1024 * 1024,
            "Not a supported OmniSync catalog (maximum 64 MiB)"
        );
        let path = self
            .state
            .join(format!("catalog-{}.tmp", crate::model::id()));
        let result = async {
            self.drive.download(id, &path, 64 * 1024 * 1024).await?;
            let catalog: Catalog = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(catalog.schema_version == 1, "Unsupported catalog version");
            for item in &catalog.items {
                filesystem::safe_relative(&item.relative_path)?;
            }
            Ok(catalog)
        }
        .await;
        let _ = std::fs::remove_file(path);
        result
    }
    pub async fn restore(
        &self,
        version: Version,
        relative: String,
        destination: PathBuf,
    ) -> Result<String> {
        filesystem::no_reparse(&destination)?;
        let destination = std::fs::canonicalize(&destination)?;
        for root in self.config()?.roots {
            anyhow::ensure!(
                !filesystem::contains(Path::new(&root.path), &destination),
                "Choose a restore location outside every selected backup folder"
            );
        }
        anyhow::ensure!(
            !filesystem::contains(&self.state, &destination),
            "Cannot restore into application state"
        );
        let relative = filesystem::safe_relative(&relative)?;
        let target = destination.join(relative);
        anyhow::ensure!(
            filesystem::contains(&destination, &target),
            "Restore escaped its destination"
        );
        let parent_owned = target
            .parent()
            .context("Invalid restore path")?
            .to_path_buf();
        let parent = parent_owned.as_path();
        for root in self.config()?.roots {
            anyhow::ensure!(
                !filesystem::contains(Path::new(&root.path), &target),
                "Restore target lies inside a protected folder"
            );
        }
        let _parent_handles = filesystem::guarded_directories(parent)?;
        let original_name = target
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let extension = target
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut target = target;
        let mut n = 1;
        while target.exists() {
            target = parent.join(format!("{original_name} (restored {n}){extension}"));
            n += 1;
            anyhow::ensure!(n < 1000, "Too many conflicting restore names");
        }
        let partial = parent.join(format!(".omnisync-{}.partial", id()));
        let result = async {
            let remote = self
                .drive
                .metadata(&version.remote_id)
                .await?
                .context("Backup version is unavailable")?;
            anyhow::ensure!(
                remote.matches_version(&version),
                "Cloud version changed; restore stopped"
            );
            self.drive
                .download(&version.remote_id, &partial, version.size)
                .await?;
            let (sha, md5, size) = staging::hashes(File::open(&partial)?)?;
            anyhow::ensure!(
                sha == version.sha256 && md5 == version.md5 && size == version.size,
                "Restored bytes failed integrity verification"
            );
            filesystem::no_reparse(parent)?;
            anyhow::ensure!(
                !target.exists(),
                "Restore destination appeared during download; try again"
            );
            filesystem::move_new(&partial, &target)?;
            Ok(filesystem::display(&target))
        }
        .await;
        let _ = std::fs::remove_file(partial);
        result
    }
}
fn walk_root(root: &Root, mut visit: impl FnMut(&Path, &str) -> Result<()>) -> Result<()> {
    if root.is_file {
        let path = Path::new(&root.path);
        return visit(
            path,
            &path
                .file_name()
                .context("Invalid filename")?
                .to_string_lossy(),
        );
    }
    for entry in walkdir::WalkDir::new(&root.path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || !e.file_type().is_dir() || filesystem::eligible(e.path()).is_ok()
        })
    {
        let entry = entry?;
        if entry.depth() == 0
            || entry.file_type().is_dir() && filesystem::eligible(entry.path()).is_ok()
        {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&root.path)?
            .to_string_lossy()
            .replace('\\', "/");
        visit(entry.path(), &relative)?;
    }
    Ok(())
}
fn aggregate(current: u32, next: u32) -> u32 {
    fn priority(state: u32) -> u8 {
        match state {
            5 => 5,
            2 => 4,
            3 => 3,
            1 => 2,
            4 => 1,
            _ => 0,
        }
    }
    if priority(next) > priority(current) {
        next
    } else {
        current
    }
}
fn metered() -> bool {
    let initialized = unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
        .is_ok()
    };
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let result = (|| -> windows::core::Result<bool> {
        let profile = NetworkInformation::GetInternetConnectionProfile()?;
        let cost = profile.GetConnectionCost()?;
        Ok(cost.NetworkCostType()? != NetworkCostType::Unrestricted
            || cost.Roaming()?
            || cost.OverDataLimit()?)
    })();
    if initialized {
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
    result.unwrap_or(true)
}
fn snapshot_name(relative: &str, id: &str) -> String {
    let path = Path::new(relative);
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    format!(
        "{name} [{}-{}]{extension}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        &id[..8]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_badges_prefer_actionable_states_and_do_not_let_exclusions_hide_backups() {
        assert_eq!(aggregate(1, 4), 1);
        assert_eq!(aggregate(4, 3), 3);
        assert_eq!(aggregate(3, 2), 2);
        assert_eq!(aggregate(2, 5), 5);
    }
    pub(super) fn fixture() -> (tempfile::TempDir, Arc<Engine>, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("keep.txt"), b"original").unwrap();
        std::fs::write(source.join("skip.tmp"), b"temporary").unwrap();
        let engine =
            Engine::open_with_cache(temp.path().join("state"), Cache::isolated().unwrap()).unwrap();
        (temp, engine, source)
    }
    pub(super) fn select(engine: &Engine, source: &Path) {
        let mut config = engine.config().unwrap();
        config.roots.push(Root {
            id: "root".into(),
            label: "Test source".into(),
            path: filesystem::display(source),
            volume_identity: String::new(),
            enabled: true,
            is_file: false,
        });
        config.rules.push(Rule {
            id: "rule".into(),
            scope: "global".into(),
            kind: RuleKind::FilenameGlob,
            action: Action::Exclude,
            value: "*.tmp".into(),
            order: 1,
        });
        let preview = engine.preview(config, "Test selection".into()).unwrap();
        assert_eq!((preview.included, preview.excluded), (1, 1));
        engine.apply_preview(&preview.token).unwrap();
    }
    #[tokio::test]
    async fn pause_is_persistent_and_sync_now_cannot_bypass_it() {
        let (temp, engine, _) = fixture();
        engine.pause(Some(0)).unwrap();
        assert!(engine.sync_now().is_err());
        assert_eq!(engine.snapshot().await.unwrap().progress.phase, "paused");
        drop(engine);
        let reopened =
            Engine::open_with_cache(temp.path().join("state"), Cache::isolated().unwrap()).unwrap();
        assert!(reopened.config().unwrap().paused());
        reopened.pause(None).unwrap();
        assert!(reopened.sync_now().is_ok());
    }
    #[test]
    fn stale_preview_does_not_overwrite_newer_configuration() {
        let (_temp, engine, source) = fixture();
        select(&engine, &source);
        let preview = engine
            .preview(engine.config().unwrap(), "Old review".into())
            .unwrap();
        engine.pause(Some(0)).unwrap();
        assert!(engine.apply_preview(&preview.token).is_err());
        assert!(engine.config().unwrap().paused());
    }
    #[test]
    fn scan_verifies_content_even_when_size_and_timestamp_are_preserved() {
        let (_temp, engine, source) = fixture();
        select(&engine, &source);
        engine.scan().unwrap();
        let file = source.join("keep.txt");
        let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut item = engine
            .items()
            .unwrap()
            .into_iter()
            .find(|i| i.relative_path == "keep.txt")
            .unwrap();
        let (sha, md5, size) = staging::hashes(File::open(&file).unwrap()).unwrap();
        item.current = Some(Version {
            remote_id: "test-id".into(),
            sha256: sha,
            md5,
            size,
            verified_at: now(),
        });
        item.status = Status::BackedUp;
        engine.store.lock().unwrap().put_item(&item).unwrap();
        std::fs::write(&file, b"modified").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        engine.scan().unwrap();
        let changed = engine
            .store
            .lock()
            .unwrap()
            .item(&item.id)
            .unwrap()
            .unwrap();
        assert_eq!(changed.status, Status::Pending);
        assert!(changed.generation > item.generation);
        assert_eq!(
            engine
                .items()
                .unwrap()
                .iter()
                .find(|i| i.relative_path == "skip.tmp")
                .unwrap()
                .status,
            Status::Excluded
        );
    }
    #[test]
    fn absent_local_files_are_retained() {
        let (_temp, engine, source) = fixture();
        select(&engine, &source);
        engine.scan().unwrap();
        std::fs::remove_file(source.join("keep.txt")).unwrap();
        engine.scan().unwrap();
        let item = engine
            .items()
            .unwrap()
            .into_iter()
            .find(|i| i.relative_path == "keep.txt")
            .unwrap();
        assert_eq!(item.status, Status::Retained);
    }
    #[tokio::test]
    async fn catalog_checkpoints_advance_counts_without_demoting_covered_files() {
        let (_temp, engine, source) = fixture();
        select(&engine, &source);
        std::fs::write(source.join("second.txt"), b"second").unwrap();
        engine.scan().unwrap();
        let mut items = engine
            .items()
            .unwrap()
            .into_iter()
            .filter(|i| i.status != Status::Excluded)
            .collect::<Vec<_>>();
        for (index, item) in items.iter_mut().enumerate() {
            let (sha, md5, size) =
                staging::hashes(File::open(source.join(&item.relative_path)).unwrap()).unwrap();
            item.current = Some(Version {
                remote_id: format!("remote-{index}"),
                sha256: sha,
                md5,
                size,
                verified_at: now(),
            });
            item.status = Status::Pending;
            item.reason = "Upload verified; publishing recovery catalog".into();
            engine.store.lock().unwrap().commit_verified(item).unwrap();
        }
        let mut catalog = Catalog {
            schema_version: 1,
            device_id: engine.config().unwrap().device_id,
            committed_at: now(),
            roots: vec![],
            items: vec![items[0].clone()],
        };
        engine
            .store
            .lock()
            .unwrap()
            .commit_catalog(&catalog, "catalog-1", now())
            .unwrap();
        engine.reconcile_cataloged().unwrap();
        let counts = engine.snapshot().await.unwrap().counts;
        assert_eq!((counts.backed_up, counts.pending), (1, 1));
        assert!(engine
            .store
            .lock()
            .unwrap()
            .setting::<bool>("catalogDirty")
            .unwrap()
            .unwrap());
        catalog.items = items.clone();
        engine
            .store
            .lock()
            .unwrap()
            .commit_catalog(&catalog, "catalog-2", now())
            .unwrap();
        engine.reconcile_cataloged().unwrap();
        let counts = engine.snapshot().await.unwrap().counts;
        assert_eq!((counts.backed_up, counts.pending), (2, 0));
        // A resumed old catalog cannot cover a newer version uploaded since it was staged.
        let mut newer = engine
            .store
            .lock()
            .unwrap()
            .item(&items[1].id)
            .unwrap()
            .unwrap();
        newer.current.as_mut().unwrap().remote_id = "newer-copy".into();
        newer.status = Status::Pending;
        engine
            .store
            .lock()
            .unwrap()
            .commit_verified(&newer)
            .unwrap();
        engine
            .store
            .lock()
            .unwrap()
            .commit_catalog(&catalog, "old-resumed-catalog", now())
            .unwrap();
        assert_eq!(engine.snapshot().await.unwrap().counts.backed_up, 1);
        assert!(engine
            .store
            .lock()
            .unwrap()
            .setting::<bool>("catalogDirty")
            .unwrap()
            .unwrap());
    }
    #[test]
    fn cleanup_commits_only_the_selected_version_and_survives_restart() {
        let (temp, engine, source) = fixture();
        select(&engine, &source);
        engine.scan().unwrap();
        let mut item = engine
            .items()
            .unwrap()
            .into_iter()
            .find(|i| i.relative_path == "skip.tmp")
            .unwrap();
        let version = Version {
            remote_id: "current".into(),
            sha256: "hash".into(),
            md5: "md5".into(),
            size: 5,
            verified_at: now(),
        };
        item.current = Some(version.clone());
        item.cataloged_remote_id = Some("current".into());
        let mut older = version;
        older.remote_id = "history".into();
        item.history.push(older);
        {
            let mut store = engine.store.lock().unwrap();
            store.put_item(&item).unwrap();
            store
                .record_cleanup("run", &item.id, "history", "requested")
                .unwrap();
            store.commit_cleanup("run", &item.id, "history").unwrap();
        }
        drop(engine);
        let store = Store::open(&temp.path().join("state/state.sqlite")).unwrap();
        let saved = store.item(&item.id).unwrap().unwrap();
        assert_eq!(saved.current.unwrap().remote_id, "current");
        assert!(saved.history.is_empty());
        assert!(store.setting::<bool>("catalogDirty").unwrap().unwrap());
        assert!(source.join("skip.tmp").exists());
    }
    #[test]
    fn restore_publish_never_overwrites_existing_destination() {
        let (temp, _engine, _) = fixture();
        let original = temp.path().join("original.txt");
        let downloaded = temp.path().join("download.tmp");
        filesystem::write_durable(&original, b"keep me").unwrap();
        filesystem::write_durable(&downloaded, b"restore").unwrap();
        assert!(filesystem::move_new(&downloaded, &original).is_err());
        assert_eq!(std::fs::read(original).unwrap(), b"keep me");
    }
}
