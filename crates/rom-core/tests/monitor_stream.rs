use std::io::Cursor;

use rom_core::{Config, InputMode, RomError, create_monitor, monitor_stream};

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
  let output = String::from_utf8(monitor.into_writer()).unwrap();

  assert_eq!(
    output.matches("these 1 derivations will be built:").count(),
    1
  );
  assert_eq!(output.matches(&format!("building '{DRV}'...")).count(), 1);
  assert!(
    output.contains("┃Building 0 · Waiting 0 · Built 1"),
    "{output}"
  );
  assert!(output.contains("Finished at"));
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
