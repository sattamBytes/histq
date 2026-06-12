use histq::config::Config;
use histq::search::Weights;

#[test]
fn empty_config_gives_defaults() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(cfg.candidate_limit, 500);
    assert_eq!(cfg.weights, Weights::default());
    assert!(cfg.redact.extra_patterns.is_empty());
}

#[test]
fn partial_weights_override_keeps_other_defaults() {
    let cfg: Config = toml::from_str(
        r#"
        candidate_limit = 100

        [weights]
        recency = 5.0
        "#,
    )
    .unwrap();
    assert_eq!(cfg.candidate_limit, 100);
    assert_eq!(cfg.weights.recency, 5.0);
    // Untouched fields keep their defaults.
    assert_eq!(cfg.weights.text, Weights::default().text);
    assert_eq!(cfg.weights.repo, Weights::default().repo);
}

#[test]
fn unknown_keys_are_rejected() {
    // Typos should fail loudly, not silently do nothing.
    assert!(toml::from_str::<Config>("candidat_limit = 9").is_err());
    assert!(toml::from_str::<Config>("[weights]\ntxt = 9.0").is_err());
}

#[test]
fn extra_redact_patterns_compile_and_apply() {
    let cfg: Config = toml::from_str(
        r#"
        [redact]
        extra_patterns = ['mycorp_[a-z0-9]{8}']
        "#,
    )
    .unwrap();
    let extra = cfg.extra_redact_patterns().unwrap();
    let out = histq::redact::redact_with("curl -d key=mycorp_abc12345 https://x", &extra);
    assert!(!out.contains("mycorp_abc12345"), "got: {out}");
    assert!(out.contains("***REDACTED***"));
}

#[test]
fn invalid_extra_pattern_is_an_error() {
    let cfg: Config = toml::from_str(
        r#"
        [redact]
        extra_patterns = ['[unclosed']
        "#,
    )
    .unwrap();
    assert!(cfg.extra_redact_patterns().is_err());
}

#[test]
fn custom_weights_change_ranking() {
    use histq::db::Candidate;
    use histq::search::{rank, RankContext};

    let mk = |cmd: &str, cwd: &str, started_at: i64| Candidate {
        id: 0,
        command: cmd.to_string(),
        cwd: cwd.to_string(),
        git_repo: None,
        git_branch: None,
        started_at,
        duration_ms: None,
        exit_code: Some(0),
        tags: String::new(),
        fts_rank: None,
    };
    const DAY: i64 = 86_400_000;
    let ctx = RankContext {
        cwd: "/here".to_string(),
        git_repo: None,
        now_ms: 100 * DAY,
        query_tags: vec![],
    };

    // Same-directory but old vs other-directory but fresh.
    let local_old = mk("local old", "/here", 40 * DAY);
    let fresh_elsewhere = mk("fresh elsewhere", "/there", 100 * DAY - 1);

    // Defaults: cwd bonus (1.5) outweighs the recency gap.
    let ranked = rank(
        vec![local_old.clone(), fresh_elsewhere.clone()],
        &ctx,
        &Weights::default(),
    );
    assert_eq!(ranked[0].command, "local old");

    // A recency-obsessed config flips the order.
    let recency_lover = Weights {
        recency: 10.0,
        cwd: 0.1,
        ..Weights::default()
    };
    let ranked = rank(vec![local_old, fresh_elsewhere], &ctx, &recency_lover);
    assert_eq!(ranked[0].command, "fresh elsewhere");
}
