use super::support::*;

#[test]
fn live_graph_keeps_status_summary_directly_below_activity() {
  let state = running_state();
  let screen =
    render_graph_screen(80, 8, &state.render_snapshot(), &tui_config());

  let first_activity = screen.row_text(0).unwrap();
  let progress_border = screen.row_text(1).unwrap();
  let status = screen.row_text(2).unwrap();
  let bottom_border = screen.row_text(3).unwrap();
  assert!(
    first_activity.contains("hello-1.0"),
    "graph should begin without a redundant title: {first_activity:?}"
  );
  assert!(progress_border.starts_with('┌'), "{progress_border:?}");
  assert!(
    status.contains("Building 1"),
    "unexpected status: {status:?}"
  );
  assert!(bottom_border.starts_with('└'), "{bottom_border:?}");
  assert_eq!(
    screen.height(),
    4,
    "compact live region should not retain blank rows"
  );
}

#[test]
fn wide_graph_keeps_activity_metadata_inline() {
  let mut state = running_state();
  let drv_id = state.forest_roots[0];
  if let BuildStatus::Building(build) = &mut state
    .get_derivation_info_mut(drv_id)
    .expect("running derivation")
    .build_status
  {
    build.start = current_time() - 4.0;
  }
  let screen =
    render_graph_screen(180, 8, &state.render_snapshot(), &tui_config());
  let row = screen.row_text(0).expect("activity row");
  assert!(
    row.contains("hello-1.0 · 4s"),
    "wide terminals should not push metadata to the right edge: {row:?}"
  );
}

#[test]
fn live_graph_footer_reports_multiple_roots() {
  let mut state = State::new();
  for name in ["first-1.0", "second-1.0"] {
    let id = add_derivation(&mut state, name);
    state.update_build_status(id, BuildStatus::Planned);
    state.forest_roots.push(id);
  }

  let screen =
    render_graph_screen(80, 8, &state.render_snapshot(), &tui_config());
  assert!(
    screen.plain_text().contains("Waiting 2"),
    "missing build context: {}",
    screen.plain_text()
  );
}

