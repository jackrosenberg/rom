use super::support::*;

#[test]
fn tui_renders_running_uploads_as_first_class_graph_activity() {
  let backend = TestBackend::new(100, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "release-1.0");
  let path_id = add_output_path(&mut state, drv_id, "release-1.0");
  state.update_build_status(drv_id, BuildStatus::Planned);
  state
    .full_summary
    .running_uploads
    .insert(path_id, TransferInfo {
      start:             current_time() - 4.0,
      host:              cognos::Host::Remote(
        "ssh://builder@cache.example.org".to_string(),
      ),
      activity_id:       77,
      bytes_transferred: 1_024,
      total_bytes:       Some(2_048),
    });
  state.forest_roots.push(drv_id);

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();

  let row = row_text(
    &terminal,
    row_containing(&terminal, "release-1.0").expect("upload row"),
  );
  assert!(row.contains('↑'), "missing upload marker: {row:?}");
  assert!(
    row.contains("to cache"),
    "missing short upload host: {row:?}"
  );
  assert!(row.contains("1.0 KiB / 2.0 KiB"), "missing bytes: {row:?}");
  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("cache.example.org"),
    "active upload cache should be listed in the footer"
  );
  let cache_row = row_text(
    &terminal,
    row_containing(&terminal, "cache.example.org").expect("cache row"),
  );
  assert!(
    cache_row.find("BUILD").is_some_and(|build| {
      cache_row
        .find("cache.example.org")
        .is_some_and(|cache| build < cache)
    }),
    "build sidecar should remain left of cache activity: {cache_row:?}"
  );
}

#[test]
fn cache_sidecar_retains_completed_cache_activity() {
  let backend = TestBackend::new(100, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "cached-1.0");
  let path_id = add_output_path(&mut state, drv_id, "cached-1.0");
  state.update_build_status(drv_id, BuildStatus::Planned);
  state.full_summary.completed_downloads.insert(
    path_id,
    CompletedTransferInfo {
      start:       current_time() - 5.0,
      end:         current_time() - 2.0,
      host:        cognos::Host::Remote("https://cache.nixos.org".to_string()),
      total_bytes: 4_096,
    },
  );
  state.forest_roots.push(drv_id);

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(rendered.contains("cache.nixos.org"), "{rendered}");
  assert!(rendered.contains("1 / 1"), "{rendered}");
  assert!(rendered.contains("100%"), "{rendered}");
}

#[test]
fn tui_keeps_secondary_transfers_visible_and_selects_primary_deterministically()
{
  let backend = TestBackend::new(100, 12);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let drv_id = add_derivation(&mut state, "consumer-1.0");
  let first_path = add_output_path(&mut state, drv_id, "first-output-1.0");
  let second_path = add_store_path(&mut state, "second-output-1.0");
  state.get_store_path_info_mut(second_path).unwrap().producer = Some(drv_id);
  state.update_build_status(drv_id, BuildStatus::Planned);
  let now = current_time();
  for (path, start, bytes) in
    [(first_path, now - 20.0, 100), (second_path, now - 2.0, 200)]
  {
    state
      .full_summary
      .running_downloads
      .insert(path, TransferInfo {
        start,
        host: cognos::Host::Localhost,
        activity_id: path as u64,
        bytes_transferred: bytes,
        total_bytes: Some(1_000),
      });
  }
  state.forest_roots.push(drv_id);

  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &tui_config()))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(rendered.contains("consumer-1.0"));
  assert!(
    rendered.contains("second-output-1.0"),
    "secondary attached transfer disappeared: {rendered}"
  );
  assert_eq!(rendered.matches('↓').count(), 2, "{rendered}");
}

