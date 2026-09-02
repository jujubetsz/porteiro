mod catalog;
mod cli;
mod ssh;
mod ui;

use anyhow::{Context, Result};
use catalog::Catalog;
use cli::Cli;
use ssh::SshClient;
use std::process::{ExitCode, ExitStatus};
use ui::{Connection, Outcome};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("porteiro: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let requested_config = cli.config_path()?;
    let catalog = Catalog::load(&requested_config)?;
    let ssh = SshClient::new(cli.ssh, catalog.config_path().to_path_buf());

    let terminal = ratatui::init();
    let ui_result = ui::run(terminal, catalog, &ssh);
    finish_ui(ui_result, ratatui::try_restore, |connection| {
        ssh.connect(&connection.alias, connection.verbosity)
    })
}

fn finish_ui<R, C>(ui_result: Result<Outcome>, restore: R, connect: C) -> Result<ExitCode>
where
    R: FnOnce() -> std::io::Result<()>,
    C: FnOnce(&Connection) -> Result<ExitStatus>,
{
    restore().context("cannot restore terminal before leaving the TUI")?;

    match ui_result? {
        Outcome::Quit => Ok(ExitCode::SUCCESS),
        Outcome::Connect(connection) => {
            let status = connect(&connection)?;
            Ok(exit_code(status))
        }
    }
}

fn exit_code(status: ExitStatus) -> ExitCode {
    ExitCode::from(exit_code_value(status))
}

fn exit_code_value(status: ExitStatus) -> u8 {
    if let Some(code) = status.code().and_then(|code| u8::try_from(code).ok()) {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return u8::try_from(128_i32.saturating_add(signal)).unwrap_or(1);
        }
    }

    1
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::exit_code_value;
    use super::finish_ui;
    use crate::ui::{Connection, Outcome};
    use std::cell::Cell;
    use std::io;
    #[cfg(unix)]
    use std::process::Command;

    #[test]
    fn restore_failure_prevents_connection() {
        let connected = Cell::new(false);
        let result = finish_ui(
            Ok(Outcome::Connect(Connection {
                alias: "production".to_string(),
                verbosity: 0,
            })),
            || Err(io::Error::other("restore failed")),
            |_| {
                connected.set(true);
                unreachable!("connection must not start after a restore failure")
            },
        );

        assert!(!connected.get());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot restore terminal"));
    }

    #[cfg(unix)]
    #[test]
    fn maps_signal_termination_to_shell_observable_status() {
        let status = Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();

        assert_eq!(exit_code_value(status), 128 + libc::SIGTERM as u8);
    }
}