#[test]
fn tui_places_older_running_builds_closer_to_the_parent() {
  let backend = TestBackend::new(80, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root = add_derivation(&mut state, "root-1.0");
  let old = add_derivation(&mut state, "old-1.0");
  let new = add_derivation(&mut state, "new-1.0");
  for child in [new, old] {
    state
      .get_derivation_info_mut(root)
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
      .insert(root);
  }
  let now = current_time();
  for (id, start) in [(new, now - 2.0), (old, now - 20.0)] {
    state.update_build_status(
      id,
      BuildStatus::Building(BuildInfo {
        start,
        host: cognos::Host::Localhost,
        activity_id: None,
      }),
    );
  }
  state.forest_roots.push(root);

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();
  let old_row = row_containing(&terminal, "old-1.0").unwrap();
  let new_row = row_containing(&terminal, "new-1.0").unwrap();
  assert!(
    old_row > new_row,
    "older build should remain closest to its parent at the bottom"
  );
}

#[test]
fn tui_places_earlier_failures_closer_to_the_parent() {
  let backend = TestBackend::new(100, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root = add_derivation(&mut state, "root-1.0");
  let early = add_derivation(&mut state, "early-failure-1.0");
  let late = add_derivation(&mut state, "late-failure-1.0");
  for child in [late, early] {
    state
      .get_derivation_info_mut(root)
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
      .insert(root);
  }
  let now = current_time();
  for (id, at) in [(late, now - 2.0), (early, now - 20.0)] {
    state.update_build_status(id, BuildStatus::Failed {
      info: BuildInfo {
        start:       at - 1.0,
        host:        cognos::Host::Localhost,
        activity_id: None,
      },
      fail: BuildFail {
        at,
        fail_type: FailType::BuildFailed(1),
      },
    });
  }
  state.forest_roots.push(root);

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();
  let early_row = row_containing(&terminal, "early-failure-1.0").unwrap();
  let late_row = row_containing(&terminal, "late-failure-1.0").unwrap();
  assert!(
    early_row > late_row,
    "earlier failure should remain closest to its parent at the bottom"
  );
}

#[test]
fn tui_disambiguates_same_names_by_platform_only_when_needed() {
  let backend = TestBackend::new(100, 10);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let first = state.get_or_create_derivation_id(
    Derivation::parse(
      "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-shared-1.0.drv",
    )
    .unwrap(),
  );
  let second = state.get_or_create_derivation_id(
    Derivation::parse(
      "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-shared-1.0.drv",
    )
    .unwrap(),
  );
  state.get_derivation_info_mut(first).unwrap().platform =
    Some("x86_64-linux".to_string());
  state.get_derivation_info_mut(second).unwrap().platform =
    Some("aarch64-linux".to_string());
  for id in [first, second] {
    state.update_build_status(id, BuildStatus::Planned);
    state.forest_roots.push(id);
  }

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();
  let rendered = format!("{}", terminal.backend());
  assert!(rendered.contains("shared-1.0 [x86_64-linux]"));
  assert!(rendered.contains("shared-1.0 [aarch64-linux]"));
}

#[test]
fn tui_uses_full_remote_hosts_when_short_labels_collide() {
  let backend = TestBackend::new(120, 10);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  for (name, host) in [
    ("first-1.0", "ssh://user@cache.one.example"),
    ("second-1.0", "ssh://user@cache.two.example"),
  ] {
    let id = add_derivation(&mut state, name);
    state.update_build_status(
      id,
      BuildStatus::Building(BuildInfo {
        start:       current_time() - 2.0,
        host:        cognos::Host::Remote(host.to_string()),
        activity_id: None,
      }),
    );
    state.forest_roots.push(id);
  }

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();
  let rendered = format!("{}", terminal.backend());
  assert!(rendered.contains("on ssh://user@cache.one.example"));
  assert!(rendered.contains("on ssh://user@cache.two.example"));
}

#[test]
fn tui_renders_running_build_as_devenv_style_activity() {
  let backend = TestBackend::new(80, 20);
  let mut terminal = Terminal::new(backend).unwrap();
  let state = running_state();
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("Building 1"),
    "build panel should report active work: {rendered}"
  );
  assert!(
    !rendered.contains("❧"),
    "old fleuron glyph should not remain: {rendered}"
  );
  assert!(
    !rendered.contains("◄") && !rendered.contains("►"),
    "old arrow glyphs should not remain: {rendered}"
  );
  assert!(
    !rendered.contains("╭")
      && !rendered.contains("╰")
      && !rendered.contains("┤"),
    "old leaf box glyphs should not remain: {rendered}"
  );
  assert!(
    rendered.contains("hello-1.0"),
    "missing build name: {rendered}"
  );
  assert!(
    !rendered.contains("Dependency Graph"),
    "old graph header should not remain in vine view: {rendered}"
  );
}

#[test]
fn final_failure_uses_the_live_console_graph_renderer() {
  let mut state = running_state();
  let drv_id = state.forest_roots[0];
  let now = current_time();
  state.update_build_status(drv_id, BuildStatus::Failed {
    info: BuildInfo {
      start:       now - 4.0,
      host:        cognos::Host::Localhost,
      activity_id: None,
    },
    fail: rom_core::state::BuildFail {
      at:        now,
      fail_type: FailType::Unknown,
    },
  });

  let screen = render_final_graph_screen(100, &state, &tui_config());
  let rendered = screen.plain_text();
  assert!(rendered.contains("hello-1.0"), "{rendered}");
  assert!(rendered.contains("Failed 1"), "{rendered}");
  assert!(!rendered.contains("Dependency Graph"), "{rendered}");
}

#[test]
fn tui_shows_evaluation_progress_before_build_graph_exists() {
  let backend = TestBackend::new(80, 16);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  state.evaluation_state.count = 42;
  state.evaluation_state.last_file_name = Some(
    "«nixpkgs»/pkgs/by-name/bc/bcachefs-tools/kernel-module.nix".to_string(),
  );
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("Evaluating"),
    "evaluation status should replace idle graph text: {rendered}"
  );
  assert!(
    rendered.contains("bcachefs-tools/kernel-module.nix"),
    "evaluation status should show the current file tail: {rendered}"
  );
  assert!(
    rendered.contains("42 files"),
    "evaluation status should show the evaluation count: {rendered}"
  );
  assert!(
    !rendered.contains("Waiting for Nix activity"),
    "evaluation should not look idle: {rendered}"
  );
}