#[test]
fn tui_prefers_structural_parent_for_shared_active_dependency() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let active_id = add_derivation(&mut state, "active-leaf-1.0");
  let aggregator_id = add_derivation(&mut state, "aggregator-1.0");

  for child_id in [active_id, aggregator_id] {
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
    .get_derivation_info_mut(aggregator_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: active_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(active_id)
    .unwrap()
    .derivation_parents
    .insert(aggregator_id);

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(aggregator_id, BuildStatus::Planned);
  state.update_build_status(
    active_id,
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
  let active_row = row_text(
    &terminal,
    row_containing(&terminal, "active-leaf-1.0").unwrap(),
  );
  let aggregator_row = row_text(
    &terminal,
    row_containing(&terminal, "aggregator-1.0").unwrap(),
  );
  assert!(
    active_row.starts_with("   ┌─"),
    "shared active dependency should render under its structural parent: \
     {active_row:?}"
  );
  assert!(
    aggregator_row.starts_with("┌─"),
    "NOM-style structural parent should close the active dependency branch: \
     {aggregator_row:?}"
  );
  let root_row = row_text(
    &terminal,
    row_containing(&terminal, "root-1.0").expect("root row"),
  );
  assert!(
    root_row.contains("shared 1"),
    "direct duplicate should collapse into the root summary: {rendered}"
  );
  assert_eq!(
    rendered.matches("active-leaf-1.0").count(),
    1,
    "active dependency should render only once: {rendered}"
  );
}

#[test]
fn tui_renders_running_downloads_inline_in_dependency_graph() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let downloaded_id = add_derivation(&mut state, "electron-41.7.1");
  let path_id = add_output_path(&mut state, downloaded_id, "electron-41.7.1");

  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: downloaded_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(downloaded_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(downloaded_id, BuildStatus::Planned);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Remote(
        "https://cache.nixos.org".to_string(),
      ),
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "electron-41.7.1").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "running substitute should render as an inline download row: \
     {download_row:?}"
  );
  assert!(
    download_row.contains("512 B / 1.0 KiB"),
    "running substitute should show transfer progress: {download_row:?}"
  );
  assert!(
    rendered.contains("0 / 1"),
    "download should still be counted in the cache table: {rendered}"
  );
  assert!(
    rendered.contains("cache.nixos.org"),
    "active download cache should be listed in the footer: {rendered}"
  );
}

#[test]
fn tui_renders_downloads_inline_by_store_path_name_without_outputs() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let downloaded_id = add_derivation(&mut state, "electron-39.8.10");
  let path_id = add_store_path(&mut state, "electron-39.8.10");

  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: downloaded_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(downloaded_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(downloaded_id, BuildStatus::Planned);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "electron-39.8.10").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "download should attach to the planned derivation by store path name when \
     output metadata has not been parsed: {download_row:?}"
  );
}

#[test]
fn tui_attaches_input_source_transfer_to_consuming_derivation() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let consumer_id = add_derivation(&mut state, "consumer-1.0");
  let path_id = add_store_path(&mut state, "source-tarball-1.0");
  state
    .get_derivation_info_mut(consumer_id)
    .unwrap()
    .input_sources
    .insert(path_id);
  state
    .get_store_path_info_mut(path_id)
    .unwrap()
    .input_for
    .insert(consumer_id);
  state.update_build_status(consumer_id, BuildStatus::Planned);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(consumer_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  let consumer_row = row_text(
    &terminal,
    row_containing(&terminal, "consumer-1.0").unwrap(),
  );
  assert!(
    consumer_row.contains("↓"),
    "input source transfer should attach to its exact consumer: {rendered}"
  );
  assert!(
    !rendered.contains("source-tarball-1.0"),
    "attached input source must not also render as an orphan: {rendered}"
  );
}

#[test]
fn tui_renders_unmatched_downloads_as_standalone_activity_rows() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let path_id = add_store_path(&mut state, "source-tarball-1.0");

  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "source-tarball-1.0").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "substitute-only downloads should not disappear when no derivation node \
     is known yet: {download_row:?}"
  );
}

#[test]
fn tui_renders_unmatched_downloads_from_render_snapshot() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let path_id = add_store_path(&mut state, "source-tarball-1.0");

  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });

  let snapshot = state.render_snapshot();
  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &snapshot, &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("source-tarball-1.0"),
    "render snapshots should retain active download path names: {rendered}"
  );
}

#[test]
fn tui_keeps_unrendered_downloads_visible_when_tree_is_truncated() {
  let backend = TestBackend::new(80, 18);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let middle_id = add_derivation(&mut state, "middle-1.0");
  let active_id = add_derivation(&mut state, "active-leaf-1.0");
  let path_id = add_store_path(&mut state, "electron-unwrapped-41.7.1");

  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: middle_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(middle_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: active_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(middle_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);
  state
    .get_derivation_info_mut(active_id)
    .unwrap()
    .derivation_parents
    .insert(middle_id);

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(middle_id, BuildStatus::Planned);
  state.update_build_status(
    active_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time() - 2.0,
      host:        cognos::Host::Localhost,
      activity_id: Some(7),
    }),
  );
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("electron-unwrapped-41.7.1"),
    "active downloads should keep a visible row even when the dependency tree \
     is taller than the graph pane: {rendered}"
  );
}

