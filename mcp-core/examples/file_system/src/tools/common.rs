use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn get_path(args: &HashMap<String, serde_json::Value>) -> Result<PathBuf> {
    tracing::debug!("Args: {args:?}");
    let path = args["path"]
        .as_str()
        .ok_or(anyhow::anyhow!("Missing path"))?;

    if path.starts_with('~') {
        let home = home::home_dir().ok_or(anyhow::anyhow!("Could not determine home directory"))?;
        // Strip the ~ and join with home path
        let path = home.join(path.strip_prefix("~/").unwrap_or_default());
        Ok(path)
    } else {
        Ok(PathBuf::from(path))
    }
}

pub fn search_directory(dir: &Path, pattern: &str, matches: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

        // Check if the current file/directory matches the pattern
        if name.contains(&pattern.to_lowercase()) {
            matches.push(path.to_string_lossy().to_string());
        }

        // Recursively search subdirectories
        if path.is_dir() {
            search_directory(&path, pattern, matches)?;
        }
    }
    Ok(())
}