#[test]
fn tui_renders_multiple_running_builds_as_activity_rows() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let first_id = add_derivation(&mut state, "first-1.0");
  let second_id = add_derivation(&mut state, "second-1.0");

  for drv_id in [first_id, second_id] {
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
  let config = tui_config();

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("first-1.0"),
    "first build should render as an activity row: {rendered}"
  );
  assert!(
    rendered.contains("second-1.0"),
    "second build should render as an activity row: {rendered}"
  );
  assert!(
    !rendered.contains("╭") && !rendered.contains("╰"),
    "devenv-style graph should not use decorative vine boxes: {rendered}"
  );
}

#[test]
fn tui_renders_dependency_branch_with_active_leaf() {
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

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("┌─"),
    "missing hierarchy connector: {rendered}"
  );
  assert!(
    !rendered.contains("╭") && !rendered.contains("╰"),
    "graph should use connected square guide rails instead of decorative \
     rounded vines: {rendered}"
  );
  assert!(
    rendered.contains("child-1.0"),
    "missing active dependency leaf: {rendered}"
  );
  assert!(
    rendered.contains("root-1.0"),
    "missing planned root bud: {rendered}"
  );
  let child_row =
    row_containing(&terminal, "child-1.0").expect("child row should render");
  let root_row =
    row_containing(&terminal, "root-1.0").expect("root row should render");
  assert!(
    child_row < root_row,
    "dependency should render above root: child={child_row} root={root_row}"
  );
}

