use std::{
  io::Write,
  process::{Command, Stdio},
};

fn run_rom(args: &[&str], input: &str) -> (String, String, i32) {
  let mut child = Command::new(env!("CARGO_BIN_EXE_rom"))
    .args(args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
  child
    .stdin
    .take()
    .unwrap()
    .write_all(input.as_bytes())
    .unwrap();
  let output = child.wait_with_output().unwrap();
  (
    String::from_utf8(output.stdout).unwrap(),
    String::from_utf8(output.stderr).unwrap(),
    output.status.code().unwrap_or(-1),
  )
}

#[test]
fn no_subcommand_monitors_human_stdin() {
  let drv = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-cli-human-1.0.drv";
  let input = format!(
    "these 1 derivations will be built:\n  {drv}\nbuilding '{drv}'...\n"
  );
  let (stdout, stderr, status) = run_rom(&[], &input);
  assert_eq!(status, 0, "{stderr}");
  assert_eq!(
    stdout.matches("these 1 derivations will be built:").count(),
    1
  );
  assert!(stdout.contains("cli-human-1.0"), "{stdout}");
  assert!(stdout.contains("Finished at"));
}

#[test]
fn json_flag_accepts_unprefixed_internal_json() {
  let input =
    "{\"action\":\"msg\",\"level\":1,\"msg\":\"warning: cli json\"}\n";
  let (stdout, stderr, status) = run_rom(&["--json"], input);
  assert_eq!(status, 0, "{stderr}");
  assert_eq!(stdout.matches("warning: cli json").count(), 1);
  assert!(!stdout.contains("\"action\""));
}

#[test]
fn prefixed_json_is_auto_detected_without_flag() {
  let input =
    "@nix {\"action\":\"msg\",\"level\":1,\"msg\":\"warning: prefixed\"}\n";
  let (stdout, stderr, status) = run_rom(&[], input);
  assert_eq!(status, 0, "{stderr}");
  assert_eq!(stdout.matches("warning: prefixed").count(), 1);
  assert!(!stdout.contains("@nix"));
}

#[test]
fn public_api_is_reexported_from_rom() {
  let config: rom::Config = rom::Config::default();
  let monitor: rom::Monitor<Vec<u8>> =
    rom::create_monitor(config, Vec::new()).unwrap();
  assert_eq!(monitor.state().derivation_infos.len(), 0);
  let _mode = rom::InputMode::Human;
  let mut output = Vec::new();
  rom::monitor_stream(config, std::io::Cursor::new(""), &mut output).unwrap();
  assert!(String::from_utf8(output).unwrap().contains("Finished at"));
}
