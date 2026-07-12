#![cfg(unix)]

use std::{
  fs,
  os::unix::fs::PermissionsExt,
  process::Command,
  time::{Duration, Instant},
};

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
    .env("SHELL", "/bin/sh")
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

#[test]
fn nonzero_evaluator_exit_is_visible_without_structured_stderr() {
  let dir = tempfile::tempdir().unwrap();
  write_fake_nix(dir.path(), "#!/bin/sh\necho plain failure >&2\nexit 7\n");
  let path = format!(
    "{}:{}",
    dir.path().display(),
    std::env::var("PATH").unwrap_or_default()
  );
  let output = Command::new(env!("CARGO_BIN_EXE_rom"))
    .args(["--platform", "nix", "build", "fake"])
    .env("PATH", path)
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(7));
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("Evaluator exited with status 7"),
    "{stderr}"
  );
}

#[test]
fn resize_preserves_unretained_child_stdout_and_avoids_full_clear() {
  if Command::new("script").arg("--version").output().is_err() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  write_fake_nix(
    dir.path(),
    "#!/bin/sh\nprintf 'BEFORE_RESIZE\\n'\nsleep 0.1\nstty cols 40 < \
     /dev/tty\nprintf 'AFTER_RESIZE\\n'\nsleep 0.1\n",
  );
  let rom = env!("CARGO_BIN_EXE_rom");
  let command = format!("'{rom}' --platform nix build fake < /dev/null");
  let path = format!(
    "{}:{}",
    dir.path().display(),
    std::env::var("PATH").unwrap_or_default()
  );
  let output = Command::new("script")
    .args(["-qfec", &command, "/dev/null"])
    .env("PATH", path)
    .env("SHELL", "/bin/sh")
    .output()
    .unwrap();
  let transcript = String::from_utf8_lossy(&output.stdout);
  assert_eq!(
    transcript.matches("BEFORE_RESIZE").count(),
    1,
    "{transcript:?}"
  );
  assert_eq!(
    transcript.matches("AFTER_RESIZE").count(),
    1,
    "{transcript:?}"
  );
  assert!(!transcript.contains("\x1b[2J"), "{transcript:?}");
  // Redirected stdin disables raw input only; stderr still uses coordinated
  // terminal rendering.
  assert!(transcript.contains("\x1b[?2026h"), "{transcript:?}");
}

#[test]
fn failing_stderr_sink_does_not_deadlock_a_full_log_queue() {
  if Command::new("timeout").arg("--version").output().is_err() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  write_fake_nix(
    dir.path(),
    "#!/bin/sh\ni=0; while [ $i -lt 3000 ]; do echo \
     giant-log-$i-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx >&2; i=$((i+1)); \
     done\nwhile :; do :; done\n",
  );
  let rom = env!("CARGO_BIN_EXE_rom");
  let path = format!(
    "{}:{}",
    dir.path().display(),
    std::env::var("PATH").unwrap_or_default()
  );
  let started = Instant::now();
  let status = Command::new("timeout")
    .args(["5", rom, "--platform", "nix", "build", "fake"])
    .env("PATH", path)
    .stderr(fs::File::options().write(true).open("/dev/full").unwrap())
    .status()
    .unwrap();
  assert_ne!(status.code(), Some(124), "monitor timed out");
  assert!(started.elapsed() < Duration::from_secs(5));
}

fn write_fake_nix(
  directory: &std::path::Path,
  body: &str,
) -> std::path::PathBuf {
  let nix = directory.join("nix");
  fs::write(&nix, body).unwrap();
  let mut permissions = fs::metadata(&nix).unwrap().permissions();
  permissions.set_mode(0o755);
  fs::set_permissions(&nix, permissions).unwrap();
  nix
}
