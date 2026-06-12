//! Secret redaction, applied to every command before it is stored.
//!
//! The secret *value* is replaced while the key/flag is kept, so history
//! entries stay recognizable. Patterns are ordered: specific shapes first.

use std::sync::LazyLock;

use regex::Regex;

pub const REDACTED: &str = "***REDACTED***";

static PATTERNS: LazyLock<Vec<(Regex, String)>> = LazyLock::new(|| {
    let keep_key = format!("${{1}}{REDACTED}");
    vec![
        // Authorization headers: `Authorization: Bearer eyJ...` (also inside -H '...')
        (
            Regex::new(
                r#"(?i)(authorization["']?\s*[:=]\s*["']?(?:bearer\s+|basic\s+|token\s+)?)[A-Za-z0-9+/_.=:-]+"#,
            )
            .unwrap(),
            keep_key.clone(),
        ),
        // key=value / key: value for sensitive key names
        (
            Regex::new(
                r#"(?i)\b((?:api[_-]?key|access[_-]?key(?:[_-]?id)?|secret(?:[_-]?(?:access[_-]?)?key)?|token|password|passwd|pwd|auth)["']?\s*[=:]\s*["']?)[^\s"']+"#,
            )
            .unwrap(),
            keep_key.clone(),
        ),
        // space-separated flags: `--password hunter2`, `--token abc`
        (
            Regex::new(r#"(?i)(--?(?:password|passwd|pwd|token|api-key|secret|auth)\s+)[^\s"']+"#)
                .unwrap(),
            keep_key,
        ),
        // AWS access key id
        (Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(), REDACTED.into()),
        // GitHub tokens (ghp_, gho_, ghu_, ghs_, ghr_)
        (
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").unwrap(),
            REDACTED.into(),
        ),
        // Slack tokens
        (
            Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap(),
            REDACTED.into(),
        ),
        // JWTs
        (
            Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b").unwrap(),
            REDACTED.into(),
        ),
    ]
});

pub fn redact(command: &str) -> String {
    let mut out = command.to_string();
    for (re, replacement) in PATTERNS.iter() {
        out = re.replace_all(&out, replacement.as_str()).into_owned();
    }
    out
}
