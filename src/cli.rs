use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "porteiro",
    version,
    about = "Browse and connect to hosts from an OpenSSH client config"
)]
pub struct Cli {
    /// Path to the OpenSSH client configuration (default: ~/.ssh/config)
    #[arg(short, long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// OpenSSH-compatible client executable
    #[arg(long, value_name = "PATH", default_value = "ssh")]
    pub ssh: PathBuf,
}

impl Cli {
    pub fn config_path(&self) -> Result<PathBuf> {
        config_path(self.config.as_deref(), dirs::home_dir().as_deref())
    }
}

fn config_path(config: Option<&Path>, home: Option<&Path>) -> Result<PathBuf> {
    let expanded = match config {
        Some(path) => expand_home(path, home),
        None => home
            .map(|home| home.join(".ssh/config"))
            .ok_or_else(|| anyhow!("cannot determine the home directory; pass --config PATH")),
    }?;
    lexically_absolute(expanded)
}

fn lexically_absolute(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    let current = std::env::current_dir().context("cannot determine the current directory")?;
    Ok(lexically_absolute_from(&path, &current))
}

fn lexically_absolute_from(path: &Path, current: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    }
}

fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return home
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("cannot expand '~'; pass an absolute --config path"));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home
            .map(|home| home.join(rest))
            .ok_or_else(|| anyhow!("cannot expand '~'; pass an absolute --config path"));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn default_config_uses_the_home_ssh_directory() {
        assert_eq!(
            config_path(None, Some(Path::new("/home/tester"))).unwrap(),
            PathBuf::from("/home/tester/.ssh/config")
        );
    }

    #[test]
    fn explicit_tilde_path_is_expanded() {
        assert_eq!(
            config_path(
                Some(Path::new("~/.ssh/work")),
                Some(Path::new("/home/tester"))
            )
            .unwrap(),
            PathBuf::from("/home/tester/.ssh/work")
        );
    }

    #[test]
    fn missing_home_is_actionable() {
        let error = config_path(None, None).unwrap_err().to_string();
        assert!(error.contains("--config"));
    }

    #[test]
    fn relative_none_sentinel_is_made_absolute_without_canonicalizing() {
        let temp = TempDir::new().unwrap();
        let mixed_case = temp.path().join("NoNe");
        fs::write(&mixed_case, "Host fixture\n").unwrap();

        assert_eq!(
            lexically_absolute_from(Path::new("none"), temp.path()),
            temp.path().join("none")
        );
        let selected = lexically_absolute_from(Path::new("NoNe"), temp.path());
        assert_eq!(selected, mixed_case);
        assert_eq!(fs::read_to_string(selected).unwrap(), "Host fixture\n");
    }
}
