use chrono::{Local, TimeZone};
use histq::search::{parse_query, ExitFilter};

fn now() -> chrono::DateTime<Local> {
    // A fixed Friday afternoon, local time.
    Local.with_ymd_and_hms(2026, 6, 12, 15, 30, 0).unwrap()
}

#[test]
fn plain_words_become_fts_with_prefix_on_last_term() {
    let q = parse_query("git ch", now());
    assert_eq!(q.fts.as_deref(), Some("\"git\" \"ch\"*"));
    assert_eq!(q.exit, ExitFilter::Any);
    assert!(q.since.is_none() && q.until.is_none());
}

#[test]
fn empty_query_has_no_fts() {
    assert_eq!(parse_query("", now()).fts, None);
    assert_eq!(parse_query("   ", now()).fts, None);
}

#[test]
fn punctuation_only_tokens_are_dropped() {
    let q = parse_query("docker && --", now());
    assert_eq!(q.fts.as_deref(), Some("\"docker\"*"));
}

#[test]
fn quotes_are_escaped_for_fts() {
    let q = parse_query("echo \"hi", now());
    assert_eq!(q.fts.as_deref(), Some("\"echo\" \"\"\"hi\"*"));
}

#[test]
fn failed_maps_to_nonzero_exit() {
    let q = parse_query("docker failed", now());
    assert_eq!(q.exit, ExitFilter::Failed);
    assert_eq!(q.fts.as_deref(), Some("\"docker\"*"));
}

#[test]
fn success_words_map_to_zero_exit() {
    for word in ["worked", "success", "succeeded", "passed"] {
        let q = parse_query(&format!("make {word}"), now());
        assert_eq!(q.exit, ExitFilter::Success, "word: {word}");
    }
}

#[test]
fn today_sets_since_to_local_midnight() {
    let q = parse_query("today", now());
    let midnight = Local.with_ymd_and_hms(2026, 6, 12, 0, 0, 0).unwrap();
    assert_eq!(q.since, Some(midnight.timestamp_millis()));
    assert_eq!(q.until, None);
    assert_eq!(q.fts, None);
}

#[test]
fn yesterday_sets_both_bounds() {
    let q = parse_query("yesterday", now());
    let midnight = Local.with_ymd_and_hms(2026, 6, 12, 0, 0, 0).unwrap();
    assert_eq!(q.until, Some(midnight.timestamp_millis()));
    assert_eq!(q.since, Some(midnight.timestamp_millis() - 86_400_000));
}

#[test]
fn last_week_is_parsed_as_a_bigram() {
    let q = parse_query("cargo last week", now());
    assert_eq!(q.since, Some(now().timestamp_millis() - 7 * 86_400_000));
    // Neither "last" nor "week" leaks into the text query.
    assert_eq!(q.fts.as_deref(), Some("\"cargo\"*"));
}

#[test]
fn last_without_week_is_a_normal_word() {
    let q = parse_query("tail last", now());
    assert_eq!(q.since, None);
    assert_eq!(q.fts.as_deref(), Some("\"tail\" \"last\"*"));
}

#[test]
fn hash_tokens_become_tags() {
    let q = parse_query("deploy #prod #infra", now());
    assert_eq!(q.tags, vec!["prod", "infra"]);
    assert_eq!(q.fts.as_deref(), Some("\"deploy\"*"));
}

#[test]
fn bare_hash_is_not_a_tag() {
    let q = parse_query("#", now());
    assert!(q.tags.is_empty());
}

#[test]
fn rule_words_combine() {
    let q = parse_query("docker push failed yesterday #release", now());
    assert_eq!(q.exit, ExitFilter::Failed);
    assert!(q.since.is_some() && q.until.is_some());
    assert_eq!(q.tags, vec!["release"]);
    assert_eq!(q.fts.as_deref(), Some("\"docker\" \"push\"*"));
}
