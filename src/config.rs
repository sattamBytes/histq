//! Optional user configuration, loaded from `~/.config/histq/config.toml`
//! (or `$HISTQ_CONFIG`). Every field has a default, so no file is required.
//!
//! ```toml
//! candidate_limit = 500
//!
//! [weights]
//! text = 4.0
//! repo = 2.0
//! cwd = 1.5
//! recency = 1.0
//! success = 0.5
//! tags = 1.0
//! recency_half_life_days = 7.0
//!
//! [redact]
//! extra_patterns = ['mycorp_[A-Za-z0-9]{32}']
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;

use crate::search::Weights;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// How many rows to fetch from SQLite before ranking in Rust.
    pub candidate_limit: usize,
    pub weights: Weights,
    pub redact: RedactConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            candidate_limit: 500,
            weights: Weights::default(),
            redact: RedactConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RedactConfig {
    /// Additional regexes; any match is replaced with ***REDACTED*** before storage.
    pub extra_patterns: Vec<String>,
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("HISTQ_CONFIG") {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .unwrap_or_default();
        base.join("histq").join("config.toml")
    }

    /// Missing file means defaults; a malformed file is a hard error so it
    /// can't silently change behavior.
    pub fn load() -> Result<Config> {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("invalid config at {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading config at {}", path.display())),
        }
    }

    /// Compile the user-supplied redaction patterns.
    pub fn extra_redact_patterns(&self) -> Result<Vec<Regex>> {
        self.redact
            .extra_patterns
            .iter()
            .map(|p| {
                Regex::new(p).with_context(|| format!("invalid redact pattern in config: {p:?}"))
            })
            .collect()
    }
}
