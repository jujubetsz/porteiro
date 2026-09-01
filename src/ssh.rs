use anyhow::{anyhow, Context, Result};
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const DEFAULT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<String>,
    pub proxy_jump: Option<String>,
    pub proxy_command: Option<String>,
    pub identity_files: Vec<String>,
}

impl ResolvedHost {
    pub fn destination(&self) -> String {
        let host = if self.hostname.contains(':') && !self.hostname.starts_with('[') {
            format!("[{}]", self.hostname)
        } else {
            self.hostname.clone()
        };
        let user_host = match &self.user {
            Some(user) if !user.is_empty() => format!("{user}@{host}"),
            _ => host,
        };
        match self.port.as_deref() {
            Some(port) if port != "22" => format!("{user_host}:{port}"),
            _ => user_host,
        }
    }
}

pub struct SshClient {
    binary: PathBuf,
    config: PathBuf,
    resolve_timeout: Duration,
}

impl SshClient {
    pub fn new(binary: PathBuf, config: PathBuf) -> Self {
        Self {
            binary,
            config,
            resolve_timeout: DEFAULT_RESOLVE_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(binary: PathBuf, config: PathBuf, timeout: Duration) -> Self {
        Self {
            binary,
            config,
            resolve_timeout: timeout,
        }
    }

    pub fn resolve(&self, alias: &str) -> Result<ResolvedHost> {
        let output = self.capture(&self.resolve_args(alias))?;
        if !output.status.success() {
            let detail = first_line(&output.stderr).unwrap_or("OpenSSH returned no error message");
            return Err(anyhow!(
                "ssh -G failed for {alias:?} ({}): {detail}",
                status_label(output.status)
            ));
        }
        parse_effective_config(&String::from_utf8_lossy(&output.stdout))
            .with_context(|| format!("cannot parse effective SSH config for {alias:?}"))
    }

    pub fn connect(&self, alias: &str) -> Result<ExitStatus> {
        Command::new(&self.binary)
            .args(self.connect_args(alias))
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("cannot start {}", self.binary.display()))
    }

    fn resolve_args(&self, alias: &str) -> Vec<OsString> {
        vec![
            OsString::from("-F"),
            self.config.as_os_str().to_owned(),
            OsString::from("-o"),
            OsString::from("CanonicalizeHostname=no"),
            OsString::from("-o"),
            OsString::from("CanonicalizePermittedCNAMEs=none"),
            OsString::from("-G"),
            OsString::from("--"),
            OsString::from(alias),
        ]
    }

    fn connect_args(&self, alias: &str) -> Vec<OsString> {
        vec![
            OsString::from("-F"),
            self.config.as_os_str().to_owned(),
            OsString::from("--"),
            OsString::from(alias),
        ]
    }

    fn capture(&self, arguments: &[OsString]) -> Result<CapturedOutput> {
        let stdout = tempfile::tempfile().context("cannot create temporary stdout capture")?;
        let stderr = tempfile::tempfile().context("cannot create temporary stderr capture")?;
        let mut command = Command::new(&self.binary);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                stdout
                    .try_clone()
                    .context("cannot clone temporary stdout capture")?,
            ))
            .stderr(Stdio::from(
                stderr
                    .try_clone()
                    .context("cannot clone temporary stderr capture")?,
            ));
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {}", self.binary.display()))?;

        let status = match child
            .wait_timeout(self.resolve_timeout)
            .context("cannot wait for ssh -G")?
        {
            Some(status) => status,
            None => {
                terminate(&mut child).context("cannot stop timed-out ssh -G")?;
                child.wait().context("cannot reap timed-out ssh -G")?;
                return Err(anyhow!(
                    "ssh -G timed out after {:.1}s",
                    self.resolve_timeout.as_secs_f32()
                ));
            }
        };

        Ok(CapturedOutput {
            status,
            stdout: read_capture(stdout, "stdout")?,
            stderr: read_capture(stderr, "stderr")?,
        })
    }
}

struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn read_capture(mut file: File, stream: &str) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("cannot rewind OpenSSH {stream}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("cannot read OpenSSH {stream}"))?;
    Ok(bytes)
}

#[cfg(unix)]
fn terminate(child: &mut std::process::Child) -> std::io::Result<()> {
    let process_group = i32::try_from(child.id())
        .map_err(|_| std::io::Error::other("child process id does not fit pid_t"))?;
    // The child starts its own process group above. A negative pid targets the
    // complete group so a Match exec helper cannot outlive the timeout.
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        Ok(())
    } else {
        let group_error = std::io::Error::last_os_error();
        if child.try_wait()?.is_some() {
            return Ok(());
        }

        child.kill().map_err(|child_error| {
            std::io::Error::new(
                child_error.kind(),
                format!(
                    "process-group kill failed: {group_error}; child kill failed: {child_error}"
                ),
            )
        })
    }
}

#[cfg(not(unix))]
fn terminate(child: &mut std::process::Child) -> std::io::Result<()> {
    child.kill()
}

