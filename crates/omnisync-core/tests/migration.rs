use omnisync_core::store::Store;
use rusqlite::Connection;

#[test]
fn legacy_settings_and_catalog_are_preserved_without_false_success() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state.sqlite");
    {
        let mut store = Store::open(&path).unwrap();
        let mut config = store.config().unwrap();
        config.rules.push(omnisync_core::model::Rule {
            id: "excluded-file".into(),
            scope: "global".into(),
            kind: omnisync_core::model::RuleKind::ExactFile,
            action: omnisync_core::model::Action::Exclude,
            value: "keep-private.txt".into(),
            order: 7,
        });
        config.revision += 1;
        store.save_config(&config, 1).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    verify(&path);
}
fn verify(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    let before: String = connection
        .query_row("SELECT value FROM settings WHERE key='config'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let before: serde_json::Value = serde_json::from_str(&before).unwrap();
    drop(connection);
    let store = Store::open(path).unwrap();
    let after = serde_json::to_value(store.config().unwrap()).unwrap();
    for field in [
        "roots",
        "rules",
        "deviceId",
        "revision",
        "pausedUntil",
        "continuous",
        "startAtSignIn",
        "theme",
    ] {
        assert_eq!(
            before.get(field),
            after.get(field),
            "migration changed {field}"
        );
    }
    assert!(store.config().unwrap().destination.is_none());
    assert!(store
        .items()
        .unwrap()
        .iter()
        .all(|i| i.current.is_none() && i.history.is_empty() && i.cataloged_remote_id.is_none()));
    assert!(store.jobs().unwrap().is_empty());
    let conn = Connection::open(path).unwrap();
    let archived: String = conn
        .query_row(
            "SELECT json FROM archived_profiles WHERE id='legacy-google-drive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let archived: serde_json::Value = serde_json::from_str(&archived).unwrap();
    assert_eq!(archived["config"]["roots"], before["roots"]);
    assert_eq!(archived["config"]["rules"], before["rules"]);
    println!(
        "Migration preserved {} sources and {} rules; archived {} legacy item records.",
        after["roots"].as_array().unwrap().len(),
        after["rules"].as_array().unwrap().len(),
        archived["items"].as_array().unwrap().len()
    );
}

/// Only a disposable SQLite backup may be supplied, never a live user database.
#[test]
#[ignore = "requires an explicitly prepared disposable database copy"]
fn migrate_disposable_profile_copy() {
    let path = std::env::var_os("OMNISYNC_MIGRATION_FIXTURE")
        .expect("Path to disposable state.sqlite copy");
    verify(std::path::Path::new(&path));
}
