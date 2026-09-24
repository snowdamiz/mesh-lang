use std::path::PathBuf;

const CREDENTIALS_FILE: &str = "credentials";
const MESH_DIR: &str = ".mesh";

pub fn credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(MESH_DIR)
        .join(CREDENTIALS_FILE)
}

/// Read the auth token from ~/.mesh/credentials.
/// Returns descriptive error if not logged in.
pub fn read_token() -> Result<String, String> {
    let path = credentials_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|_| "Not logged in. Run `meshpkg login` first.".to_string())?;
    let table: toml::Table =
        toml::from_str(&content).map_err(|e| format!("Corrupted credentials file: {}", e))?;
    table
        .get("registry")
        .and_then(|r| r.get("token"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No token found in credentials file.".to_string())
}

/// Write an auth token to ~/.mesh/credentials.
/// Creates ~/.mesh/ directory if it does not exist.
pub fn write_token(token: &str) -> Result<(), String> {
    write_token_to(&credentials_path(), token)
}

/// Write the credentials file readable by its owner only: the token publishes
/// packages as that user.
fn write_token_to(path: &std::path::Path, token: &str) -> Result<(), String> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create ~/.mesh/: {}", e))?;
    }
    let mut registry = toml::Table::new();
    registry.insert("token".to_string(), token.into());
    let mut table = toml::Table::new();
    table.insert("registry".to_string(), registry.into());
    let content =
        toml::to_string(&table).map_err(|e| format!("Failed to encode credentials: {}", e))?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .map_err(|e| format!("Failed to write credentials: {}", e))?;
    // `mode` applies only to a new file; tighten one an older meshpkg wrote.
    #[cfg(unix)]
    std::fs::set_permissions(
        path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .map_err(|e| format!("Failed to restrict credentials: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write credentials: {}", e))
}

#[cfg(test)]
mod tests {
    #[test]
    fn credentials_are_owner_only_and_round_trip_any_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mesh/credentials");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old").unwrap();

        super::write_token_to(&path, "tok\"en\\x").unwrap();

        let table: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["registry"]["token"].as_str(), Some("tok\"en\\x"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
