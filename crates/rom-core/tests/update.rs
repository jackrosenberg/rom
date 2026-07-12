use std::collections::HashSet;

use cognos::{Actions, Activities, OutputName, ResultType, Verbosity};
use rom_core::{
  state::{
    BuildInfo,
    BuildStatus,
    Derivation,
    InputDerivation,
    State,
    StorePath,
  },
  update::{action_may_update_state, finish_state, process_message},
};

#[test]
fn internal_json_plan_line_marks_derivation_planned() {
  let mut state = State::new();

  let changed = process_message(&mut state, Actions::Message {
    level:   Verbosity::Info,
    msg:     "  /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-nixos-system-fool.\
              drv"
      .to_string(),
    raw_msg: None,
    file:    None,
    line:    None,
    column:  None,
  });

  assert!(changed);
  let (drv_id, info) = state.derivation_infos.iter().next().unwrap();
  assert_eq!(info.name.name, "nixos-system-fool");
  assert!(matches!(info.build_status, BuildStatus::Planned));
  assert!(state.full_summary.planned_builds.contains(drv_id));
  assert!(state.forest_roots.contains(drv_id));
}

#[test]
fn build_start_does_not_promote_known_dependency_to_root() {
  let mut state = State::new();
  let root_id = add_drv(
    &mut state,
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-root.drv",
  );
  let child_path = "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-child.drv";
  let child_id = add_drv(&mut state, child_path);

  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: child_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(child_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);
  state.update_build_status(root_id, BuildStatus::Planned);
  state.forest_roots.push(root_id);

  let changed = process_message(&mut state, Actions::Start {
    id:       1,
    level:    Verbosity::Info,
    parent:   0,
    text:     format!("building '{child_path}'"),
    activity: Activities::Build,
    fields:   vec![serde_json::json!(child_path), serde_json::json!("")],
  });

  assert!(changed);
  assert!(matches!(
    state.get_derivation_info(child_id).unwrap().build_status,
    BuildStatus::Building(BuildInfo { .. })
  ));
  assert!(state.forest_roots.contains(&root_id));
  assert!(!state.forest_roots.contains(&child_id));
}

