//! Migration CLI: scaffold generation, runner, and status.
//!
//! - `generate_migration`: Creates timestamped migration files with up/down stubs
//! - `run_migrations_up`: Apply pending migrations (implemented by plan 101-02)
//! - `run_migrations_down`: Rollback last migration (implemented by plan 101-02)
//! - `show_migration_status`: Show applied vs pending (implemented by plan 101-02)

use std::path::Path;

/// Apply all pending migrations.
///
/// Placeholder -- full implementation provided by plan 101-02.
pub fn run_migrations_up(_project_dir: &Path) -> Result<(), String> {
    Err("meshc migrate up: not yet implemented (see plan 101-02)".to_string())
}

/// Rollback the last applied migration.
///
/// Placeholder -- full implementation provided by plan 101-02.
pub fn run_migrations_down(_project_dir: &Path) -> Result<(), String> {
    Err("meshc migrate down: not yet implemented (see plan 101-02)".to_string())
}

/// Show migration status (applied vs pending).
///
/// Placeholder -- full implementation provided by plan 101-02.
pub fn show_migration_status(_project_dir: &Path) -> Result<(), String> {
    Err("meshc migrate status: not yet implemented (see plan 101-02)".to_string())
}

/// Generate a new migration scaffold file in the `migrations/` directory.
///
/// Creates `migrations/{timestamp}_{name}.mpl` with up/down function stubs.
/// The `migrations/` directory is created if it doesn't exist.
///
/// # Errors
///
/// Returns an error if:
/// - `name` is empty
/// - `name` contains characters other than lowercase ASCII letters, digits, or underscores
/// - Directory creation or file writing fails
pub fn generate_migration(project_dir: &Path, name: &str) -> Result<(), String> {
    // Validate name: must be non-empty, only lowercase letters, digits, underscores
    if name.is_empty() {
        return Err("Migration name cannot be empty".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(
            "Migration name must contain only lowercase letters, digits, and underscores"
                .to_string(),
        );
    }

    let migrations_dir = project_dir.join("migrations");
    std::fs::create_dir_all(&migrations_dir)
        .map_err(|e| format!("Failed to create migrations directory: {}", e))?;

    let timestamp = format_timestamp_now();
    let filename = format!("{}_{}.mpl", timestamp, name);
    let filepath = migrations_dir.join(&filename);

    let content = format!(
        r#"# Migration: {name}
# Generated: {timestamp}

pub fn up(pool :: PoolHandle) -> Int!String do
  # Add your migration code here
  # Examples:
  #   Migration.create_table(pool, "users", [
  #     "id:UUID:PRIMARY KEY DEFAULT gen_random_uuid()",
  #     "name:TEXT:NOT NULL",
  #     "email:TEXT:NOT NULL UNIQUE",
  #     "inserted_at:TIMESTAMPTZ:NOT NULL DEFAULT now()",
  #     "updated_at:TIMESTAMPTZ:NOT NULL DEFAULT now()"
  #   ])?
  #
  #   Migration.create_index(pool, "users", ["email"], "unique:true")?
  #
  #   Migration.add_column(pool, "users", "age:BIGINT")?
  #
  #   Migration.execute(pool, "CREATE EXTENSION IF NOT EXISTS pgcrypto")?
  Ok(0)
end

pub fn down(pool :: PoolHandle) -> Int!String do
  # Add your rollback code here
  # Examples:
  #   Migration.drop_table(pool, "users")?
  #   Migration.drop_column(pool, "users", "age")?
  #   Migration.drop_index(pool, "users", ["email"])?
  Ok(0)
end
"#,
        name = name,
        timestamp = timestamp
    );

    std::fs::write(&filepath, content)
        .map_err(|e| format!("Failed to write migration file: {}", e))?;

    eprintln!("Created migration: migrations/{}", filename);
    Ok(())
}

/// Generate a YYYYMMDDHHMMSS timestamp string from the current UTC time.
fn format_timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format_timestamp(secs)
}

