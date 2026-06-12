use histq::db::Db;
use histq::import::{import_file, parse_history};

#[test]
fn parses_extended_zsh_format() {
    let text = ": 1612345678:0;cargo build\n: 1612345700:12;cargo test\n";
    let entries = parse_history(text);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].command, "cargo build");
    assert_eq!(entries[0].started_at, Some(1_612_345_678_000));
    assert_eq!(entries[1].duration_ms, Some(12_000));
}

#[test]
fn parses_plain_format() {
    let text = "ls -la\ngit status\n\n";
    let entries = parse_history(text);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].command, "ls -la");
    assert_eq!(entries[0].started_at, None);
}

#[test]
fn joins_backslash_continuations() {
    let text = ": 1612345678:0;echo one \\\ntwo\nls\n";
    let entries = parse_history(text);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].command, "echo one \ntwo");
    assert_eq!(entries[1].command, "ls");
}

#[test]
fn mixed_formats_in_one_file() {
    let text = "plain command\n: 1612345678:0;extended command\n";
    let entries = parse_history(text);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].started_at, None);
    assert_eq!(entries[1].started_at, Some(1_612_345_678_000));
}

#[test]
fn plain_entries_stay_idempotent_across_runs_with_different_clocks() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("h.db")).unwrap();
    let file = dir.path().join("history");
    // Plain format: no timestamps in the file.
    std::fs::write(&file, "ls -la\ngit status\nls -la\n").unwrap();

    // First run: duplicate lines within the file collapse too.
    let report = import_file(&db, &file, histq::redact::redact, 1_700_000_000_000).unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(report.skipped, 1);

    // Second run with a *different* clock: nothing new.
    let report = import_file(&db, &file, histq::redact::redact, 1_700_000_999_999).unwrap();
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 3);
}

#[test]
fn import_is_idempotent_and_redacts() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("h.db")).unwrap();
    let file = dir.path().join("zsh_history");
    std::fs::write(
        &file,
        ": 1612345678:0;export API_KEY=secret123\n: 1612345680:1;ls\n",
    )
    .unwrap();

    let now = 1_700_000_000_000;
    let report = import_file(&db, &file, histq::redact::redact, now).unwrap();
    assert_eq!(report.imported, 2);
    assert_eq!(report.skipped, 0);

    // Second run: everything already present.
    let report = import_file(&db, &file, histq::redact::redact, now).unwrap();
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped, 2);

    // The secret was redacted before storage.
    let rows = db
        .candidates(&histq::search::ParsedQuery::default(), 10, true)
        .unwrap();
    let api_key_row = rows.iter().find(|c| c.command.contains("API_KEY")).unwrap();
    assert!(!api_key_row.command.contains("secret123"));
    assert!(api_key_row.command.contains("***REDACTED***"));
    // Imported rows are reachable by arrow navigation (exit_code set).
    assert_eq!(api_key_row.exit_code, Some(0));
}
