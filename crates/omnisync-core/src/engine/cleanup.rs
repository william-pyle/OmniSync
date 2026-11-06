use super::*;
use crate::drive::Metadata;
use serde::Serialize;

pub(super) struct CleanupPlan {
    revision: u64,
    expires: i64,
    permission_id: String,
    items: Vec<Item>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupFile {
    item_id: String,
    path: String,
    versions: usize,
    bytes: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPreview {
    token: String,
    revision: u64,
    files: Vec<CleanupFile>,
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    trashed: usize,
    already_absent: usize,
    skipped: Vec<String>,
    failed: Vec<String>,
    catalog_pending: bool,
}
struct CleanupGuard<'a>(&'a Engine);
impl Drop for CleanupGuard<'_> {
    fn drop(&mut self) {
        self.0.cleanup_running.store(false, Ordering::SeqCst);
        self.0.dirty.store(true, Ordering::SeqCst);
        self.0.wake.notify_one();
    }
}
// Only explicit rules or a disabled configured root authorize cleanup. A safety
// exclusion, unavailable source or removed root alone never authorizes it.
fn excluded(config: &Config, item: &Item) -> Result<bool> {
    let Some(root) = config.roots.iter().find(|r| r.id == item.root_id) else {
        return Ok(false);
    };
    Ok(!root.enabled
        || !Matcher::new(&config.rules)?
            .decide(&root.id, &item.relative_path)?
            .included)
}
fn versions(item: &Item) -> Vec<Version> {
    let mut seen = HashSet::new();
    item.current
        .iter()
        .chain(item.history.iter())
        .filter(|v| seen.insert(v.remote_id.clone()))
        .cloned()
        .collect()
}
fn owned_snapshot(meta: &Metadata, version: &Version, device: &str, item: &str) -> bool {
    let mut current = meta.clone();
    current.trashed = false;
    meta.owned_by_me
        && current.matches_version(version)
        && meta.mime_type != "application/vnd.google-apps.folder"
        && meta.app_properties.get("omniDevice").map(String::as_str) == Some(device)
        && meta.app_properties.get("omniItem").map(String::as_str) == Some(item)
        && matches!(
            meta.app_properties.get("omniRole").map(String::as_str),
            Some("current" | "history")
        )
}
impl Engine {
    pub fn preview_cleanup(&self) -> Result<CleanupPreview> {
        let config = self.config()?;
        let permission_id = self.account()?.permission_id;
        anyhow::ensure!(
            !permission_id.is_empty(),
            "Connect Google Drive before reviewing cloud cleanup"
        );
        let items = self
            .items()?
            .into_iter()
            .filter_map(|i| match excluded(&config, &i) {
                Ok(true) if !versions(&i).is_empty() => Some(Ok(i)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<_>>>()?;
        let files = items
            .iter()
            .map(|i| {
                let copies = versions(i);
                CleanupFile {
                    item_id: i.id.clone(),
                    path: format!(
                        "{} / {}",
                        config
                            .roots
                            .iter()
                            .find(|r| r.id == i.root_id)
                            .map(|r| r.label.as_str())
                            .unwrap_or("Source"),
                        i.relative_path
                    ),
                    versions: copies.len(),
                    bytes: copies.iter().map(|v| v.size).sum(),
                }
            })
            .collect();
        let token = id();
        let mut plans = self.cleanup_previews.lock().unwrap();
        plans.retain(|_, p| p.expires > now());
        plans.insert(
            token.clone(),
            CleanupPlan {
                revision: config.revision,
                expires: now() + 600,
                permission_id,
                items,
            },
        );
        Ok(CleanupPreview {
            token,
            revision: config.revision,
            files,
        })
    }
    pub async fn apply_cleanup(&self, token: &str, selected: &[String]) -> Result<CleanupResult> {
        let plan = self
            .cleanup_previews
            .lock()
            .unwrap()
            .remove(token)
            .context("Cleanup review expired; review it again")?;
        anyhow::ensure!(plan.expires > now(), "Cleanup review expired");
        let unique: HashSet<_> = selected.iter().collect();
        anyhow::ensure!(
            !selected.is_empty() && unique.len() == selected.len(),
            "Select one or more distinct reviewed files"
        );
        anyhow::ensure!(
            selected
                .iter()
                .all(|id| plan.items.iter().any(|i| &i.id == id)),
            "Selection was not in this cleanup review"
        );
        {
            let store = self.store.lock().unwrap();
            anyhow::ensure!(
                store.config()?.revision == plan.revision,
                "Exclusions changed; review cleanup again"
            );
            anyhow::ensure!(
                self.cleanup_running
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok(),
                "Cloud cleanup is already running"
            );
        }
        let _cleanup = CleanupGuard(self);
        let _operation = self.run_lock.lock().await;
        anyhow::ensure!(
            plan.expires > now(),
            "Cleanup review expired while waiting for backup; review again"
        );
        let config = self.config()?;
        anyhow::ensure!(
            config.revision == plan.revision,
            "Exclusions changed; review cleanup again"
        );
        self.refresh_account().await?;
        anyhow::ensure!(
            self.account()?.permission_id == plan.permission_id,
            "Google account changed; review cleanup again"
        );
        let obsolete = self
            .store
            .lock()
            .unwrap()
            .setting::<Option<Job>>("catalogJob")?
            .flatten();
        if let Some(job) = obsolete {
            self.store
                .lock()
                .unwrap()
                .set("catalogJob", &Option::<Job>::None)?;
            self.remove_stage(&job);
        }
        let run = id();
        let mut result = CleanupResult::default();
        for reviewed in plan.items.iter().filter(|i| selected.contains(&i.id)) {
            let Some(current) = self.store.lock().unwrap().item(&reviewed.id)? else {
                result.skipped.push(reviewed.relative_path.clone());
                continue;
            };
            if !excluded(&config, &current)? {
                result.skipped.push(current.relative_path.clone());
                continue;
            }
            let current_versions = versions(&current);
            for version in versions(reviewed) {
                if !current_versions
                    .iter()
                    .any(|v| v.remote_id == version.remote_id)
                {
                    result.skipped.push(format!(
                        "{}: copy changed since review",
                        current.relative_path
                    ));
                    continue;
                }
                if self.stopping.load(Ordering::SeqCst) {
                    result
                        .failed
                        .push("OmniSync is closing; remaining copies were not removed".into());
                    return Ok(result);
                }
                let outcome = async {
                    let metadata = self.drive.metadata(&version.remote_id).await?;
                    if let Some(meta) = &metadata {
                        anyhow::ensure!(owned_snapshot(meta, &version, &config.device_id, &current.id),
                            "Copy changed, belongs to another owner/device, or is not an OmniSync file snapshot");
                    }
                    self.store.lock().unwrap().record_cleanup(&run, &current.id, &version.remote_id, "requested")?;
                    let absent = metadata.as_ref().is_none_or(|m| m.trashed);
                    if !absent { self.drive.trash(&version.remote_id).await?; }
                    self.store.lock().unwrap().commit_cleanup(&run, &current.id, &version.remote_id)?;
                    Ok::<bool,anyhow::Error>(absent)
                }.await;
                match outcome {
                    Ok(true) => result.already_absent += 1,
                    Ok(false) => result.trashed += 1,
                    Err(e) => {
                        let _ = self.store.lock().unwrap().record_cleanup(
                            &run,
                            &current.id,
                            &version.remote_id,
                            "needs-review",
                        );
                        result
                            .failed
                            .push(format!("{}: {e}", current.relative_path));
                    }
                }
            }
        }
        result.catalog_pending = self
            .store
            .lock()
            .unwrap()
            .setting::<bool>("catalogDirty")?
            .unwrap_or(false);
        self.publish_cache()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cleanup_partial_failure_retries_only_remaining_copies_and_never_deletes_local_files() {
        use crate::drive::TestExchange;
        use reqwest::Method;
        use serde_json::json;
        let (_temp, engine, source) = super::super::tests::fixture();
        super::super::tests::select(&engine, &source);
        engine.scan().unwrap();
        let mut engine = Arc::try_unwrap(engine).ok().unwrap();
        engine
            .store
            .lock()
            .unwrap()
            .set(
                "account",
                &Account {
                    permission_id: "test-account".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut item = engine
            .items()
            .unwrap()
            .into_iter()
            .find(|i| i.relative_path == "skip.tmp")
            .unwrap();
        let version = |remote: &str| Version {
            remote_id: remote.into(),
            sha256: "hash".into(),
            md5: "md5".into(),
            size: 4,
            verified_at: 0,
        };
        item.current = Some(version("current"));
        item.history = vec![version("history"), version("gone"), version("foreign")];
        engine.store.lock().unwrap().put_item(&item).unwrap();
        let device = engine.config().unwrap().device_id;
        let metadata = |remote: &str, owner: bool| json!({"id":remote,"size":"4","sha256Checksum":"hash","ownedByMe":owner,"mimeType":"application/octet-stream","appProperties":{"omniDevice":device,"omniItem":item.id,"omniRole":"current"}});
        let exchange = |method, path: &str, body, status, response| TestExchange {
            method,
            path: path.into(),
            body,
            status,
            response,
        };
        let about = || {
            exchange(
                Method::GET,
                "/about",
                None,
                200,
                json!({"user":{"permissionId":"test-account"}}),
            )
        };
        engine.drive = Drive::scripted(
            engine.auth.clone(),
            vec![
                about(),
                exchange(
                    Method::GET,
                    "/files/current",
                    None,
                    200,
                    metadata("current", true),
                ),
                exchange(
                    Method::PATCH,
                    "/files/current",
                    Some(json!({"trashed":true})),
                    200,
                    json!({"id":"current","trashed":true}),
                ),
                exchange(
                    Method::GET,
                    "/files/history",
                    None,
                    200,
                    metadata("history", true),
                ),
                exchange(
                    Method::PATCH,
                    "/files/history",
                    Some(json!({"trashed":true})),
                    503,
                    json!({"error":{"message":"Test unavailable"}}),
                ),
                exchange(Method::GET, "/files/gone", None, 404, json!({})),
                exchange(
                    Method::GET,
                    "/files/foreign",
                    None,
                    200,
                    metadata("foreign", false),
                ),
            ],
        )
        .unwrap();
        let review = engine.preview_cleanup().unwrap();
        let result = engine
            .apply_cleanup(&review.token, std::slice::from_ref(&item.id))
            .await
            .unwrap();
        engine.drive.assert_script_consumed();
        assert_eq!(
            (result.trashed, result.already_absent, result.failed.len()),
            (1, 1, 2)
        );
        assert!(result.catalog_pending);
        assert!(!engine.cleanup_running.load(Ordering::SeqCst));
        let saved = engine
            .store
            .lock()
            .unwrap()
            .item(&item.id)
            .unwrap()
            .unwrap();
        assert!(saved.current.is_none());
        assert_eq!(
            saved
                .history
                .iter()
                .map(|v| v.remote_id.as_str())
                .collect::<Vec<_>>(),
            vec!["history", "foreign"]
        );
        // A fresh review retries the failure but never repeats the completed PATCH.
        engine.drive = Drive::scripted(
            engine.auth.clone(),
            vec![
                about(),
                exchange(
                    Method::GET,
                    "/files/history",
                    None,
                    200,
                    metadata("history", true),
                ),
                exchange(
                    Method::PATCH,
                    "/files/history",
                    Some(json!({"trashed":true})),
                    200,
                    json!({"id":"history","trashed":true}),
                ),
                exchange(
                    Method::GET,
                    "/files/foreign",
                    None,
                    200,
                    metadata("foreign", false),
                ),
            ],
        )
        .unwrap();
        let review = engine.preview_cleanup().unwrap();
        assert_eq!(review.files[0].versions, 2);
        let result = engine
            .apply_cleanup(&review.token, std::slice::from_ref(&item.id))
            .await
            .unwrap();
        engine.drive.assert_script_consumed();
        assert_eq!(result.trashed, 1);
        assert_eq!(result.failed.len(), 1);
        assert!(source.join("skip.tmp").is_file());
    }
    #[tokio::test]
    async fn cleanup_refuses_stale_or_unreviewed_selection_before_any_cloud_call() {
        let (_temp, engine, source) = super::super::tests::fixture();
        super::super::tests::select(&engine, &source);
        engine.scan().unwrap();
        engine
            .store
            .lock()
            .unwrap()
            .set(
                "account",
                &Account {
                    permission_id: "test-account".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut item = engine
            .items()
            .unwrap()
            .into_iter()
            .find(|i| i.relative_path == "skip.tmp")
            .unwrap();
        item.current = Some(Version {
            remote_id: "known-copy".into(),
            sha256: "hash".into(),
            md5: "md5".into(),
            size: 1,
            verified_at: 0,
        });
        engine.store.lock().unwrap().put_item(&item).unwrap();
        let preview = engine.preview_cleanup().unwrap();
        assert_eq!(preview.files.len(), 1);
        assert!(engine
            .apply_cleanup(&preview.token, &["unreviewed-id".into()])
            .await
            .unwrap_err()
            .to_string()
            .contains("not in this cleanup review"));
        let preview = engine.preview_cleanup().unwrap();
        engine.pause(Some(0)).unwrap();
        assert!(engine
            .apply_cleanup(&preview.token, &[item.id])
            .await
            .unwrap_err()
            .to_string()
            .contains("Exclusions changed"));
    }
    #[test]
    fn cleanup_requires_an_explicit_exclusion_and_respects_include_overrides() {
        let (_temp, engine, source) = super::super::tests::fixture();
        super::super::tests::select(&engine, &source);
        engine.scan().unwrap();
        let config = engine.config().unwrap();
        let items = engine.items().unwrap();
        let skip = items
            .iter()
            .find(|i| i.relative_path == "skip.tmp")
            .unwrap();
        assert!(excluded(&config, skip).unwrap());
        let mut keep = items
            .iter()
            .find(|i| i.relative_path == "keep.txt")
            .unwrap()
            .clone();
        keep.status = Status::Excluded;
        assert!(!excluded(&config, &keep).unwrap());
        let mut changed = config.clone();
        changed.rules.push(Rule {
            id: "include".into(),
            scope: "root".into(),
            kind: RuleKind::ExactFile,
            action: Action::Include,
            value: "skip.tmp".into(),
            order: 2,
        });
        assert!(!excluded(&changed, skip).unwrap());
        changed.roots.clear();
        assert!(!excluded(&changed, skip).unwrap());
    }
    #[test]
    fn cleanup_rejects_foreign_folders_and_modified_versions() {
        let version = Version {
            remote_id: "id".into(),
            sha256: "hash".into(),
            md5: "md5".into(),
            size: 4,
            verified_at: 0,
        };
        let mut meta = Metadata {
            id: "id".into(),
            owned_by_me: true,
            size: Some("4".into()),
            sha256_checksum: Some("hash".into()),
            mime_type: "application/octet-stream".into(),
            app_properties: HashMap::from([
                ("omniDevice".into(), "device".into()),
                ("omniItem".into(), "item".into()),
                ("omniRole".into(), "current".into()),
            ]),
            ..Default::default()
        };
        assert!(owned_snapshot(&meta, &version, "device", "item"));
        meta.trashed = true;
        assert!(owned_snapshot(&meta, &version, "device", "item"));
        assert!(!owned_snapshot(&meta, &version, "other-device", "item"));
        meta.mime_type = "application/vnd.google-apps.folder".into();
        assert!(!owned_snapshot(&meta, &version, "device", "item"));
        meta.mime_type = "application/octet-stream".into();
        meta.sha256_checksum = Some("changed".into());
        assert!(!owned_snapshot(&meta, &version, "device", "item"));
    }
}
