use std::io::Cursor;

use rom_core::{
  Config,
  InputMode,
  RomError,
  create_monitor,
  monitor_stream,
  state::{BuildStatus, FailType},
};

const DRV: &str = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hello-1.0.drv";

#[test]
fn prefixed_json_is_auto_detected_and_logs_once() {
  let input = concat!(
    "@nix {\"action\":\"msg\",\"level\":1,\"msg\":\"warning: hello\"}\n",
    "@nix {\"action\":\"result\",\"fields\":[\"compile \
     output\"],\"id\":1,\"type\":101}\n"
  );
  let mut output = Vec::new();
  monitor_stream(Config::default(), Cursor::new(input), &mut output).unwrap();
  let output = String::from_utf8(output).unwrap();

  assert_eq!(output.matches("warning: hello").count(), 1);
  assert_eq!(output.matches("compile output").count(), 1);
  assert!(!output.contains("@nix"));
  assert!(output.contains("Finished at"));
}

#[test]
fn unprefixed_json_mode_processes_actions() {
  let input =
    "{\"action\":\"msg\",\"level\":1,\"msg\":\"warning: json mode\"}\n";
  let mut output = Vec::new();
  monitor_stream(
    Config {
      input_mode: InputMode::Json,
      ..Config::default()
    },
    Cursor::new(input),
    &mut output,
  )
  .unwrap();
  let output = String::from_utf8(output).unwrap();
  assert_eq!(output.matches("warning: json mode").count(), 1);
  assert!(!output.contains("\"action\""));
}

#[test]
fn human_plan_and_build_are_preserved_and_rendered() {
  let input = format!(
    "these 1 derivations will be built:\n  {DRV}\nbuilding '{DRV}'...\n"
  );
  let mut monitor = create_monitor(Config::default(), Vec::new()).unwrap();
  monitor.process_stream(Cursor::new(&input)).unwrap();
  assert_eq!(monitor.state().derivation_infos.len(), 1);
  assert_eq!(monitor.state().full_summary.completed_builds.len(), 1);
  let output = String::from_utf8(monitor.into_writer()).unwrap();

  assert_eq!(
    output.matches("these 1 derivations will be built:").count(),
    1
  );
  assert_eq!(output.matches(&format!("building '{DRV}'...")).count(), 1);
  assert!(!output.contains("Building 0"), "{output}");
  assert!(!output.contains("Waiting for Nix activity"), "{output}");
  assert!(output.contains("Finished at"));
}

#[test]
fn public_actions_and_json_share_the_exact_once_log_path() {
  let action: cognos::Actions = serde_json::from_str(
    r#"{"action":"result","fields":["from action"],"id":1,"type":101}"#,
  )
  .unwrap();
  let mut monitor = create_monitor(Config::default(), Vec::new()).unwrap();
  monitor.process_action(action).unwrap();
  monitor
    .process_line(
      r#"@nix {"action":"result","fields":["from json"],"id":1,"type":101}"#,
    )
    .unwrap();
  monitor.finish().unwrap();
  let output = String::from_utf8(monitor.into_writer()).unwrap();
  assert_eq!(output.matches("from action").count(), 1);
  assert_eq!(output.matches("from json").count(), 1);
}

#[test]
fn human_diagnostics_are_structural_and_sgr_safe() {
  let dependency = "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-dependent.drv";
  let input = format!(
    "compiler: error: harmless builder output\n\x1b[31merror:\x1b[0m 1 \
     dependencies of derivation '{dependency}' failed to build\n"
  );
  let mut monitor = create_monitor(Config::default(), Vec::new()).unwrap();
  let result = monitor.process_stream(Cursor::new(input));
  assert!(matches!(result, Err(RomError::BuildFailed)));
  assert_eq!(monitor.state().nix_errors.len(), 1);
  let info = monitor.state().derivation_infos.values().next().unwrap();
  assert!(matches!(info.build_status, BuildStatus::Failed {
    fail: rom_core::state::BuildFail {
      fail_type: FailType::DependencyFailed,
      ..
    },
    ..
  }));
}

#[test]
fn giant_input_record_is_truncated_before_unbounded_growth() {
  let input = format!(
    "{}\n",
    "x".repeat(rom_core::monitor::MAX_INPUT_RECORD_BYTES * 4)
  );
  let mut output = Vec::new();
  monitor_stream(Config::default(), Cursor::new(input), &mut output).unwrap();
  assert!(output.len() < rom_core::monitor::MAX_INPUT_RECORD_BYTES * 2);
  assert!(
    String::from_utf8(output)
      .unwrap()
      .contains("record truncated")
  );
}

#[test]
fn final_graph_precedes_build_failed_result() {
  let input = format!(
    "building '{DRV}'...\nerror: builder for '{DRV}' failed with exit code 1\n"
  );
  let mut monitor = create_monitor(Config::default(), Vec::new()).unwrap();
  let result = monitor.process_stream(Cursor::new(input));
  assert!(matches!(result, Err(RomError::BuildFailed)));
  let output = String::from_utf8(monitor.into_writer()).unwrap();
  assert!(output.contains("hello-1.0"));
  assert!(output.contains("Exited after 1 build failure"));
}
