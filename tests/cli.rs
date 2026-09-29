use std::process::Command;
use tempfile::TempDir;

fn porteiro() -> Command {
    Command::new(env!("CARGO_BIN_EXE_porteiro"))
}

#[test]
fn help_describes_the_primary_options() {
    let output = porteiro().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--config"));
    assert!(stdout.contains("--ssh"));
}

#[test]
fn version_identifies_the_binary_without_loading_the_ssh_config() {
    let temp = TempDir::new().unwrap();
    let output = porteiro()
        .arg("--config")
        .arg(temp.path().join("missing"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!("porteiro ", env!("CARGO_PKG_VERSION"), "\n")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn missing_config_exits_with_an_actionable_error() {
    let temp = TempDir::new().unwrap();
    let output = porteiro()
        .args(["--config", temp.path().join("missing").to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SSH config not found"));
    assert!(stderr.contains("--config"));
}
