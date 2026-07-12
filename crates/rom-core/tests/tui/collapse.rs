use super::support::*;

#[test]
fn tui_prefers_active_subtrees_near_graph_root() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let waiting_id = add_derivation(&mut state, "waiting-only-1.0");
  let active_parent_id = add_derivation(&mut state, "active-parent-1.0");
  let active_leaf_id = add_derivation(&mut state, "active-leaf-1.0");

  for child_id in [waiting_id, active_parent_id] {
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
  }
  state
    .get_derivation_info_mut(active_parent_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: active_leaf_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(active_leaf_id)
    .unwrap()
    .derivation_parents
    .insert(active_parent_id);

  for drv_id in [root_id, waiting_id, active_parent_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
  state.update_build_status(
    active_leaf_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time(),
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let active_row = row_containing(&terminal, "active-leaf-1.0")
    .expect("active subtree should render");
  let root_row =
    row_containing(&terminal, "root-1.0").expect("root should render");
  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("waiting-only-1.0"),
    "spare graph capacity should show a waiting sibling: {rendered}"
  );
  assert!(
    active_row < root_row,
    "active subtree should remain connected above the root: \
     active={active_row} root={root_row}"
  );
}

#[test]
fn tui_uses_spare_capacity_for_waiting_subtrees() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let parent_id = add_derivation(&mut state, "waiting-parent-1.0");
  let child_id = add_derivation(&mut state, "waiting-child-1.0");
  let leaf_id = add_derivation(&mut state, "waiting-leaf-1.0");

  for (parent, child) in [
    (root_id, parent_id),
    (parent_id, child_id),
    (child_id, leaf_id),
  ] {
    state
      .get_derivation_info_mut(parent)
      .unwrap()
      .input_derivations
      .push(InputDerivation {
        derivation: child,
        outputs:    HashSet::new(),
      });
    state
      .get_derivation_info_mut(child)
      .unwrap()
      .derivation_parents
      .insert(parent);
  }

  for drv_id in [leaf_id, child_id, parent_id, root_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  for name in [
    "waiting-parent-1.0",
    "waiting-child-1.0",
    "waiting-leaf-1.0",
  ] {
    assert!(
      rendered.contains(name),
      "spare graph capacity should show waiting path {name}: {rendered}"
    );
  }
}

#[test]
fn tui_limits_large_planned_root_sets() {
  let backend = TestBackend::new(80, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();

  for index in 0..100 {
    let drv_id = add_derivation(&mut state, &format!("planned-{index:03}"));
    state.update_build_status(drv_id, BuildStatus::Planned);
    state.forest_roots.push(drv_id);
  }
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("planned-000"),
    "large root sets should retain the first/highest-priority root: {rendered}"
  );
  assert!(
    rendered.contains("planned-001"),
    "equally ranked roots should use stable derivation order: {rendered}"
  );
  assert!(
    !rendered.contains("planned-050"),
    "large root sets should not render every planned root: {rendered}"
  );
  assert!(
    rendered.contains("100 builds") && rendered.contains("│ 100         │"),
    "build table should report the waiting root count: {rendered}"
  );
}

#[test]
fn snapshot_never_drops_mandatory_running_roots() {
  let mut state = State::new();
  for index in 0..257 {
    let drv_id = add_derivation(&mut state, &format!("running-{index:03}"));
    state.update_build_status(
      drv_id,
      BuildStatus::Building(BuildInfo {
        start:       current_time(),
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
    state.forest_roots.push(drv_id);
  }

  let snapshot = state.render_snapshot();
  assert_eq!(snapshot.forest_roots.len(), 257);
  assert_eq!(snapshot.derivation_infos.len(), 257);
}

#[test]
fn tui_uses_spare_capacity_for_completed_dependencies() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let completed_id = add_derivation(&mut state, "completed-dep-1.0");
  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: completed_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(completed_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);

  let now = current_time();
  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(completed_id, BuildStatus::Built {
    info: BuildInfo {
      start:       now - 10.0,
      host:        cognos::Host::Localhost,
      activity_id: None,
    },
    end:  now - 6.0,
  });
  state.forest_roots.push(root_id);
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("completed-dep-1.0"),
    "spare graph capacity should show completed dependency: {rendered}"
  );
  assert!(
    !rendered.contains("○"),
    "waiting rows should be color-coded instead of using a marker: {rendered}"
  );
}

#[test]
fn tui_collapses_shared_dependency_references_into_parent_summary() {
  let backend = TestBackend::new(80, 28);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let first_root_id = add_derivation(&mut state, "first-root-1.0");
  let second_root_id = add_derivation(&mut state, "second-root-1.0");
  let shared_id = add_derivation(&mut state, "shared-dep-1.0");

  for root_id in [first_root_id, second_root_id] {
    state
      .get_derivation_info_mut(root_id)
      .unwrap()
      .input_derivations
      .push(InputDerivation {
        derivation: shared_id,
        outputs:    HashSet::new(),
      });
    state
      .get_derivation_info_mut(shared_id)
      .unwrap()
      .derivation_parents
      .insert(root_id);
    state.update_build_status(root_id, BuildStatus::Planned);
    state.forest_roots.push(root_id);
  }

  state.update_build_status(
    shared_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time(),
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("shared-dep-1.0"),
    "shared dependency should render fully once: {rendered}"
  );
  let second_root_row = row_text(
    &terminal,
    row_containing(&terminal, "second-root-1.0").expect("second root"),
  );
  assert!(
    second_root_row.contains("shared 1"),
    "second occurrence should collapse into a parent summary: {rendered}"
  );
  assert!(
    !rendered.contains("⧉"),
    "shared dependency references should not use a marker glyph: {rendered}"
  );
  assert!(
    !rendered.contains("↳"),
    "shared dependency reference should not use a directional arrow: \
     {rendered}"
  );
  assert_eq!(
    rendered.matches("shared-dep-1.0").count(),
    1,
    "shared dependency should not duplicate its active subtree or reference \
     rows: {rendered}"
  );
}

#[test]
fn tui_completed_builds_render_only_when_capacity_and_relevance_allow() {
  let backend = TestBackend::new(80, 20);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "done-1.0");
  let now = current_time();
  state.update_build_status(drv_id, BuildStatus::Built {
    info: BuildInfo {
      start:       now - 3.0,
      host:        cognos::Host::Localhost,
      activity_id: None,
    },
    end:  now - 1.0,
  });
  state.forest_roots.push(drv_id);
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("✓"),
    "completed build should show success status: {rendered}"
  );
  assert!(
    rendered.contains("✓ done-1.0"),
    "recent completed build should linger briefly: {rendered}"
  );
  assert!(
    !rendered.contains("~")
      && !rendered.contains("╭")
      && !rendered.contains("╰"),
    "completed activity should not use the old wind/leaf treatment: {rendered}"
  );

  let backend = TestBackend::new(80, 9);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "old-done-1.0");
  state.update_build_status(drv_id, BuildStatus::Built {
    info: BuildInfo {
      start:       now - 9.0,
      host:        cognos::Host::Localhost,
      activity_id: None,
    },
    end:  now - 6.0,
  });
  state.forest_roots.push(drv_id);
  for index in 0..6 {
    let running_id = add_derivation(&mut state, &format!("running-{index}"));
    state.update_build_status(
      running_id,
      BuildStatus::Building(BuildInfo {
        start:       now,
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
    state.forest_roots.push(running_id);
  }

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("old-done-1.0"),
    "the one-row footer fallback should leave spare graph capacity: {rendered}"
  );
}

#[test]
fn tui_color_codes_activity_statuses() {
  let backend = TestBackend::new(80, 20);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let building_id = add_derivation(&mut state, "building-1.0");
  let waiting_id = add_derivation(&mut state, "waiting-1.0");
  let failed_id = add_derivation(&mut state, "failed-1.0");
  let now = current_time();

  state.update_build_status(
    building_id,
    BuildStatus::Building(BuildInfo {
      start:       now,
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  state.update_build_status(waiting_id, BuildStatus::Planned);
  state.update_build_status(failed_id, BuildStatus::Failed {
    info: BuildInfo {
      start:       now - 3.0,
      host:        cognos::Host::Localhost,
      activity_id: None,
    },
    fail: rom_core::state::BuildFail {
      at:        now,
      fail_type: FailType::Unknown,
    },
  });
  state
    .forest_roots
    .extend([building_id, waiting_id, failed_id]);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let buffer = terminal.backend().buffer();
  let building_row =
    row_containing(&terminal, "building-1.0").expect("building row");
  let waiting_row =
    row_containing(&terminal, "waiting-1.0").expect("waiting row");
  let failed_row = row_containing(&terminal, "failed-1.0").expect("failed row");

  assert_eq!(buffer[(0, building_row)].style.foreground, Some(MOSS_GREEN));
  assert_eq!(
    buffer[(0, waiting_row)].style.foreground,
    Some(MUTED_YELLOW)
  );
  assert_eq!(buffer[(3, failed_row)].style.foreground, Some(MUTED_RED));
}

#[test]
fn tui_uses_muted_graph_connector_lines() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let child_id = add_derivation(&mut state, "child-1.0");

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
  state.update_build_status(
    child_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time(),
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let buffer = terminal.backend().buffer();
  let child_row =
    row_containing(&terminal, "child-1.0").expect("child row should render");
  assert_eq!(
    buffer[(0, child_row)].style.foreground,
    Some(GRAPH_LINE_COLOR)
  );
  assert!(!buffer[(0, child_row)].style.attributes.has(Attribute::Dim));
}
