#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn tty_child_stdout_is_streamed_once_while_graph_is_live() {
  if Command::new("script").arg("--version").output().is_err() {
    eprintln!("skipping: util-linux script is unavailable");
    return;
  }

  let dir = tempfile::tempdir().unwrap();
  let nix = dir.path().join("nix");
  fs::write(
    &nix,
    "#!/bin/sh\nprintf 'ROM_STDOUT_FIRST\\n'\nsleep 0.08\nprintf \
     'ROM_STDOUT_SECOND\\n'\nsleep 0.08\n",
  )
  .unwrap();
  let mut permissions = fs::metadata(&nix).unwrap().permissions();
  permissions.set_mode(0o755);
  fs::set_permissions(&nix, permissions).unwrap();

  let rom = env!("CARGO_BIN_EXE_rom");
  let command = format!("'{rom}' --platform nix build fake");
  let path = format!(
    "{}:{}",
    dir.path().display(),
    std::env::var("PATH").unwrap_or_default()
  );
  let output = Command::new("script")
    .args(["-qfec", &command, "/dev/null"])
    .env("PATH", path)
    .output()
    .unwrap();

  assert!(output.status.success(), "PTY run failed: {output:?}");
  let transcript = String::from_utf8_lossy(&output.stdout);
  assert_eq!(
    transcript.matches("ROM_STDOUT_FIRST").count(),
    1,
    "{transcript:?}"
  );
  assert_eq!(
    transcript.matches("ROM_STDOUT_SECOND").count(),
    1,
    "{transcript:?}"
  );
  assert!(
    transcript.find("ROM_STDOUT_FIRST") < transcript.find("ROM_STDOUT_SECOND"),
    "{transcript:?}"
  );
}
