use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone)]
pub struct Migration {
    pub timestamp: String,
    pub path: PathBuf,
    pub applied: bool,
}

pub async fn ensure_migrations_table(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS migrations (
            timestamp TEXT PRIMARY KEY,
            applied INTEGER NOT NULL DEFAULT 0,
            applied_at TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn scan_migrations(pool: &SqlitePool, migrations_path: &Path) -> Result<Vec<Migration>> {
    ensure_migrations_table(pool).await?;

    let mut migrations = Vec::new();

    if !migrations_path.exists() {
        return Ok(migrations);
    }

    let mut entries: Vec<_> = fs::read_dir(migrations_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter(|e| e.path().join("up.sql").exists())
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let applied_rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT timestamp, applied FROM migrations ORDER BY timestamp",
    )
    .fetch_all(pool)
    .await?;

    let applied_map: std::collections::HashMap<String, bool> = applied_rows
        .into_iter()
        .map(|(ts, applied)| (ts, applied != 0))
        .collect();

    for entry in entries {
        let folder_name = entry.file_name().to_string_lossy().to_string();
        let applied = applied_map.get(&folder_name).copied().unwrap_or(false);

        migrations.push(Migration {
            timestamp: folder_name,
            path: entry.path(),
            applied,
        });
    }

    Ok(migrations)
}

pub async fn run_up(pool: &SqlitePool, migrations_path: &Path, steps: Option<usize>) -> Result<usize> {
    let migrations = scan_migrations(pool, migrations_path).await?;

    let pending: Vec<_> = migrations.into_iter().filter(|m| !m.applied).collect();

    if pending.is_empty() {
        return Ok(0);
    }

    let to_apply: Vec<_> = match steps {
        Some(n) => pending.into_iter().take(n).collect(),
        None => pending,
    };

    let count = to_apply.len();

    for migration in to_apply {
        info!(migration = %migration.timestamp, "Applying migration");

        let up_sql = fs::read_to_string(migration.path.join("up.sql"))
            .with_context(|| format!("Failed to read up.sql for {}", migration.timestamp))?;

        execute_sql_statements(pool, &up_sql)
            .await
            .with_context(|| format!("Failed to apply migration {}", migration.timestamp))?;

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO migrations (timestamp, applied, applied_at)
            VALUES (?, 1, ?)
            ON CONFLICT (timestamp) DO UPDATE SET applied = 1, applied_at = ?
            "#,
        )
        .bind(&migration.timestamp)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        info!(migration = %migration.timestamp, "Migration applied");
    }

    Ok(count)
}

pub async fn run_down(pool: &SqlitePool, migrations_path: &Path, steps: Option<usize>) -> Result<usize> {
    let migrations = scan_migrations(pool, migrations_path).await?;

    let mut applied: Vec<_> = migrations.into_iter().filter(|m| m.applied).collect();
    applied.reverse();

    if applied.is_empty() {
        return Ok(0);
    }

    let steps = steps.unwrap_or(1);
    let to_revert: Vec<_> = applied.into_iter().take(steps).collect();

    let count = to_revert.len();

    for migration in to_revert {
        info!(migration = %migration.timestamp, "Reverting migration");

        let down_path = migration.path.join("down.sql");
        if !down_path.exists() {
            anyhow::bail!(
                "No down.sql found for migration {}. Cannot revert.",
                migration.timestamp
            );
        }

        let down_sql = fs::read_to_string(&down_path)
            .with_context(|| format!("Failed to read down.sql for {}", migration.timestamp))?;

        execute_sql_statements(pool, &down_sql)
            .await
            .with_context(|| format!("Failed to revert migration {}", migration.timestamp))?;

        sqlx::query("UPDATE migrations SET applied = 0, applied_at = NULL WHERE timestamp = ?")
            .bind(&migration.timestamp)
            .execute(pool)
            .await?;

        info!(migration = %migration.timestamp, "Migration reverted");
    }

    Ok(count)
}

pub fn create_migration(migrations_path: &Path, name: &str) -> Result<PathBuf> {
    fs::create_dir_all(migrations_path).context("Failed to create migrations directory")?;

    let timestamp = Utc::now().format("%Y%m%d%H%M%S").to_string();

    let folder_name = if name.is_empty() {
        timestamp.clone()
    } else {
        format!("{}_{}", timestamp, name)
    };

    let migration_path = migrations_path.join(&folder_name);
    fs::create_dir_all(&migration_path)?;

    let up_path = migration_path.join("up.sql");
    let down_path = migration_path.join("down.sql");

    fs::write(&up_path, "-- Write your UP migration SQL here\n")?;
    fs::write(&down_path, "-- Write your DOWN migration SQL here\n")?;

    Ok(migration_path)
}

pub async fn dump_schema(pool: &SqlitePool, schema_path: &Path) -> Result<()> {
    if let Some(parent) = schema_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut output = String::new();

    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    output.push_str("--\n");
    output.push_str("-- Chores Database Schema\n");
    output.push_str("-- Auto-generated by migrate tool - do not edit manually\n");
    output.push_str("--\n");
    output.push_str(&format!("-- Generated at: {}\n", now));

    let latest: Option<(String,)> = sqlx::query_as(
        "SELECT timestamp FROM migrations WHERE applied = 1 ORDER BY timestamp DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    if let Some((ts,)) = latest {
        output.push_str(&format!("-- Last migration: {}\n", ts));
    }

    output.push_str("--\n\n");

    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    for (name, sql) in tables {
        if name == "migrations" {
            continue;
        }
        output.push_str(&format!("-- Table: {}\n", name));
        output.push_str(&sql);
        output.push_str(";\n\n");
    }

    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, sql FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    if !indexes.is_empty() {
        output.push_str("-- Indexes\n");
        for (_, sql) in indexes {
            output.push_str(&sql);
            output.push_str(";\n");
        }
        output.push_str("\n");
    }

    let mut file = fs::File::create(schema_path)?;
    file.write_all(output.as_bytes())?;

    info!(path = %schema_path.display(), "Schema dumped");

    Ok(())
}

async fn execute_sql_statements(pool: &SqlitePool, sql: &str) -> Result<()> {
    let statements = split_sql_statements(sql);

    for stmt in statements {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        sqlx::query(trimmed).execute(pool).await?;
    }

    Ok(())
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                current.push(c);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                current.push(c);
            }
            '-' if !in_single_quote && !in_double_quote => {
                if chars.peek() == Some(&'-') {
                    chars.next();
                    while let Some(&nc) = chars.peek() {
                        if nc == '\n' {
                            break;
                        }
                        chars.next();
                    }
                } else {
                    current.push(c);
                }
            }
            ';' if !in_single_quote && !in_double_quote => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    statements.push(current.clone());
                }
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(current);
    }

    statements
}

pub fn default_migrations_path() -> PathBuf {
    PathBuf::from("migrations")
}

pub fn default_schema_path() -> PathBuf {
    PathBuf::from("db/schema.sql")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_sql_statements_simple() {
        let sql = "CREATE TABLE foo (id INT); CREATE TABLE bar (id INT);";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn test_split_sql_statements_with_quotes() {
        let sql = "INSERT INTO foo VALUES ('hello; world'); SELECT * FROM foo;";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("hello; world"));
    }

    #[test]
    fn test_split_sql_statements_with_comments() {
        let sql = "-- This is a comment\nCREATE TABLE foo (id INT); -- another comment\nSELECT 1;";
        let stmts = split_sql_statements(sql);
        assert_eq!(stmts.len(), 2);
    }
}
