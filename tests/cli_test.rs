use assert_cmd::Command;
use predicates::prelude::*;

fn histq(db: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("histq").unwrap();
    cmd.env("HISTQ_DB", db);
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
        .args(["record-start", "--session", "t1", "--", "cargo build --release"])
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
