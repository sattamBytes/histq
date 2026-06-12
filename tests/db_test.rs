use histq::db::{Db, NewEntry};
use histq::search::{parse_query, ParsedQuery};

fn temp_db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("history.db")).unwrap();
    (dir, db)
}

fn entry(command: &str, session: &str, started_at: i64) -> NewEntry {
    NewEntry {
        command: command.to_string(),
        cwd: "/home/raj/project".to_string(),
        git_repo: Some("/home/raj/project".to_string()),
        git_branch: Some("main".to_string()),
        started_at,
        tags: String::new(),
        session_id: session.to_string(),
    }
}

#[test]
fn record_start_and_end_roundtrip() {
    let (_dir, db) = temp_db();
    db.record_start(&entry("cargo build", "s1", 1_000)).unwrap();
    assert!(db.record_end("s1", 0, 3_500).unwrap());

    let rows = db.candidates(&ParsedQuery::default(), 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "cargo build");
    assert_eq!(rows[0].exit_code, Some(0));
    assert_eq!(rows[0].duration_ms, Some(2_500));
}

#[test]
fn record_end_without_pending_command_is_a_noop() {
    let (_dir, db) = temp_db();
    assert!(!db.record_end("s1", 0, 1_000).unwrap());
}

#[test]
fn record_end_only_touches_its_own_session() {
    let (_dir, db) = temp_db();
    db.record_start(&entry("sleep 100", "tab-a", 1_000))
        .unwrap();
    db.record_start(&entry("ls", "tab-b", 2_000)).unwrap();
    assert!(db.record_end("tab-b", 0, 2_100).unwrap());

    let rows = db.candidates(&ParsedQuery::default(), 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "ls");
}

#[test]
fn record_end_finalizes_the_newest_pending_command() {
    let (_dir, db) = temp_db();
    // An older pending row (e.g. the shell crashed mid-command) must not
    // swallow the exit code of the command that just finished.
    db.record_start(&entry("orphaned", "s1", 1_000)).unwrap();
    db.record_start(&entry("echo hi", "s1", 5_000)).unwrap();
    assert!(db.record_end("s1", 0, 5_100).unwrap());

    let rows = db.candidates(&ParsedQuery::default(), 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "echo hi");
}

#[test]
fn completed_only_excludes_pending_rows() {
    let (_dir, db) = temp_db();
    db.record_start(&entry("still running", "s1", 1_000))
        .unwrap();
    assert!(db
        .candidates(&ParsedQuery::default(), 10, true)
        .unwrap()
        .is_empty());
    assert_eq!(
        db.candidates(&ParsedQuery::default(), 10, false)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fts_index_stays_in_sync_through_insert_and_update() {
    let (_dir, db) = temp_db();
    db.record_start(&entry("docker compose up", "s1", 1_000))
        .unwrap();
    db.record_end("s1", 0, 2_000).unwrap();

    let q = parse_query("docker", chrono::Local::now());
    let rows = db.candidates(&q, 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "docker compose up");
    assert!(rows[0].fts_rank.is_some());

    let q = parse_query("kubernetes", chrono::Local::now());
    assert!(db.candidates(&q, 10, true).unwrap().is_empty());
}

#[test]
fn exit_filters_apply() {
    let (_dir, db) = temp_db();
    db.record_start(&entry("make test", "s1", 1_000)).unwrap();
    db.record_end("s1", 2, 2_000).unwrap();
    db.record_start(&entry("make build", "s1", 3_000)).unwrap();
    db.record_end("s1", 0, 4_000).unwrap();

    let failed = parse_query("failed", chrono::Local::now());
    let rows = db.candidates(&failed, 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "make test");

    let worked = parse_query("worked", chrono::Local::now());
    let rows = db.candidates(&worked, 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "make build");
}

#[test]
fn tag_filter_matches_whole_tags_only() {
    let (_dir, db) = temp_db();
    let mut tagged = entry("cargo publish # release", "s1", 1_000);
    tagged.tags = "release".to_string();
    db.record_start(&tagged).unwrap();
    db.record_end("s1", 0, 2_000).unwrap();

    let mut other = entry("echo prerelease", "s1", 3_000);
    other.tags = "prerelease".to_string();
    db.record_start(&other).unwrap();
    db.record_end("s1", 0, 4_000).unwrap();

    let q = parse_query("#release", chrono::Local::now());
    let rows = db.candidates(&q, 10, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].command, "cargo publish # release");
}
