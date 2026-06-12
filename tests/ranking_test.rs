use histq::db::Candidate;
use histq::search::{rank, RankContext};

const DAY_MS: i64 = 86_400_000;
const NOW: i64 = 100 * DAY_MS;

fn candidate(command: &str) -> Candidate {
    Candidate {
        id: 0,
        command: command.to_string(),
        cwd: "/somewhere/else".to_string(),
        git_repo: None,
        git_branch: None,
        started_at: NOW - DAY_MS,
        duration_ms: Some(100),
        exit_code: Some(0),
        tags: String::new(),
        fts_rank: None,
    }
}

fn ctx() -> RankContext {
    RankContext {
        cwd: "/home/raj/project/api".to_string(),
        git_repo: Some("/home/raj/project".to_string()),
        now_ms: NOW,
        query_tags: Vec::new(),
    }
}

#[test]
fn same_repo_beats_other_repo() {
    let mut here = candidate("cargo test");
    here.git_repo = Some("/home/raj/project".to_string());
    let mut elsewhere = candidate("npm test");
    elsewhere.git_repo = Some("/home/raj/other".to_string());

    let ranked = rank(vec![elsewhere, here], &ctx());
    assert_eq!(ranked[0].command, "cargo test");
}

#[test]
fn same_cwd_beats_other_cwd() {
    let mut here = candidate("ls -la");
    here.cwd = "/home/raj/project/api".to_string();
    let elsewhere = candidate("ls -lh");

    let ranked = rank(vec![elsewhere, here], &ctx());
    assert_eq!(ranked[0].command, "ls -la");
}

#[test]
fn recent_beats_old() {
    let mut recent = candidate("git pull");
    recent.started_at = NOW - DAY_MS;
    let mut old = candidate("git fetch");
    old.started_at = NOW - 60 * DAY_MS;

    let ranked = rank(vec![old, recent], &ctx());
    assert_eq!(ranked[0].command, "git pull");
}

#[test]
fn success_beats_failure() {
    let mut ok = candidate("pytest -x");
    ok.exit_code = Some(0);
    let mut failed = candidate("pytest -k broken");
    failed.exit_code = Some(1);

    let ranked = rank(vec![failed, ok], &ctx());
    assert_eq!(ranked[0].command, "pytest -x");
}

#[test]
fn strong_text_match_outweighs_context() {
    // A much better text match (more negative bm25) in another repo should
    // beat a weak match in the current repo: W_TEXT > W_REPO + W_CWD.
    let mut weak_match_here = candidate("grep -r foo");
    weak_match_here.git_repo = Some("/home/raj/project".to_string());
    weak_match_here.cwd = "/home/raj/project/api".to_string();
    weak_match_here.fts_rank = Some(-0.1);

    let mut strong_match_elsewhere = candidate("grep -r foobar baz");
    strong_match_elsewhere.fts_rank = Some(-9.0);

    let ranked = rank(vec![weak_match_here, strong_match_elsewhere], &ctx());
    assert_eq!(ranked[0].command, "grep -r foobar baz");
}

#[test]
fn duplicate_commands_are_deduped_keeping_one() {
    let mut a = candidate("cargo build");
    a.started_at = NOW - DAY_MS;
    let mut b = candidate("cargo build");
    b.started_at = NOW - 10 * DAY_MS;
    let c = candidate("cargo clippy");

    let ranked = rank(vec![a, b, c], &ctx());
    let builds = ranked.iter().filter(|c| c.command == "cargo build").count();
    assert_eq!(builds, 1);
    assert_eq!(ranked.len(), 2);
    // The kept instance is the better-scored (more recent) one.
    assert_eq!(ranked[0].started_at, NOW - DAY_MS);
}

#[test]
fn tag_overlap_boosts_score() {
    let mut tagged = candidate("kubectl apply -f prod.yaml # deploy");
    tagged.tags = "deploy".to_string();
    let untagged = candidate("kubectl get pods");

    let mut context = ctx();
    context.query_tags = vec!["deploy".to_string()];
    let ranked = rank(vec![untagged, tagged], &context);
    assert_eq!(ranked[0].tags, "deploy");
}

#[test]
fn ties_break_by_recency() {
    let mut newer = candidate("echo a");
    newer.started_at = NOW - DAY_MS;
    let mut older = candidate("echo b");
    older.started_at = NOW - DAY_MS - 1;

    // Identical context, near-identical recency: newer first, deterministically.
    let ranked = rank(vec![older.clone(), newer.clone()], &ctx());
    assert_eq!(ranked[0].command, "echo a");
    let ranked = rank(vec![newer, older], &ctx());
    assert_eq!(ranked[0].command, "echo a");
}