#[test]
fn tui_falls_back_when_inline_download_row_is_truncated() {
  let backend = TestBackend::new(80, 18);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let parent_id = add_derivation(&mut state, "parent-1.0");
  let middle_id = add_derivation(&mut state, "middle-1.0");
  let downloaded_id = add_derivation(&mut state, "electron-unwrapped-41.7.1");
  let path_id =
    add_output_path(&mut state, downloaded_id, "electron-unwrapped-41.7.1");

  for (parent, child) in [
    (root_id, parent_id),
    (parent_id, middle_id),
    (middle_id, downloaded_id),
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

  for drv_id in [root_id, parent_id, middle_id, downloaded_id] {
    state.update_build_status(drv_id, BuildStatus::Planned);
  }
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "electron-unwrapped-41.7.1").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "downloads whose inline tree row was truncated should get a fallback row: \
     {download_row:?}"
  );
}

#[test]
fn tui_renders_name_matched_downloads_when_derivation_is_not_in_tree() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let _orphan_id = add_derivation(&mut state, "electron-unwrapped-41.7.1");
  let path_id = add_store_path(&mut state, "electron-unwrapped-41.7.1");

  state.update_build_status(root_id, BuildStatus::Planned);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              cognos::Host::Localhost,
      activity_id:       42,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "electron-unwrapped-41.7.1").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "name-matched downloads should fall back to a standalone row when their \
     derivation node was not rendered: {download_row:?}"
  );
}

#[test]
fn tui_caps_standalone_downloads_so_dependency_tree_stays_visible() {
  let backend = TestBackend::new(100, 32);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let middle_id = add_derivation(&mut state, "middle-1.0");
  let active_id = add_derivation(&mut state, "active-leaf-1.0");

  for (parent, child) in [(root_id, middle_id), (middle_id, active_id)] {
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

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(middle_id, BuildStatus::Planned);
  state.update_build_status(
    active_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time() - 2.0,
      host:        cognos::Host::Localhost,
      activity_id: Some(7),
    }),
  );
  state.forest_roots.push(root_id);

  for index in 0..24 {
    let path_id = add_store_path(&mut state, &format!("download-only-{index}"));
    state
      .full_summary
      .running_downloads
      .insert(path_id, TransferInfo {
        start:             current_time() - 2.0,
        host:              cognos::Host::Localhost,
        activity_id:       100 + index,
        bytes_transferred: 512,
        total_bytes:       Some(1024),
      });
  }

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let rendered = format!("{}", terminal.backend());
  assert!(
    rendered.contains("active-leaf-1.0")
      && rendered.contains("middle-1.0")
      && rendered.contains("root-1.0"),
    "standalone downloads should not crowd out the dependency tree: {rendered}"
  );

  let download_rows = (0..terminal.backend().buffer().height())
    .filter(|row| row_text(&terminal, *row).contains("download-only-"))
    .count();
  assert_eq!(
    download_rows, 24,
    "active orphan downloads are mandatory beyond the soft budget: {rendered}"
  );
}

#[test]
fn tui_renders_planned_downloads_inline_in_dependency_graph() {
  let backend = TestBackend::new(80, 24);
  let mut terminal = Terminal::new(backend).unwrap();
  let mut state = State::new();
  let root_id = add_derivation(&mut state, "root-1.0");
  let downloaded_id = add_derivation(&mut state, "signal-desktop-8.9.1");
  let path_id =
    add_output_path(&mut state, downloaded_id, "signal-desktop-8.9.1");

  state
    .get_derivation_info_mut(root_id)
    .unwrap()
    .input_derivations
    .push(InputDerivation {
      derivation: downloaded_id,
      outputs:    HashSet::new(),
    });
  state
    .get_derivation_info_mut(downloaded_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);

  state.update_build_status(root_id, BuildStatus::Planned);
  state.update_build_status(downloaded_id, BuildStatus::Planned);
  state.full_summary.planned_downloads.insert(path_id);
  state.forest_roots.push(root_id);

  let config = tui_config();
  terminal
    .draw(|frame| draw(frame, &state.render_snapshot(), &config))
    .unwrap();

  let download_row = row_text(
    &terminal,
    row_containing(&terminal, "signal-desktop-8.9.1").unwrap(),
  );
  assert!(
    download_row.contains("↓"),
    "planned substitute should render as an inline download row: \
     {download_row:?}"
  );
}
