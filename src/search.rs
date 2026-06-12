//! Query parsing (rule words, tags, FTS) and ranking. No AI — just rules and weights.

use std::collections::HashSet;

use chrono::{DateTime, Local, TimeZone};

use crate::db::Candidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitFilter {
    #[default]
    Any,
    Success,
    Failed,
}

/// A user query after rule words have been peeled off.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedQuery {
    /// FTS5 match expression, e.g. `"git" "ch"*` — None when the query had no text terms.
    pub fts: Option<String>,
    pub exit: ExitFilter,
    /// Unix epoch ms bounds derived from time words (`today`, `yesterday`, `last week`).
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Tags from `#tag` tokens.
    pub tags: Vec<String>,
}

/// Parse a query string. `now` is injected so tests are deterministic.
pub fn parse_query(input: &str, now: DateTime<Local>) -> ParsedQuery {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut parsed = ParsedQuery::default();
    let mut terms: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        let lower = token.to_lowercase();
        match lower.as_str() {
            "failed" | "failing" | "error" | "errored" | "broke" | "broken" => {
                parsed.exit = ExitFilter::Failed;
            }
            "worked" | "success" | "successful" | "succeeded" | "passed" => {
                parsed.exit = ExitFilter::Success;
            }
            "today" => {
                parsed.since = Some(local_midnight_ms(now));
            }
            "yesterday" => {
                let midnight = local_midnight_ms(now);
                parsed.since = Some(midnight - DAY_MS);
                parsed.until = Some(midnight);
            }
            "last" if tokens.get(i + 1).map(|t| t.to_lowercase()).as_deref() == Some("week") => {
                parsed.since = Some(now.timestamp_millis() - 7 * DAY_MS);
                i += 1;
            }
            _ if token.len() > 1 && token.starts_with('#') => {
                parsed.tags.push(token[1..].to_string());
            }
            _ => terms.push(token),
        }
        i += 1;
    }

    parsed.fts = build_fts(&terms);
    parsed
}

const DAY_MS: i64 = 86_400_000;

fn local_midnight_ms(now: DateTime<Local>) -> i64 {
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .unwrap_or(now)
        .timestamp_millis()
}

/// Build an FTS5 match expression: every term as a quoted phrase, the last
/// term as a prefix match so a partially typed command still hits.
fn build_fts(terms: &[&str]) -> Option<String> {
    // Punctuation-only tokens ("&&", "|") tokenize to nothing in FTS5 and
    // would make the query trivially empty — drop them.
    let usable: Vec<&str> = terms
        .iter()
        .copied()
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
        .collect();
    if usable.is_empty() {
        return None;
    }
    let last = usable.len() - 1;
    let parts: Vec<String> = usable
        .iter()
        .enumerate()
        .map(|(i, term)| {
            let escaped = term.replace('"', "\"\"");
            if i == last {
                format!("\"{escaped}\"*")
            } else {
                format!("\"{escaped}\"")
            }
        })
        .collect();
    Some(parts.join(" "))
}

/// Context the ranking compares each candidate against.
#[derive(Debug, Clone, Default)]
pub struct RankContext {
    pub cwd: String,
    pub git_repo: Option<String>,
    pub now_ms: i64,
    pub query_tags: Vec<String>,
}

// Text match must outweigh the combined context bonuses (W_REPO + W_CWD),
// so an explicit query is never drowned out by "same place" candidates.
const W_TEXT: f64 = 4.0;
const W_REPO: f64 = 2.0;
const W_CWD: f64 = 1.5;
const W_RECENCY: f64 = 1.0;
const W_SUCCESS: f64 = 0.5;
const W_TAGS: f64 = 1.0;
const RECENCY_HALF_LIFE_DAYS: f64 = 7.0;

/// Score a single candidate. `max_relevance` is the best (-bm25) in the
/// candidate set, used to normalize the text component to 0..1.
pub fn score(candidate: &Candidate, ctx: &RankContext, max_relevance: f64) -> f64 {
    let mut s = 0.0;

    if let Some(rank) = candidate.fts_rank {
        if max_relevance > 0.0 {
            s += W_TEXT * ((-rank) / max_relevance).clamp(0.0, 1.0);
        }
    }
    if candidate.git_repo.is_some() && candidate.git_repo == ctx.git_repo {
        s += W_REPO;
    }
    if candidate.cwd == ctx.cwd {
        s += W_CWD;
    }
    let age_days = (ctx.now_ms - candidate.started_at).max(0) as f64 / DAY_MS as f64;
    s += W_RECENCY * 0.5_f64.powf(age_days / RECENCY_HALF_LIFE_DAYS);
    if candidate.exit_code == Some(0) {
        s += W_SUCCESS;
    }
    if !ctx.query_tags.is_empty() {
        let candidate_tags: HashSet<&str> = candidate.tags.split_whitespace().collect();
        let overlap = ctx
            .query_tags
            .iter()
            .filter(|t| candidate_tags.contains(t.as_str()))
            .count();
        s += W_TAGS * overlap as f64 / ctx.query_tags.len() as f64;
    }
    s
}

/// Rank candidates: score, sort (ties broken by recency), and dedupe by
/// command text keeping the best-scored instance.
pub fn rank(candidates: Vec<Candidate>, ctx: &RankContext) -> Vec<Candidate> {
    let max_relevance = candidates
        .iter()
        .filter_map(|c| c.fts_rank.map(|r| -r))
        .fold(0.0_f64, f64::max);

    let mut scored: Vec<(f64, Candidate)> = candidates
        .into_iter()
        .map(|c| (score(&c, ctx, max_relevance), c))
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.started_at.cmp(&a.1.started_at))
    });

    let mut seen = HashSet::new();
    scored
        .into_iter()
        .filter(|(_, c)| seen.insert(c.command.clone()))
        .map(|(_, c)| c)
        .collect()
}
