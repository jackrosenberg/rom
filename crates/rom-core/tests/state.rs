use rom_core::{
  graph::GraphIndexer,
  state::{
    ActivityStatus,
    BuildInfo,
    BuildStatus,
    Derivation,
    InputDerivation,
    State,
    StorePath,
    TransferInfo,
  },
};

#[test]
fn test_state_creation() {
  let state = State::new();
  assert_eq!(state.total_builds(), 0);
}

#[test]
fn test_get_or_create_ids() {
  let mut state = State::new();
  let path = StorePath::parse("/nix/store/abc123-hello-1.0").unwrap();
  let id1 = state.get_or_create_store_path_id(path.clone());
  let id2 = state.get_or_create_store_path_id(path);
  assert_eq!(id1, id2);
}

#[test]
fn plan_derivation_marks_requested_build_as_waiting_root() {
  let mut state = State::new();
  let drv =
    Derivation::parse("/nix/store/abc123-nixos-system-fool.drv").unwrap();

  let drv_id = state.plan_derivation(drv);

  let info = state.get_derivation_info(drv_id).unwrap();
  assert!(matches!(info.build_status, BuildStatus::Planned));
  assert!(state.full_summary.planned_builds.contains(&drv_id));
  assert!(state.forest_roots.contains(&drv_id));
}

#[test]
fn render_snapshot_drops_transient_diagnostics() {
  let mut state = State::new();
  let drv = Derivation::parse("/nix/store/abc123-hello.drv").unwrap();
  let drv_id = state.plan_derivation(drv);
  state.nix_errors.push("error: failed".to_string());

  let snapshot = state.render_snapshot();

  assert!(snapshot.get_derivation_info(drv_id).is_some());
  assert!(snapshot.full_summary.planned_builds.contains(&drv_id));
}

#[test]
fn render_snapshot_keeps_transfer_store_path_names() {
  let mut state = State::new();
  let path = StorePath::parse("/nix/store/abc123-source-tarball-1.0").unwrap();
  let path_id = state.get_or_create_store_path_id(path);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             rom_core::state::current_time(),
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 0,
      total_bytes:       None,
    });

  let snapshot = state.render_snapshot();

  assert_eq!(
    snapshot.get_store_path_info(path_id).unwrap().name.name,
    "source-tarball-1.0"
  );
}

#[test]
fn render_snapshot_prunes_unfocused_derivations() {
  let mut state = State::new();
  let root_id = state
    .plan_derivation(Derivation::parse("/nix/store/abc123-root.drv").unwrap());
  let active_id = state.get_or_create_derivation_id(
    Derivation::parse("/nix/store/abc123-active.drv").unwrap(),
  );
  let inactive_id = state.get_or_create_derivation_id(
    Derivation::parse("/nix/store/abc123-inactive.drv").unwrap(),
  );

  for child_id in [active_id, inactive_id] {
    state
      .get_derivation_info_mut(root_id)
      .unwrap()
      .input_derivations
      .push(InputDerivation {
        derivation: child_id,
        outputs:    std::collections::HashSet::new(),
      });
    state
      .get_derivation_info_mut(child_id)
      .unwrap()
      .derivation_parents
      .insert(root_id);
  }

  state.update_build_status(inactive_id, BuildStatus::Planned);
  state.update_build_status(
    active_id,
    BuildStatus::Building(BuildInfo {
      start:       rom_core::state::current_time(),
      host:        cognos::Host::Localhost,
      activity_id: Some(7),
    }),
  );

  for i in 0..400 {
    state.plan_derivation(
      Derivation::parse(&format!("/nix/store/abc123-unrelated-{i}.drv"))
        .unwrap(),
    );
  }

  let snapshot = state.render_snapshot();

  assert!(
    snapshot.derivation_infos.len() < state.derivation_infos.len(),
    "render snapshot should not clone the full indexed graph"
  );
  assert!(snapshot.get_derivation_info(root_id).is_some());
  assert!(snapshot.get_derivation_info(active_id).is_some());
  assert!(
    snapshot.get_derivation_info(inactive_id).is_some(),
    "direct inactive inputs are retained so collapsed counts stay accurate"
  );
}

#[test]
fn render_snapshot_expands_relevant_optional_branches_beyond_direct_children() {
  let mut state = State::new();
  let root_id = state
    .plan_derivation(Derivation::parse("/nix/store/abc123-root.drv").unwrap());
  let middle_id = state.get_or_create_derivation_id(
    Derivation::parse("/nix/store/abc123-middle.drv").unwrap(),
  );
  let leaf_id = state.get_or_create_derivation_id(
    Derivation::parse("/nix/store/abc123-leaf.drv").unwrap(),
  );

  for (parent, child) in [(root_id, middle_id), (middle_id, leaf_id)] {
    state
      .get_derivation_info_mut(parent)
      .unwrap()
      .input_derivations
      .push(InputDerivation {
        derivation: child,
        outputs:    std::collections::HashSet::new(),
      });
    state
      .get_derivation_info_mut(child)
      .unwrap()
      .derivation_parents
      .insert(parent);
  }
  state.update_build_status(leaf_id, BuildStatus::Planned);

  let snapshot = state.render_snapshot();

  assert!(snapshot.get_derivation_info(root_id).is_some());
  assert!(snapshot.get_derivation_info(middle_id).is_some());
  assert!(
    snapshot.get_derivation_info(leaf_id).is_some(),
    "global activity ranking needs candidates deeper than one edge"
  );
  assert_eq!(
    snapshot
      .get_derivation_info(middle_id)
      .unwrap()
      .input_derivations
      .first()
      .map(|input| input.derivation),
    Some(leaf_id)
  );
}