#[test]
fn tui_keeps_root_visible_when_activity_graph_overflows() {
  let backend = TestBackend::new(80, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "nixos-system-fool");

  for index in 0..8 {
    let child_id = add_derivation(&mut state, &format!("dep-{index:02}"));
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
    state.update_build_status(
      child_id,
      BuildStatus::Building(BuildInfo {
        start:       current_time(),
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
  }

  state.forest_roots.push(root_id);
  let config = tui_config();
  let snapshot = state.render_snapshot();
  assert_eq!(
    rom_core::tui::minimum_required_graph_rows_at_width(100, &snapshot),
    12,
    "eight running dependencies, their unknown root, and console footer are \
     required"
  );

  terminal
    .draw(|frame| draw(frame, &snapshot, &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("nixos-system-fool"),
    "overflowing graph should keep the requested root visible: {rendered}"
  );
  for index in 0..8 {
    assert!(
      rendered.contains(&format!("dep-{index:02}")),
      "soft graph budget must retain every running dependency: {rendered}"
    );
  }
  assert!(
    !rendered.contains("hidden rows above"),
    "mandatory rows must not be replaced by a clipping summary: {rendered}"
  );
}

#[test]
fn tui_depth_limit_never_hides_a_mandatory_activity_path() {
  let backend = TestBackend::new(80, 16);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let middle_id = add_derivation(&mut state, "middle-1.0");
  let leaf_id = add_derivation(&mut state, "leaf-1.0");

  for (parent, child) in [(root_id, middle_id), (middle_id, leaf_id)] {
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
  state.update_build_status(
    leaf_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time(),
      host:        cognos::Host::Localhost,
      activity_id: None,
    }),
  );
  state.forest_roots.push(root_id);

  let mut config = tui_config();
  config.console.max_tree_depth = 1;
  terminal
    .draw(|frame| {
      draw(frame, &state.render_snapshot(), &config);
    })
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  for name in ["root-1.0", "middle-1.0", "leaf-1.0"] {
    assert!(
      rendered.contains(name),
      "mandatory path node {name} was depth-clipped: {rendered}"
    );
  }
}

#[test]
fn tui_uses_thin_connectors_for_dependency_siblings() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let first_id = add_derivation(&mut state, "first-dep-1.0");
  let middle_id = add_derivation(&mut state, "middle-dep-1.0");
  let last_id = add_derivation(&mut state, "last-dep-1.0");

  state.update_build_status(root_id, BuildStatus::Planned);
  for child_id in [first_id, middle_id, last_id] {
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
    state.update_build_status(
      child_id,
      BuildStatus::Building(BuildInfo {
        start:       current_time(),
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
  }
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let first_row = row_text(
    &terminal,
    row_containing(&terminal, "first-dep-1.0").unwrap(),
  );
  let middle_row = row_text(
    &terminal,
    row_containing(&terminal, "middle-dep-1.0").unwrap(),
  );
  let last_row = row_text(
    &terminal,
    row_containing(&terminal, "last-dep-1.0").unwrap(),
  );
  let root_row =
    row_text(&terminal, row_containing(&terminal, "root-1.0").unwrap());

  assert!(
    last_row.contains("┌─"),
    "topmost dependency should start the reversed branch: {last_row:?}"
  );
  assert!(
    middle_row.contains("├─"),
    "middle dependency should use a true intersection: {middle_row:?}"
  );
  assert!(
    first_row.contains("├─"),
    "bottom dependency should keep the branch open for the root: {first_row:?}"
  );
  assert!(
    root_row.starts_with("root-1.0"),
    "NOM-style root should remain undecorated: {root_row:?}"
  );
}

#[test]
fn tui_joins_visible_dependency_branch_into_parent() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let parent_id = add_derivation(&mut state, "parent-1.0");
  let child_id = add_derivation(&mut state, "child-1.0");

  for (parent, child) in [(root_id, parent_id), (parent_id, child_id)] {
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

  for drv_id in [root_id, parent_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
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

  let parent_row =
    row_text(&terminal, row_containing(&terminal, "parent-1.0").unwrap());
  let child_row =
    row_text(&terminal, row_containing(&terminal, "child-1.0").unwrap());
  let root_row =
    row_text(&terminal, row_containing(&terminal, "root-1.0").unwrap());
  assert!(
    child_row.starts_with("   ┌─"),
    "dependency rows should not inherit a left-edge ancestor rail: \
     {child_row:?}"
  );
  assert!(
    parent_row.starts_with("┌─"),
    "NOM-style parent row should close its dependency rail: {parent_row:?}"
  );
  assert!(
    root_row.starts_with("root-1.0"),
    "NOM-style top-level root should not carry a connector: {root_row:?}"
  );
}

#[test]
fn tui_keeps_sibling_rail_through_nested_active_subtree() {
  let backend = TestBackend::new(96, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let direct_id = add_derivation(&mut state, "direct-build-1.0");
  let parent_id = add_derivation(&mut state, "parent-1.0");
  let nested_id = add_derivation(&mut state, "nested-build-1.0");

  for child_id in [direct_id, parent_id] {
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
    .get_derivation_info_mut(parent_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: nested_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(nested_id)
    .unwrap()
    .derivation_parents
    .insert(parent_id);

  for drv_id in [root_id, parent_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
  for drv_id in [direct_id, nested_id] {
    state.update_build_status(
      drv_id,
      BuildStatus::Building(BuildInfo {
        start:       current_time(),
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
  }
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let nested_row = row_text(
    &terminal,
    row_containing(&terminal, "nested-build-1.0").unwrap(),
  );
  assert!(
    nested_row.starts_with("   ┌─"),
    "topmost nested subtree should not retain a dangling sibling rail: \
     {nested_row:?}"
  );
}

#[test]
fn tui_removes_dangling_left_rail_from_nested_sibling_subtrees() {
  let backend = TestBackend::new(96, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "nixos-system-fool");
  let system_path_id = add_derivation(&mut state, "system-path");
  let portal_id = add_derivation(&mut state, "xdg-desktop-portal-kde-6.6.5");
  let plasma_id = add_derivation(&mut state, "plasma-workspace-6.6.5");
  let bitwarden_wrapped_id =
    add_derivation(&mut state, "bitwarden-desktop-wrapped");
  let bitwarden_id = add_derivation(&mut state, "bitwarden-desktop-2026.5.0");

  for (parent, child) in [
    (root_id, system_path_id),
    (system_path_id, portal_id),
    (portal_id, plasma_id),
    (system_path_id, bitwarden_wrapped_id),
    (bitwarden_wrapped_id, bitwarden_id),
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

  for drv_id in [root_id, system_path_id, portal_id, bitwarden_wrapped_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
  for drv_id in [plasma_id, bitwarden_id] {
    state.update_build_status(
      drv_id,
      BuildStatus::Building(BuildInfo {
        start:       current_time() - 2.0,
        host:        cognos::Host::Localhost,
        activity_id: None,
      }),
    );
  }
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let plasma_row = row_text(
    &terminal,
    row_containing(&terminal, "plasma-workspace-6.6.5").unwrap(),
  );
  assert!(
    !plasma_row.starts_with("│ "),
    "nested sibling subtree should not show a dangling left rail: \
     {plasma_row:?}"
  );
}