/// Convert a Unix timestamp (seconds since epoch) to a YYYYMMDDHHMMSS string.
fn format_timestamp(secs: u64) -> String {
    // Convert Unix timestamp to UTC date/time components
    // Algorithm: days since epoch -> Gregorian calendar date
    let days = (secs / 86400) as i64;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Civil date from days since 1970-01-01 (Howard Hinnant algorithm)
    let (year, month, day) = civil_from_days(days);

    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since 1970-01-01 to (year, month, day).
/// Uses the Howard Hinnant algorithm from chrono-free date calculations.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u64, d as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp_epoch() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(format_timestamp(0), "19700101000000");
    }

    #[test]
    fn test_format_timestamp_known_date() {
        // 2026-02-16 12:00:00 UTC = 1771243200
        assert_eq!(format_timestamp(1771243200), "20260216120000");
    }

    #[test]
    fn test_format_timestamp_y2k() {
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(format_timestamp(946684800), "20000101000000");
    }

    #[test]
    fn test_format_timestamp_end_of_day() {
        // 2026-02-16 23:59:59 UTC = 1771243200 + 43199 = 1771286399
        assert_eq!(format_timestamp(1771286399), "20260216235959");
    }

    #[test]
    fn test_civil_from_days_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn test_civil_from_days_known() {
        // 2026-02-16 is day 20500 since epoch
        // Actually, let's compute: 1771243200 / 86400 = 20500.5 -> day 20500
        assert_eq!(civil_from_days(20500), (2026, 2, 16));
    }

    #[test]
    fn test_civil_from_days_leap_year() {
        // 2024-02-29 (leap day)
        // 2024-02-29 00:00:00 UTC = 1709164800 / 86400 = 19782
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
    }

    #[test]
    fn test_generate_migration_validates_empty_name() {
        let tmp = tempfile::tempdir().unwrap();
        let result = generate_migration(tmp.path(), "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot be empty"));
    }

    #[test]
    fn test_generate_migration_validates_uppercase() {
        let tmp = tempfile::tempdir().unwrap();
        let result = generate_migration(tmp.path(), "Create_Users");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("lowercase"));
    }

    #[test]
    fn test_generate_migration_validates_spaces() {
        let tmp = tempfile::tempdir().unwrap();
        let result = generate_migration(tmp.path(), "create users");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("lowercase"));
    }

    #[test]
    fn test_generate_migration_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        generate_migration(tmp.path(), "create_users").unwrap();

        let migrations_dir = tmp.path().join("migrations");
        assert!(migrations_dir.exists(), "migrations/ directory should exist");

        let entries: Vec<_> = std::fs::read_dir(&migrations_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "Should have exactly one migration file");

        let filename = entries[0].file_name().to_string_lossy().to_string();
        assert!(
            filename.ends_with("_create_users.mpl"),
            "Filename should end with _create_users.mpl, got: {}",
            filename
        );

        // Check timestamp prefix is 14 digits
        let prefix = &filename[..14];
        assert!(
            prefix.chars().all(|c| c.is_ascii_digit()),
            "First 14 chars should be digits, got: {}",
            prefix
        );
        assert_eq!(
            &filename[14..15],
            "_",
            "Character 15 should be underscore"
        );
    }

    #[test]
    fn test_generate_migration_file_content() {
        let tmp = tempfile::tempdir().unwrap();
        generate_migration(tmp.path(), "create_users").unwrap();

        let migrations_dir = tmp.path().join("migrations");
        let entry = std::fs::read_dir(&migrations_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let content = std::fs::read_to_string(entry.path()).unwrap();

        assert!(
            content.contains("pub fn up(pool :: PoolHandle) -> Int!String do"),
            "Should contain up function signature"
        );
        assert!(
            content.contains("pub fn down(pool :: PoolHandle) -> Int!String do"),
            "Should contain down function signature"
        );
        assert!(
            content.contains("# Migration: create_users"),
            "Should contain migration name comment"
        );
        assert!(
            content.contains("Migration.create_table"),
            "Should contain example DSL usage"
        );
    }

    #[test]
    fn test_generate_migration_creates_migrations_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let migrations_dir = tmp.path().join("migrations");
        assert!(!migrations_dir.exists(), "migrations/ should not exist yet");

        generate_migration(tmp.path(), "init").unwrap();
        assert!(
            migrations_dir.exists(),
            "migrations/ should be created by generate"
        );
    }

    #[test]
    fn test_generate_migration_allows_digits_and_underscores() {
        let tmp = tempfile::tempdir().unwrap();
        let result = generate_migration(tmp.path(), "add_column_v2");
        assert!(result.is_ok(), "Should allow digits and underscores");
    }
}
