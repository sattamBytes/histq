use assert_cmd::Command;
use predicates::prelude::*;

fn histq(db: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("histq").unwrap();
    cmd.env("HISTQ_DB", db);
    // Isolate tests from any real user config.
    cmd.env("HISTQ_CONFIG", db.with_extension("no-config.toml"));
    cmd
}

#[test]
fn init_zsh_prints_the_integration_script() {
    let dir = tempfile::tempdir().unwrap();
    histq(&dir.path().join("h.db"))
        .args(["init", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preexec"))
        .stdout(predicate::str::contains("precmd"))
        .stdout(predicate::str::contains("BUFFER"))
        .stdout(predicate::str::contains("CURSOR"))
        .stdout(predicate::str::contains("bindkey"))
        .stdout(predicate::str::contains("zle -N"));
}

#[test]
fn init_rejects_unknown_shells() {
    let dir = tempfile::tempdir().unwrap();
    histq(&dir.path().join("h.db"))
        .args(["init", "fish"])
        .assert()
        .failure();
}

#[test]
fn record_then_navigate_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    histq(&db)
        .args([
            "record-start",
            "--session",
            "t1",
            "--",
            "cargo build --release",
        ])
        .assert()
        .success();
    histq(&db)
        .args(["record-end", "--session", "t1", "--exit-code", "0"])
        .assert()
        .success();

    histq(&db)
        .args(["previous", "--query", "cargo", "--offset", "0"])
        .assert()
        .success()
        .stdout("cargo build --release\n");

    // Out of range: silent exit 1, the widget beeps.
    histq(&db)
        .args(["previous", "--query", "cargo", "--offset", "99"])
        .assert()
        .failure()
        .stdout("");

    histq(&db)
        .args(["next", "--query", "cargo", "--offset", "0"])
        .assert()
        .success()
        .stdout("cargo build --release\n");
}

#[test]
fn search_filters_by_rule_words() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    for (cmd, exit) in [("make test", "2"), ("make build", "0")] {
        histq(&db)
            .args(["record-start", "--session", "t1", "--", cmd])
            .assert()
            .success();
        histq(&db)
            .args(["record-end", "--session", "t1", "--exit-code", exit])
            .assert()
            .success();
    }

    histq(&db)
        .args(["search", "failed"])
        .assert()
        .success()
        .stdout(predicate::str::contains("make test"))
        .stdout(predicate::str::contains("make build").not());

    histq(&db)
        .args(["timeline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("make test"))
        .stdout(predicate::str::contains("make build"));
}

#[test]
fn secrets_are_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    histq(&db)
        .args([
            "record-start",
            "--session",
            "t1",
            "--",
            "curl -H \"Authorization: Bearer abc123\" https://api.example.com",
        ])
        .assert()
        .success();
    histq(&db)
        .args(["record-end", "--session", "t1", "--exit-code", "0"])
        .assert()
        .success();

    histq(&db)
        .args(["previous", "--query", "", "--offset", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("***REDACTED***"))
        .stdout(predicate::str::contains("abc123").not());
}

#[test]
fn delete_by_contains_lists_then_deletes_with_yes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    for cmd in ["echo oops-mypassword123", "ls -la"] {
        histq(&db)
            .args(["record-start", "--session", "t1", "--", cmd])
            .assert()
            .success();
        histq(&db)
            .args(["record-end", "--session", "t1", "--exit-code", "0"])
            .assert()
            .success();
    }

    // Without --yes: lists matches, deletes nothing.
    histq(&db)
        .args(["delete", "--contains", "mypassword123"])
        .assert()
        .success()
        .stdout(predicate::str::contains("re-run with --yes"));
    histq(&db)
        .args(["timeline"])
        .assert()
        .stdout(predicate::str::contains("mypassword123"));

    // With --yes: gone.
    histq(&db)
        .args(["delete", "--contains", "mypassword123", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted 1 entries"));
    histq(&db)
        .args(["timeline"])
        .assert()
        .stdout(predicate::str::contains("mypassword123").not())
        .stdout(predicate::str::contains("ls -la"));
}

#[test]
fn import_command_backfills_history_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");
    let hist = dir.path().join("zsh_history");
    std::fs::write(
        &hist,
        ": 1612345678:0;docker compose up\nplain old command\n",
    )
    .unwrap();

    histq(&db)
        .args(["import", "--file", hist.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 2 entries"));

    // Imported commands are reachable via up-arrow navigation.
    histq(&db)
        .args(["previous", "--query", "docker", "--offset", "0"])
        .assert()
        .success()
        .stdout("docker compose up\n");
}

#[test]
fn stats_reports_counts() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    for (cmd, exit) in [("make test", "1"), ("make test", "0"), ("ls", "0")] {
        histq(&db)
            .args(["record-start", "--session", "t1", "--", cmd])
            .assert()
            .success();
        histq(&db)
            .args(["record-end", "--session", "t1", "--exit-code", exit])
            .assert()
            .success();
    }

    histq(&db)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("commands recorded : 3"))
        .stdout(predicate::str::contains("most used:"))
        .stdout(predicate::str::contains("make test"));
}

#[test]
fn config_weights_are_honored() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "[weights]\nsuccess = 100.0\n").unwrap();

    // Same command text relevance; one failed recently, one succeeded earlier.
    for (cmd, exit) in [
        ("echo succeeded-earlier", "0"),
        ("echo failed-recently", "1"),
    ] {
        let mut c = Command::cargo_bin("histq").unwrap();
        c.env("HISTQ_DB", &db)
            .env("HISTQ_CONFIG", &config)
            .args(["record-start", "--session", "t1", "--", cmd])
            .assert()
            .success();
        let mut c = Command::cargo_bin("histq").unwrap();
        c.env("HISTQ_DB", &db)
            .env("HISTQ_CONFIG", &config)
            .args(["record-end", "--session", "t1", "--exit-code", exit])
            .assert()
            .success();
    }

    // With success weighted at 100, the successful command must win despite
    // being older.
    let mut c = Command::cargo_bin("histq").unwrap();
    c.env("HISTQ_DB", &db)
        .env("HISTQ_CONFIG", &config)
        .args(["previous", "--query", "echo", "--offset", "0"])
        .assert()
        .success()
        .stdout("echo succeeded-earlier\n");
}

#[test]
fn leading_space_commands_are_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("h.db");

    histq(&db)
        .args(["record-start", "--session", "t1", "--", " secret-command"])
        .assert()
        .success();

    histq(&db)
        .args(["timeline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("secret-command").not());
}