#[test]
fn transfer_changes_refresh_derivation_and_parent_summaries() {
  let mut state = State::new();
  let parent_id = add_drv(
    &mut state,
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-parent.drv",
  );
  let producer_id = add_drv(
    &mut state,
    "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-producer.drv",
  );
  state
    .get_derivation_info_mut(parent_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: producer_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(producer_id)
    .unwrap()
    .derivation_parents
    .insert(parent_id);
  let path = "/nix/store/cccccccc-output";
  let path_id =
    state.get_or_create_store_path_id(StorePath::parse(path).unwrap());
  state.get_store_path_info_mut(path_id).unwrap().producer = Some(producer_id);
  state
    .get_derivation_info_mut(producer_id)
    .unwrap()
    .outputs
    .insert(OutputName::parse("out"), path_id);

  assert!(process_message(&mut state, Actions::Start {
    id:       42,
    level:    Verbosity::Info,
    parent:   0,
    text:     format!("copying path '{path}'"),
    activity: Activities::Substitute,
    fields:   vec![serde_json::json!(path), serde_json::json!("")],
  }));
  assert!(
    state
      .get_derivation_info(producer_id)
      .unwrap()
      .dependency_summary
      .running_downloads
      .contains_key(&path_id)
  );
  assert!(
    state
      .get_derivation_info(parent_id)
      .unwrap()
      .dependency_summary
      .running_downloads
      .contains_key(&path_id)
  );

  assert!(process_message(&mut state, Actions::Stop { id: 42 }));
  assert!(
    state
      .get_derivation_info(parent_id)
      .unwrap()
      .dependency_summary
      .running_downloads
      .is_empty()
  );
}

#[test]
fn child_activity_progress_updates_live_and_completed_cache_transfer() {
  let mut state = State::new();
  let path = "/nix/store/cccccccc-output";
  let path_id =
    state.get_or_create_store_path_id(StorePath::parse(path).unwrap());

  assert!(process_message(&mut state, Actions::Start {
    id:       42,
    level:    Verbosity::Info,
    parent:   0,
    text:     format!("copying path '{path}'"),
    activity: Activities::Substitute,
    fields:   vec![serde_json::json!(path), serde_json::json!("cache.test")],
  }));
  assert!(process_message(&mut state, Actions::Start {
    id:       43,
    level:    Verbosity::Info,
    parent:   42,
    text:     "downloading".to_string(),
    activity: Activities::FileTransfer,
    fields:   vec![serde_json::json!("https://cache.test/nar")],
  }));
  assert!(process_message(&mut state, Actions::Result {
    id:          43,
    result_type: ResultType::Progress,
    fields:      vec![
      serde_json::json!(768),
      serde_json::json!(1024),
      serde_json::json!(1),
      serde_json::json!(0),
    ],
  }));

  let running = &state.full_summary.running_downloads[&path_id];
  assert_eq!(running.bytes_transferred, 768);
  assert_eq!(running.total_bytes, Some(1024));

  assert!(process_message(&mut state, Actions::Stop { id: 42 }));
  assert_eq!(
    state.full_summary.completed_downloads[&path_id].total_bytes,
    768
  );
}

#[test]
fn ordinary_messages_are_log_only() {
  let action = Actions::Message {
    level:   Verbosity::Info,
    msg:     "kio-extras> -- Found samba: /nix/store/example".to_string(),
    raw_msg: None,
    file:    None,
    line:    None,
    column:  None,
  };

  assert!(!action_may_update_state(&action));

  let mut state = State::new();
  assert!(!process_message(&mut state, action));
  assert_eq!(state.evaluation_state.count, 0);
  assert!(state.nix_errors.is_empty());
  assert!(state.derivation_infos.is_empty());
}

#[test]
fn warning_messages_are_log_only_unless_structural() {
  let action = Actions::Message {
    level:   Verbosity::Warning,
    msg:     "warning: noisy configure output".to_string(),
    raw_msg: None,
    file:    None,
    line:    None,
    column:  None,
  };

  assert!(!action_may_update_state(&action));

  let mut state = State::new();
  assert!(!process_message(&mut state, action));
}

#[test]
fn evaluation_and_error_messages_still_update_state() {
  let eval = Actions::Message {
    level:   Verbosity::Info,
    msg:     "evaluating file '/nix/store/source/default.nix'".to_string(),
    raw_msg: None,
    file:    None,
    line:    None,
    column:  None,
  };
  let error = Actions::Message {
    level:   Verbosity::Error,
    msg:     "error: builder failed".to_string(),
    raw_msg: None,
    file:    None,
    line:    None,
    column:  None,
  };

  assert!(action_may_update_state(&eval));
  assert!(action_may_update_state(&error));

  let mut state = State::new();
  assert!(process_message(&mut state, eval));
  assert_eq!(state.evaluation_state.count, 1);
  assert!(process_message(&mut state, error));
  assert_eq!(state.nix_errors, vec!["error: builder failed"]);
}

#[test]
fn finish_state_records_transferred_bytes_not_expected_total() {
  let mut state = State::new();
  let path = StorePath::parse("/nix/store/cccccccc-interrupted").unwrap();
  let path_id = state.get_or_create_store_path_id(path);
  state.full_summary.running_downloads.insert(
    path_id,
    rom_core::state::TransferInfo {
      start:             0.0,
      host:              cognos::Host::Localhost,
      activity_id:       7,
      bytes_transferred: 37,
      total_bytes:       Some(1_000),
    },
  );

  finish_state(&mut state);

  assert_eq!(
    state.full_summary.completed_downloads[&path_id].total_bytes,
    37
  );
}

fn add_drv(state: &mut State, path: &str) -> usize {
  state.get_or_create_derivation_id(Derivation::parse(path).unwrap())
}
