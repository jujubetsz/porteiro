use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "Usage: porteiro [OPTIONS]";
const HELP: &str = "Browse and connect to hosts from an OpenSSH client config

Usage: porteiro [OPTIONS]

Options:
  -c, --config <PATH>  Path to the OpenSSH client configuration (default: ~/.ssh/config)
      --ssh <PATH>     OpenSSH-compatible client executable [default: ssh]
  -h, --help           Print help
  -V, --version        Print version
";

#[derive(Debug, PartialEq, Eq)]
pub struct Cli {
    pub config: Option<PathBuf>,
    pub ssh: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Run(Cli),
    Help,
    Version,
}

impl Cli {
    /// Parses the process arguments, printing help, version, or a usage
    /// error and exiting like a conventional CLI when the run cannot start.
    pub fn parse() -> Self {
        match parse_args(std::env::args_os().skip(1)) {
            Ok(Parsed::Run(cli)) => cli,
            Ok(Parsed::Help) => {
                print!("{HELP}");
                exit(0)
            }
            Ok(Parsed::Version) => {
                println!("porteiro {}", env!("CARGO_PKG_VERSION"));
                exit(0)
            }
            Err(message) => {
                eprintln!("error: {message}\n\n{USAGE}\n\nFor more information, try '--help'.");
                exit(2)
            }
        }
    }

    pub fn config_path(&self) -> Result<PathBuf> {
        config_path(self.config.as_deref(), std::env::home_dir().as_deref())
    }
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, String> {
    let mut config = None;
    let mut ssh = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        let Some(text) = argument
            .to_str()
            .filter(|text| text.starts_with('-') && text.len() > 1)
        else {
            return Err(format!("unexpected argument {argument:?} found"));
        };
        // Accept `--name value`, `--name=value`, `-n value`, `-nvalue`, and `-n=value`.
        let (name, attached) = if text.starts_with("--") {
            text.split_once('=')
                .map_or((text, None), |(name, value)| (name, Some(value)))
        } else {
            let split = text
                .char_indices()
                .nth(2)
                .map_or(text.len(), |(index, _)| index);
            let (name, value) = text.split_at(split);
            let value = value.strip_prefix('=').unwrap_or(value);
            (name, (!value.is_empty()).then_some(value))
        };
        let slot = match name {
            "--" => match args.next() {
                Some(extra) => return Err(format!("unexpected argument {extra:?} found")),
                None => break,
            },
            "-h" | "--help" | "-V" | "--version" => {
                if let Some(value) = attached {
                    return Err(format!("unexpected value {value:?} for '{name}' found"));
                }
                return Ok(if matches!(name, "-h" | "--help") {
                    Parsed::Help
                } else {
                    Parsed::Version
                });
            }
            "-c" | "--config" => (&mut config, "--config <PATH>"),
            "--ssh" => (&mut ssh, "--ssh <PATH>"),
            _ => return Err(format!("unexpected argument '{text}' found")),
        };
        if slot.0.is_some() {
            return Err(format!(
                "the argument '{}' cannot be used multiple times",
                slot.1
            ));
        }
        let value = match attached {
            Some(value) => OsString::from(value),
            None => args.next().ok_or_else(|| {
                format!("a value is required for '{}' but none was supplied", slot.1)
            })?,
        };
        *slot.0 = Some(PathBuf::from(value));
    }
    Ok(Parsed::Run(Cli {
        config,
        ssh: ssh.unwrap_or_else(|| PathBuf::from("ssh")),
    }))
}

fn config_path(config: Option<&Path>, home: Option<&Path>) -> Result<PathBuf> {
    let expanded = match config {
        Some(path) => expand_home(path, home),
        None => home
            .map(|home| home.join(".ssh/config"))
            .ok_or_else(|| anyhow!("cannot determine the home directory; pass --config PATH")),
    }?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let current = std::env::current_dir().context("cannot determine the current directory")?;
    Ok(current.join(expanded))
}

