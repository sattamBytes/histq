//! SQLite storage: schema setup and all queries.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, ToSql};

use crate::search::{ExitFilter, ParsedQuery};

/// A command about to be recorded (exit code and duration come later).
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub command: String,
    pub cwd: String,
    pub git_repo: Option<String>,
    pub git_branch: Option<String>,
    pub started_at: i64,
    pub tags: String,
    pub session_id: String,
}

/// A stored command, as returned by candidate queries.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: i64,
    pub command: String,
    pub cwd: String,
    pub git_repo: Option<String>,
    pub git_branch: Option<String>,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub tags: String,
    /// FTS5 bm25 rank (more negative = better match); None when the query had no text terms.
    pub fts_rank: Option<f64>,
}

pub struct Db {
    conn: Connection,
}

/// Aggregates returned by [`Db::stats`]. Grouped rows are `(label, count, fail_count)`.
pub struct Stats {
    pub total: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub top_commands: Vec<(String, i64, i64)>,
    pub failing: Vec<(String, i64, i64)>,
    pub top_dirs: Vec<(String, i64, i64)>,
    pub by_hour: Vec<i64>,
}

/// Resolve the database path: `$HISTQ_DB` if set, otherwise the XDG data dir.
pub fn default_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("HISTQ_DB") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let dirs = directories::ProjectDirs::from("", "", "histq")
        .context("could not determine a data directory for the history database")?;
    Ok(dirs.data_dir().join("history.db"))
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_millis(100))?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    fn migrate(&self) -> Result<()> {
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            self.conn.execute_batch(
                r#"
                BEGIN;
                CREATE TABLE commands (
                    id          INTEGER PRIMARY KEY,
                    command     TEXT NOT NULL,
                    cwd         TEXT NOT NULL,
                    git_repo    TEXT,
                    git_branch  TEXT,
                    started_at  INTEGER NOT NULL,
                    duration_ms INTEGER,
                    exit_code   INTEGER,
                    tags        TEXT NOT NULL DEFAULT '',
                    session_id  TEXT NOT NULL
                );
                CREATE INDEX idx_commands_session ON commands(session_id, started_at);
                CREATE INDEX idx_commands_started ON commands(started_at);

                CREATE VIRTUAL TABLE commands_fts USING fts5(
                    command, tags,
                    content='commands', content_rowid='id'
                );
                CREATE TRIGGER commands_ai AFTER INSERT ON commands BEGIN
                    INSERT INTO commands_fts(rowid, command, tags)
                    VALUES (new.id, new.command, new.tags);
                END;
                CREATE TRIGGER commands_ad AFTER DELETE ON commands BEGIN
                    INSERT INTO commands_fts(commands_fts, rowid, command, tags)
                    VALUES ('delete', old.id, old.command, old.tags);
                END;
                CREATE TRIGGER commands_au AFTER UPDATE ON commands BEGIN
                    INSERT INTO commands_fts(commands_fts, rowid, command, tags)
                    VALUES ('delete', old.id, old.command, old.tags);
                    INSERT INTO commands_fts(rowid, command, tags)
                    VALUES (new.id, new.command, new.tags);
                END;

                PRAGMA user_version = 1;
                COMMIT;
                "#,
            )?;
        }
        Ok(())
    }

    pub fn record_start(&self, entry: &NewEntry) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO commands (command, cwd, git_repo, git_branch, started_at, tags, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.command,
                entry.cwd,
                entry.git_repo,
                entry.git_branch,
                entry.started_at,
                entry.tags,
                entry.session_id,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Finalize the newest pending command for a session. Returns false when
    /// there is nothing pending (first prompt, Ctrl-C on an empty line, ...).
    pub fn record_end(&self, session: &str, exit_code: i32, now_ms: i64) -> Result<bool> {
        let updated = self.conn.execute(
            "UPDATE commands
             SET exit_code = ?1,
                 duration_ms = MAX(0, ?2 - started_at)
             WHERE id = (
                 SELECT id FROM commands
                 WHERE session_id = ?3 AND exit_code IS NULL
                 ORDER BY started_at DESC, id DESC
                 LIMIT 1
             )",
            params![exit_code, now_ms, session],
        )?;
        Ok(updated > 0)
    }

    /// Session id used for rows backfilled by `histq import`.
    pub const IMPORT_SESSION: &'static str = "imported";

    /// Bulk-insert imported entries `(command, started_at_ms, duration_ms)`.
    /// Idempotent: timestamped rows dedupe on command + timestamp; rows
    /// without a real timestamp (plain history format, `started_at` None)
    /// dedupe on command text alone — their fallback timestamp differs
    /// between runs, so it can't participate in the identity check.
    /// Imported rows get exit_code 0 (unknown, but assuming success keeps
    /// them reachable by arrow navigation) and an empty cwd so they never
    /// collect a same-directory ranking bonus.
    pub fn import_entries(
        &self,
        rows: &[(String, Option<i64>, Option<i64>)],
        fallback_ts: i64,
    ) -> Result<(usize, usize)> {
        self.conn.execute_batch("BEGIN")?;
        let result = (|| {
            let mut exists_with_ts = self.conn.prepare(
                "SELECT EXISTS(SELECT 1 FROM commands
                 WHERE session_id = ?1 AND command = ?2 AND started_at = ?3)",
            )?;
            let mut exists_command = self.conn.prepare(
                "SELECT EXISTS(SELECT 1 FROM commands
                 WHERE session_id = ?1 AND command = ?2)",
            )?;
            let mut insert = self.conn.prepare(
                "INSERT INTO commands
                 (command, cwd, git_repo, git_branch, started_at, duration_ms, exit_code, tags, session_id)
                 VALUES (?1, '', NULL, NULL, ?2, ?3, 0, '', ?4)",
            )?;
            let mut imported = 0;
            let mut skipped = 0;
            for (command, started_at, duration_ms) in rows {
                let already: bool = match started_at {
                    Some(ts) => exists_with_ts
                        .query_row(params![Self::IMPORT_SESSION, command, ts], |r| r.get(0))?,
                    None => exists_command
                        .query_row(params![Self::IMPORT_SESSION, command], |r| r.get(0))?,
                };
                if already {
                    skipped += 1;
                } else {
                    insert.execute(params![
                        command,
                        started_at.unwrap_or(fallback_ts),
                        duration_ms,
                        Self::IMPORT_SESSION
                    ])?;
                    imported += 1;
                }
            }
            Ok::<_, anyhow::Error>((imported, skipped))
        })();
        match result {
            Ok(counts) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(counts)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Delete rows by id. Returns how many were actually deleted.
    pub fn delete_ids(&self, ids: &[i64]) -> Result<usize> {
        let mut deleted = 0;
        for id in ids {
            deleted += self
                .conn
                .execute("DELETE FROM commands WHERE id = ?1", params![id])?;
        }
        Ok(deleted)
    }

    /// Find rows whose command contains a literal substring (no FTS
    /// tokenization — exact match works even inside tokens, which matters
    /// when hunting down an accidentally stored secret).
    pub fn find_containing(&self, needle: &str, limit: usize) -> Result<Vec<Candidate>> {
        let escaped = needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.command, c.cwd, c.git_repo, c.git_branch,
                    c.started_at, c.duration_ms, c.exit_code, c.tags, NULL
             FROM commands c
             WHERE c.command LIKE '%' || ?1 || '%' ESCAPE '\\'
             ORDER BY c.started_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![escaped, limit as i64], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2)?,
                git_repo: row.get(3)?,
                git_branch: row.get(4)?,
                started_at: row.get(5)?,
                duration_ms: row.get(6)?,
                exit_code: row.get(7)?,
                tags: row.get(8)?,
                fts_rank: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Aggregates for `histq stats`.
    pub fn stats(&self, top_limit: usize) -> Result<Stats> {
        let (total, succeeded, failed): (i64, i64, i64) = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(exit_code = 0), 0),
                    COALESCE(SUM(exit_code IS NOT NULL AND exit_code <> 0), 0)
             FROM commands",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        let top_commands = self.grouped(
            "SELECT command, COUNT(*) AS n,
                    COALESCE(SUM(exit_code IS NOT NULL AND exit_code <> 0), 0)
             FROM commands GROUP BY command ORDER BY n DESC LIMIT ?1",
            top_limit,
        )?;

        let failing = self.grouped(
            "SELECT command, COUNT(*) AS n,
                    COALESCE(SUM(exit_code IS NOT NULL AND exit_code <> 0), 0) AS fails
             FROM commands GROUP BY command
             HAVING n >= 3 AND fails > 0
             ORDER BY CAST(fails AS REAL) / n DESC, n DESC LIMIT ?1",
            top_limit,
        )?;

        let top_dirs = self.grouped(
            "SELECT cwd, COUNT(*) AS n, 0 FROM commands
             WHERE cwd <> '' GROUP BY cwd ORDER BY n DESC LIMIT ?1",
            top_limit,
        )?;

        let mut by_hour = vec![0i64; 24];
        let mut stmt = self.conn.prepare(
            "SELECT CAST(strftime('%H', started_at / 1000, 'unixepoch', 'localtime') AS INTEGER),
                    COUNT(*)
             FROM commands WHERE session_id <> 'imported' GROUP BY 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (hour, n) = row?;
            if (0..24).contains(&hour) {
                by_hour[hour as usize] = n;
            }
        }

        Ok(Stats {
            total,
            succeeded,
            failed,
            top_commands,
            failing,
            top_dirs,
            by_hour,
        })
    }

    fn grouped(&self, sql: &str, limit: usize) -> Result<Vec<(String, i64, i64)>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Fetch candidate rows for a parsed query. Ranking happens in Rust
    /// (see `search::rank`); this only filters and bounds the set.
    pub fn candidates(
        &self,
        query: &ParsedQuery,
        limit: usize,
        completed_only: bool,
    ) -> Result<Vec<Candidate>> {
        let mut binds: Vec<Box<dyn ToSql>> = Vec::new();
        let columns = "c.id, c.command, c.cwd, c.git_repo, c.git_branch, \
                       c.started_at, c.duration_ms, c.exit_code, c.tags";

        let mut sql = match &query.fts {
            Some(fts) => {
                binds.push(Box::new(fts.clone()));
                format!(
                    "SELECT {columns}, f.rank FROM commands_fts f \
                     JOIN commands c ON c.id = f.rowid \
                     WHERE commands_fts MATCH ?"
                )
            }
            None => format!("SELECT {columns}, NULL FROM commands c WHERE 1=1"),
        };

        if completed_only {
            sql.push_str(" AND c.exit_code IS NOT NULL");
        }
        match query.exit {
            ExitFilter::Any => {}
            ExitFilter::Success => sql.push_str(" AND c.exit_code = 0"),
            ExitFilter::Failed => sql.push_str(" AND c.exit_code IS NOT NULL AND c.exit_code <> 0"),
        }
        if let Some(since) = query.since {
            sql.push_str(" AND c.started_at >= ?");
            binds.push(Box::new(since));
        }
        if let Some(until) = query.until {
            sql.push_str(" AND c.started_at < ?");
            binds.push(Box::new(until));
        }
        for tag in &query.tags {
            sql.push_str(" AND (' ' || c.tags || ' ') LIKE ?");
            binds.push(Box::new(format!("% {tag} %")));
        }

        if query.fts.is_some() {
            sql.push_str(" ORDER BY f.rank LIMIT ?");
        } else {
            sql.push_str(" ORDER BY c.started_at DESC LIMIT ?");
        }
        binds.push(Box::new(limit as i64));

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(binds.iter().map(|b| b.as_ref())), |row| {
            Ok(Candidate {
                id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2)?,
                git_repo: row.get(3)?,
                git_branch: row.get(4)?,
                started_at: row.get(5)?,
                duration_ms: row.get(6)?,
                exit_code: row.get(7)?,
                tags: row.get(8)?,
                fts_rank: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}