#[test]
fn render_snapshot_keeps_visible_activity_phases_only() {
  let mut state = State::new();
  let drv_id = state.get_or_create_derivation_id(
    Derivation::parse("/nix/store/abc123-building.drv").unwrap(),
  );
  state.update_build_status(
    drv_id,
    BuildStatus::Building(BuildInfo {
      start:       rom_core::state::current_time(),
      host:        cognos::Host::Localhost,
      activity_id: Some(7),
    }),
  );
  state.activities.insert(7, ActivityStatus {
    activity: cognos::Activities::Build as u8,
    text:     "building '/nix/store/abc123-building.drv'".to_string(),
    parent:   None,
    phase:    Some("configurePhase".to_string()),
    progress: None,
  });
  state.activities.insert(8, ActivityStatus {
    activity: cognos::Activities::Build as u8,
    text:     "unrelated".to_string(),
    parent:   None,
    phase:    Some("buildPhase".to_string()),
    progress: None,
  });

  let snapshot = state.render_snapshot();

  assert!(snapshot.activities.contains_key(&7));
  assert!(!snapshot.activities.contains_key(&8));
}

#[test]
fn summary_propagation_handles_deep_shared_and_cyclic_graphs() {
  let mut state = State::new();
  let ids = (0..2_500)
    .map(|index| {
      state.get_or_create_derivation_id(
        Derivation::parse(&format!("/nix/store/hash-node-{index}.drv"))
          .unwrap(),
      )
    })
    .collect::<Vec<_>>();

  for edge in ids.windows(2) {
    connect(&mut state, edge[0], edge[1]);
  }
  // The extra edge exercises shared-descendant de-duplication.
  connect(&mut state, ids[100], ids[2_499]);

  state.update_build_status(ids[2_499], BuildStatus::Planned);

  assert!(
    state
      .get_derivation_info(ids[0])
      .unwrap()
      .dependency_summary
      .planned_builds
      .contains(&ids[2_499])
  );

  let mut cyclic = State::new();
  let a = cyclic.get_or_create_derivation_id(
    Derivation::parse("/nix/store/hash-cycle-a.drv").unwrap(),
  );
  let b = cyclic.get_or_create_derivation_id(
    Derivation::parse("/nix/store/hash-cycle-b.drv").unwrap(),
  );
  let c = cyclic.get_or_create_derivation_id(
    Derivation::parse("/nix/store/hash-cycle-c.drv").unwrap(),
  );
  connect(&mut cyclic, a, b);
  connect(&mut cyclic, b, c);
  connect(&mut cyclic, c, a);
  cyclic.update_build_status(c, BuildStatus::Planned);
  let once = cyclic
    .get_derivation_info(a)
    .unwrap()
    .dependency_summary
    .clone();
  cyclic.update_build_status(c, BuildStatus::Planned);
  let twice = &cyclic.get_derivation_info(a).unwrap().dependency_summary;
  assert_eq!(once.planned_builds, twice.planned_builds);
}

fn connect(state: &mut State, parent: usize, child: usize) {
  state
    .get_derivation_info_mut(parent)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: child,
      outputs:    std::collections::HashSet::new(),
    });
  state
    .get_derivation_info_mut(child)
    .unwrap()
    .derivation_parents
    .insert(parent);
}

#[test]
fn planned_derivation_dependencies_are_populated_incrementally() {
  let dir = tempfile::tempdir().unwrap();
  let leaf_path =
    write_test_drv(dir.path(), "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-leaf", &[]);
  let root_path =
    write_test_drv(dir.path(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-root", &[
      &leaf_path,
    ]);

  let mut state = State::new();
  let mut graph = GraphIndexer::new();
  let root_id =
    graph.plan_derivation(&mut state, Derivation::parse(&root_path).unwrap());
  let leaf_id =
    graph.plan_derivation(&mut state, Derivation::parse(&leaf_path).unwrap());

  assert!(state.forest_roots.contains(&root_id));
  assert!(state.forest_roots.contains(&leaf_id));

  wait_for_graph(&mut graph, &mut state, |state| {
    state
      .get_derivation_info(root_id)
      .unwrap()
      .input_derivations
      .iter()
      .any(|input| input.derivation == leaf_id)
  });

  let root_inputs: Vec<InputDerivation> = state
    .get_derivation_info(root_id)
    .unwrap()
    .input_derivations
    .clone();
  assert!(root_inputs.iter().any(|input| input.derivation == leaf_id));
  assert!(
    state
      .get_derivation_info(leaf_id)
      .unwrap()
      .derivation_parents
      .contains(&root_id)
  );
  assert!(state.forest_roots.contains(&root_id));
  assert!(!state.forest_roots.contains(&leaf_id));
}

fn wait_for_graph(
  graph: &mut GraphIndexer,
  state: &mut State,
  ready: impl Fn(&State) -> bool,
) {
  for _ in 0..100 {
    graph.populate_pending(state, 4);
    if ready(state) {
      return;
    }
    std::thread::sleep(std::time::Duration::from_millis(10));
  }
  panic!("graph indexer did not finish in time");
}

fn write_test_drv(
  dir: &std::path::Path,
  name: &str,
  input_paths: &[&str],
) -> String {
  let path = dir.join(format!("{name}.drv"));
  let output_path = format!("/nix/store/{name}-out");
  let inputs = input_paths
    .iter()
    .map(|input| format!(r#"("{input}",["out"])"#))
    .collect::<Vec<_>>()
    .join(",");
  let content = format!(
    r#"Derive([("out","{output_path}","","")],[{inputs}],[],"x86_64-linux","/bin/sh",[],[("name","{name}")])"#
  );
  std::fs::write(&path, content).unwrap();
  path.display().to_string()
}