fn expand_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let raw = path.to_string_lossy();
    let suffix = match raw.strip_prefix('~') {
        Some("") => None,
        Some(rest) => match rest.strip_prefix('/') {
            Some(rest) => Some(rest),
            None => return Ok(path.to_path_buf()),
        },
        None => return Ok(path.to_path_buf()),
    };
    let home = home.ok_or_else(|| anyhow!("cannot expand '~'; pass an absolute --config path"))?;
    Ok(suffix.map_or_else(|| home.to_path_buf(), |rest| home.join(rest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn parse(arguments: &[&str]) -> Result<Parsed, String> {
        parse_args(arguments.iter().map(OsString::from))
    }

    fn run(arguments: &[&str]) -> Cli {
        match parse(arguments).unwrap() {
            Parsed::Run(cli) => cli,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn accepts_every_option_spelling() {
        for arguments in [
            &["--config", "cfg", "--ssh", "bin"][..],
            &["--config=cfg", "--ssh=bin"],
            &["-c", "cfg", "--ssh", "bin"],
            &["-ccfg", "--ssh=bin"],
            &["-c=cfg", "--ssh", "bin"],
        ] {
            let cli = run(arguments);
            assert_eq!(
                cli.config.as_deref(),
                Some(Path::new("cfg")),
                "{arguments:?}"
            );
            assert_eq!(cli.ssh, PathBuf::from("bin"), "{arguments:?}");
        }
        assert_eq!(run(&[]).ssh, PathBuf::from("ssh"));
        assert_eq!(run(&[]).config, None);
    }

    #[test]
    fn help_and_version_win_and_help_lists_the_options() {
        assert_eq!(parse(&["--config", "x", "-h"]), Ok(Parsed::Help));
        assert_eq!(parse(&["--help"]), Ok(Parsed::Help));
        assert_eq!(parse(&["-V"]), Ok(Parsed::Version));
        assert_eq!(parse(&["--version"]), Ok(Parsed::Version));
        assert!(HELP.contains("--config") && HELP.contains("--ssh"));
    }

    #[test]
    fn usage_errors_are_specific() {
        let error = |arguments| parse(arguments).unwrap_err();
        assert!(error(&["extra"]).contains("unexpected argument"));
        assert!(error(&["--bogus"]).contains("unexpected argument '--bogus'"));
        assert!(error(&["-"]).contains("unexpected argument"));
        assert!(error(&["--", "x"]).contains("unexpected argument"));
        assert_eq!(run(&["--"]).ssh, PathBuf::from("ssh"));
        assert!(error(&["-é"]).contains("unexpected argument"));
        assert!(error(&["--config"]).contains("a value is required for '--config <PATH>'"));
        assert!(error(&["-c", "a", "-c", "b"]).contains("cannot be used multiple times"));
        assert!(error(&["--help=1"]).contains("unexpected value"));
    }

    #[test]
    fn default_config_uses_the_home_ssh_directory() {
        assert_eq!(
            config_path(None, Some(Path::new("/home/tester"))).unwrap(),
            PathBuf::from("/home/tester/.ssh/config")
        );
    }

    #[test]
    fn explicit_tilde_paths_are_expanded() {
        let home = Some(Path::new("/home/tester"));
        assert_eq!(
            config_path(Some(Path::new("~/.ssh/work")), home).unwrap(),
            PathBuf::from("/home/tester/.ssh/work")
        );
        assert_eq!(
            config_path(Some(Path::new("~")), home).unwrap().as_os_str(),
            "/home/tester"
        );
        assert!(config_path(Some(Path::new("~other/x")), home)
            .unwrap()
            .ends_with("~other/x"));
        assert_eq!(
            config_path(Some(Path::new("/etc/~other")), home).unwrap(),
            PathBuf::from("/etc/~other")
        );
    }

    #[test]
    fn missing_home_is_actionable() {
        assert!(config_path(None, None)
            .unwrap_err()
            .to_string()
            .contains("--config"));
        assert!(config_path(Some(Path::new("~/x")), None)
            .unwrap_err()
            .to_string()
            .contains("--config"));
    }

    #[test]
    fn relative_none_sentinel_is_made_absolute_without_canonicalizing() {
        let temp = TempDir::new().unwrap();
        let mixed_case = temp.path().join("NoNe");
        fs::write(&mixed_case, "Host fixture\n").unwrap();
        let current = std::env::current_dir().unwrap();

        let selected = config_path(Some(Path::new("none")), None).unwrap();
        assert_eq!(selected, current.join("none"));
        assert!(config_path(Some(&mixed_case), None).unwrap() == mixed_case);
        assert_eq!(fs::read_to_string(&mixed_case).unwrap(), "Host fixture\n");
    }
}
