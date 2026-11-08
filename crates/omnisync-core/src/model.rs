use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub id: String,
    pub label: String,
    pub path: String,
    pub volume_identity: String,
    pub enabled: bool,
    pub is_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleKind {
    ExactFile,
    Subtree,
    FilenameGlob,
    PathGlob,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Include,
    Exclude,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub scope: String,
    pub kind: RuleKind,
    pub action: Action,
    pub value: String,
    pub order: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub revision: u64,
    pub device_id: String,
    pub roots: Vec<Root>,
    pub rules: Vec<Rule>,
    pub paused_until: Option<i64>,
    pub continuous: bool,
    pub pause_on_metered: bool,
    pub prior_versions: usize,
    pub staging_budget_bytes: u64,
    pub free_space_reserve_bytes: u64,
    pub upload_limit_kib: Option<u64>,
    pub start_at_sign_in: bool,
    pub theme: String,
    pub destination: Option<Destination>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            revision: 1,
            device_id: id(),
            roots: vec![],
            rules: vec![],
            paused_until: None,
            continuous: true,
            pause_on_metered: true,
            prior_versions: 10,
            staging_budget_bytes: 50 * 1024 * 1024 * 1024,
            free_space_reserve_bytes: 5 * 1024 * 1024 * 1024,
            upload_limit_kib: None,
            start_at_sign_in: false,
            theme: "system".into(),
            destination: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    pub path: String,
    pub id: String,
}
impl Config {
    pub fn paused(&self) -> bool {
        self.paused_until.is_some_and(|t| t == 0 || t > now())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Syncing,
    BackedUp,
    Excluded,
    Issue,
    Retained,
}
impl Status {
    pub fn overlay(self) -> u32 {
        match self {
            Self::BackedUp => 1,
            Self::Syncing => 2,
            Self::Pending => 3,
            Self::Excluded => 4,
            Self::Issue => 5,
            Self::Retained => 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub remote_id: String,
    pub sha256: String,
    pub md5: String,
    pub size: u64,
    pub verified_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub root_id: String,
    pub relative_path: String,
    pub size: u64,
    pub modified: u64,
    pub generation: u64,
    pub status: Status,
    pub reason: String,
    pub winning_rule: Option<String>,
    pub current: Option<Version>,
    /// The current remote version covered by a successfully published recovery catalog.
    #[serde(default)]
    pub cataloged_remote_id: Option<String>,
    pub history: Vec<Version>,
    pub last_seen: i64,
}
impl Item {
    pub fn effective_status(&self) -> Status {
        if self.status == Status::BackedUp
            && !self
                .current
                .as_ref()
                .is_some_and(|v| self.cataloged_remote_id.as_deref() == Some(&v.remote_id))
        {
            Status::Pending
        } else {
            self.status
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub item_id: String,
    pub generation: u64,
    pub config_revision: u64,
    pub stage: String,
    pub size: u64,
    pub sha256: String,
    pub md5: String,
    pub target_id: Option<String>,
    pub parent_id: Option<String>,
    pub history_id: Option<String>,
    pub history_saved: bool,
    pub session: Option<Vec<u8>>,
    pub offset: u64,
    pub phase: String,
    pub attempts: u32,
    pub retry_at: i64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub connected: bool,
    pub display_name: String,
    pub email: String,
    pub permission_id: String,
    pub base_id: Option<String>,
    pub device_folder_id: Option<String>,
    pub catalog_id: Option<String>,
    pub cursor: Option<String>,
    pub quota_used: u64,
    pub quota_limit: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub current_file: String,
    pub sent: u64,
    pub total: u64,
    pub message: String,
    pub last_backup: Option<i64>,
    pub connecting: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub backed_up: usize,
    pub pending: usize,
    pub excluded: usize,
    pub issues: usize,
    pub retained: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub config: Config,
    pub progress: Progress,
    pub counts: Counts,
    pub recent: Vec<Item>,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub device_id: String,
    pub committed_at: i64,
    pub roots: Vec<CatalogRoot>,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRoot {
    pub id: String,
    pub label: String,
    pub is_file: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub token: String,
    pub revision: u64,
    pub description: String,
    pub included: usize,
    pub excluded: usize,
    pub bytes: u64,
    pub examples: Vec<String>,
    pub warnings: Vec<String>,
}