fn parse_effective_config(contents: &str) -> Result<ResolvedHost> {
    let mut hostname = None;
    let mut user = None;
    let mut port = None;
    let mut proxy_jump = None;
    let mut proxy_command = None;
    let mut identity_files = Vec::new();

    for line in contents.lines() {
        let Some((keyword, value)) = split_keyword_value(line) else {
            continue;
        };
        let value = value.trim();
        match keyword.to_ascii_lowercase().as_str() {
            "hostname" => {
                hostname.get_or_insert_with(|| value.to_string());
            }
            "user" => {
                user.get_or_insert_with(|| value.to_string());
            }
            "port" => {
                port.get_or_insert_with(|| value.to_string());
            }
            "proxyjump" if !is_none(value) => {
                proxy_jump.get_or_insert_with(|| value.to_string());
            }
            "proxycommand" if !is_none(value) => {
                proxy_command.get_or_insert_with(|| value.to_string());
            }
            "identityfile" if !is_none(value) => {
                identity_files.push(value.to_string());
            }
            _ => continue,
        }
    }

    Ok(ResolvedHost {
        hostname: hostname.ok_or_else(|| anyhow!("ssh -G output has no HostName"))?,
        user,
        port,
        proxy_jump,
        proxy_command,
        identity_files,
    })
}

fn split_keyword_value(line: &str) -> Option<(&str, &str)> {
    let boundary = line.find(char::is_whitespace)?;
    let keyword = &line[..boundary];
    let value = line[boundary..].trim_start();
    (!keyword.is_empty() && !value.is_empty()).then_some((keyword, value))
}

fn is_none(value: &str) -> bool {
    value.eq_ignore_ascii_case("none")
}

fn first_line(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

fn status_label(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_effective_config_and_formats_destination() {
        let resolved = parse_effective_config(
            "hostname 2001:db8::1\nuser deploy\nport 2222\nproxyjump bastion\n\
             proxycommand none\nidentityfile ~/.ssh/id_one\nidentityfile ~/.ssh/id_two\n",
        )
        .unwrap();

        assert_eq!(resolved.hostname, "2001:db8::1");
        assert_eq!(resolved.proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(resolved.proxy_command, None);
        assert_eq!(resolved.identity_files.len(), 2);
        assert_eq!(resolved.destination(), "deploy@[2001:db8::1]:2222");
    }

    #[test]
    fn missing_hostname_is_rejected() {
        let error = parse_effective_config("user deploy\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("HostName"));
    }

    #[test]
    fn aliases_remain_one_argv_item() {
        let client = SshClient::new(PathBuf::from("ssh"), PathBuf::from("config"));
        let arguments = client.connect_args("prod; touch /tmp/not-run");
        assert_eq!(arguments.len(), 4);
        assert_eq!(arguments[3], OsString::from("prod; touch /tmp/not-run"));
    }

    #[test]
    fn preview_disables_hostname_and_cname_canonicalization() {
        let client = SshClient::new(PathBuf::from("ssh"), PathBuf::from("config"));

        assert_eq!(
            client.resolve_args("production"),
            [
                "-F",
                "config",
                "-o",
                "CanonicalizeHostname=no",
                "-o",
                "CanonicalizePermittedCNAMEs=none",
                "-G",
                "--",
                "production",
            ]
            .map(OsString::from)
        );
    }

    #[cfg(unix)]
    fn executable_script(contents: &str) -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let script = temp.path().join("ssh-stub");
        fs::write(&script, contents).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (temp, script)
    }

    #[cfg(unix)]
    #[test]
    fn resolve_runs_compatible_binary_and_parses_success() {
        let (_temp, script) = executable_script(
            "#!/bin/sh\nprintf 'hostname server.example\\nuser deploy\\nport 22\\n'\n",
        );
        let client = SshClient::new(script, PathBuf::from("config"));

        let resolved = client.resolve("prod").unwrap();
        assert_eq!(resolved.destination(), "deploy@server.example");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_surfaces_process_failure() {
        let (_temp, script) = executable_script("#!/bin/sh\necho 'bad config' >&2\nexit 7\n");
        let client = SshClient::new(script, PathBuf::from("config"));

        let error = client.resolve("prod").unwrap_err().to_string();
        assert!(error.contains("exit 7"));
        assert!(error.contains("bad config"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_times_out_a_stuck_process() {
        let (_temp, script) = executable_script("#!/bin/sh\nwhile :; do :; done\n");
        let client =
            SshClient::with_timeout(script, PathBuf::from("config"), Duration::from_millis(20));

        let error = client.resolve("prod").unwrap_err().to_string();
        assert!(error.contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn connect_returns_the_child_status_and_preserves_arguments() {
        let temp = TempDir::new().unwrap();
        let argv_file = temp.path().join("argv");
        let script = temp.path().join("ssh-stub");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\nexit 7\n",
                argv_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let client = SshClient::new(script, PathBuf::from("config path"));

        let status = client.connect("my host").unwrap();
        assert_eq!(status.code(), Some(7));
        assert_eq!(
            fs::read_to_string(argv_file).unwrap(),
            "-F\nconfig path\n--\nmy host\n"
        );
    }
}
