use std::{io::Cursor, str::FromStr};

use cognos::Host;
use rom_core::{
  Config,
  Monitor,
  MonitorOptions,
  PresentationStyle,
  RenderOptions,
  console::{ConsoleConfig, write_final_graph_with_options},
  create_monitor_with_options,
  monitor_stream_with_options,
  state::{
    BuildFail,
    BuildInfo,
    BuildStatus,
    CompletedTransferInfo,
    Derivation,
    FailType,
    State,
    StorePath,
    TransferInfo,
    current_time,
  },
  tui::{
    Color,
    TuiConfig,
    minimum_required_graph_rows_at_width_with_options,
    render_final_graph_screen,
    render_final_graph_screen_with_options,
    render_graph_screen_with_options,
  },
};

#[test]
fn presentation_style_parser_is_strict_and_supports_documented_aliases() {
  let canonical = [
    ("connected", PresentationStyle::Connected),
    ("compact", PresentationStyle::Compact),
    ("verbose", PresentationStyle::Verbose),
    ("plain", PresentationStyle::Plain),
    ("dashboard", PresentationStyle::Dashboard),
    ("table-summary", PresentationStyle::TableSummary),
    ("full-summary", PresentationStyle::FullSummary),
  ];
  for (name, style) in canonical {
    assert_eq!(PresentationStyle::from_str(name).unwrap(), style);
    assert_eq!(style.to_string(), name);
  }

  assert_eq!("tree".parse(), Ok(PresentationStyle::Connected));
  assert_eq!("table".parse(), Ok(PresentationStyle::TableSummary));
  assert_eq!("full".parse(), Ok(PresentationStyle::FullSummary));
  assert!(PresentationStyle::from_str("CONNECTED").is_err());
  assert!(PresentationStyle::from_str("unknown").is_err());
}

#[test]
fn connected_is_the_default_at_every_options_layer() {
  assert_eq!(PresentationStyle::default(), PresentationStyle::Connected);
  assert_eq!(RenderOptions::default().style, PresentationStyle::Connected);
  assert_eq!(
    MonitorOptions::default().render.style,
    PresentationStyle::Connected
  );
}

#[test]
fn legacy_and_explicit_connected_screens_are_identical() {
  let state = State::new();
  let config = TuiConfig::default();
  let legacy = render_final_graph_screen(100, &state, &config);
  let explicit = render_final_graph_screen_with_options(
    100,
    &state,
    &config,
    RenderOptions::default(),
  );
  assert_eq!(legacy, explicit);
}

fn styled_state() -> State {
  let mut state = State::new();
  state.start_time = current_time() - 4.0;
  let derivation = Derivation::parse(
    "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hello-1.0.drv",
  )
  .unwrap();
  let drv_id = state.get_or_create_derivation_id(derivation);
  state.update_build_status(
    drv_id,
    BuildStatus::Building(BuildInfo {
      start:       current_time() - 2.0,
      host:        Host::Localhost,
      activity_id: None,
    }),
  );
  state.forest_roots.push(drv_id);
  let path =
    StorePath::parse("/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-hello-1.0")
      .unwrap();
  let path_id = state.get_or_create_store_path_id(path);
  state.get_store_path_info_mut(path_id).unwrap().producer = Some(drv_id);
  state
    .full_summary
    .running_downloads
    .insert(path_id, TransferInfo {
      start:             current_time() - 2.0,
      host:              Host::Remote("https://cache.nixos.org".to_string()),
      activity_id:       7,
      bytes_transferred: 512,
      total_bytes:       Some(1024),
    });
  state
}

fn render(style: PresentationStyle, width: u16) -> rom_core::tui::Screen {
  let state = styled_state();
  render_graph_screen_with_options(
    width,
    20,
    &state.render_snapshot(),
    &TuiConfig::default(),
    style.into(),
  )
}

#[test]
fn compact_keeps_graph_and_replaces_table_with_one_nom_summary_line() {
  for width in [32, 100] {
    let screen = render(PresentationStyle::Compact, width);
    let text = screen.plain_text();
    assert!(text.lines().next().unwrap().starts_with("├─"), "{text}");
    if width >= 100 {
      assert!(text.contains("hello-1.0"), "{text}");
    }
    assert!(text.contains("builds"), "{text}");
    assert!(!text.contains("BUILDS"), "{text}");
    assert!(!text.contains("HOSTS"), "{text}");
    assert_eq!(screen.height(), 2, "{text}");
    assert!(screen.row_text(1).unwrap().starts_with("└─"), "{text}");
  }
}

#[test]
fn verbose_footer_is_readable_wide_and_narrow_without_repeating_activity() {
  for width in [32, 100] {
    let screen = render(PresentationStyle::Verbose, width);
    let text = screen.plain_text();
    for label in ["Builds", "Transfers", "Hosts", "Elapsed"] {
      assert!(text.contains(label), "missing {label} at {width}: {text}");
    }
    if width >= 100 {
      assert_eq!(text.matches("hello-1.0").count(), 1, "{text}");
    }
    assert_eq!(screen.height(), 6, "{text}");
  }
  let wide = render(PresentationStyle::Verbose, 100).plain_text();
  assert!(wide.contains("cache.nixos.org"), "{wide}");
  assert!(wide.contains("pull 0/1 · 50% 512B/1.0K"), "{wide}");
  assert!(wide.contains("active"), "{wide}");
}

#[test]
fn preset_resize_sizing_accounts_for_each_footer_and_mandatory_activity() {
  let snapshot = styled_state().render_snapshot();
  assert_eq!(
    minimum_required_graph_rows_at_width_with_options(
      32,
      &snapshot,
      PresentationStyle::Compact.into(),
    ),
    2
  );
  assert_eq!(
    minimum_required_graph_rows_at_width_with_options(
      32,
      &snapshot,
      PresentationStyle::Verbose.into(),
    ),
    6
  );
  for style in [PresentationStyle::Compact, PresentationStyle::Verbose] {
    let minimum = minimum_required_graph_rows_at_width_with_options(
      32,
      &snapshot,
      style.into(),
    ) as u16;
    let screen = render_graph_screen_with_options(
      32,
      minimum,
      &snapshot,
      &TuiConfig::default(),
      style.into(),
    );
    assert!(
      screen
        .plain_text()
        .lines()
        .next()
        .unwrap()
        .starts_with("├─")
    );
  }
}

#[test]
fn preset_screens_keep_cobalt_and_copper_colors() {
  for style in [PresentationStyle::Compact, PresentationStyle::Verbose] {
    let screen = render(style, 100);
    let colors = screen
      .cells()
      .iter()
      .filter_map(|cell| cell.style.foreground)
      .collect::<Vec<_>>();
    assert!(colors.contains(&Color::Rgb {
      r: 75,
      g: 88,
      b: 112,
    }));
    assert!(colors.contains(&Color::Rgb {
      r: 109,
      g: 145,
      b: 229,
    }));
  }
}

#[test]
fn plain_is_flat_prioritized_and_compact() {
  for width in [18, 100] {
    let screen = render(PresentationStyle::Plain, width);
    let text = screen.plain_text();
    assert_eq!(screen.height(), 3, "{text}");
    assert!(text.contains("hello"), "{text}");
    assert!(text.contains("builds"), "{text}");
    assert!(!text.contains('├'), "{text}");
    assert!(!text.contains('│'), "{text}");
    assert!(!text.contains("BUILDS"), "{text}");
  }
}

#[test]
fn plain_retains_all_active_rows_before_summary_under_height_pressure() {
  let state = styled_state();
  let screen = render_graph_screen_with_options(
    100,
    2,
    &state.render_snapshot(),
    &TuiConfig::default(),
    PresentationStyle::Plain.into(),
  );
  let text = screen.plain_text();
  assert_eq!(screen.height(), 2, "{text}");
  assert_eq!(text.matches("hello-1.0").count(), 2, "{text}");
  assert!(!text.contains("builds ·"), "{text}");
  assert_eq!(
    minimum_required_graph_rows_at_width_with_options(
      100,
      &state.render_snapshot(),
      PresentationStyle::Plain.into(),
    ),
    3
  );
}

#[test]
fn dashboard_is_a_content_sized_resize_safe_operations_panel() {
  for width in [12, 100] {
    let screen = render(PresentationStyle::Dashboard, width);
    let text = screen.plain_text();
    assert_eq!(screen.height(), 6, "{text}");
    for label in ["Root", "Builds", "Transfers", "Host", "Status", "Duration"] {
      assert!(text.contains(label), "missing {label} at {width}: {text}");
    }
    assert!(screen.cells().len() <= usize::from(width) * 6);
  }

  let snapshot = styled_state().render_snapshot();
  assert_eq!(
    minimum_required_graph_rows_at_width_with_options(
      12,
      &snapshot,
      PresentationStyle::Dashboard.into(),
    ),
    6
  );
  let pressured = render_graph_screen_with_options(
    12,
    3,
    &snapshot,
    &TuiConfig::default(),
    PresentationStyle::Dashboard.into(),
  );
  assert_eq!(pressured.height(), 3);
  assert!(pressured.plain_text().contains("Transfers"));
}

#[test]
fn plain_and_dashboard_use_semantic_colors() {
  for style in [PresentationStyle::Plain, PresentationStyle::Dashboard] {
    let colors = render(style, 100)
      .cells()
      .iter()
      .filter_map(|cell| cell.style.foreground)
      .collect::<Vec<_>>();
    assert!(colors.contains(&Color::Rgb {
      r: 109,
      g: 145,
      b: 229,
    }));
    assert!(colors.contains(&Color::Rgb {
      r: 85,
      g: 180,
      b: 204,
    }));
  }
}

#[test]
fn plain_and_dashboard_live_and_final_rendering_match() {
  let state = styled_state();
  for style in [PresentationStyle::Plain, PresentationStyle::Dashboard] {
    let live = render_graph_screen_with_options(
      100,
      20,
      &state.render_snapshot(),
      &TuiConfig::default(),
      style.into(),
    );
    let final_screen = render_final_graph_screen_with_options(
      100,
      &state,
      &TuiConfig::default(),
      style.into(),
    );
    assert_eq!(live.plain_text(), final_screen.plain_text());
  }
}

#[test]
fn plain_and_dashboard_idle_states_remain_elapsed_only() {
  for style in [PresentationStyle::Plain, PresentationStyle::Dashboard] {
    let screen = render_graph_screen_with_options(
      8,
      20,
      &State::new().render_snapshot(),
      &TuiConfig::default(),
      style.into(),
    );
    assert_eq!(screen.height(), 1);
    assert!(!screen.plain_text().contains("Builds"));
  }
}

#[test]
fn preset_final_output_honors_no_color() {
  for style in [
    PresentationStyle::Compact,
    PresentationStyle::Verbose,
    PresentationStyle::Plain,
    PresentationStyle::Dashboard,
  ] {
    let mut output = Vec::new();
    write_final_graph_with_options(
      &mut output,
      &styled_state(),
      ConsoleConfig {
        use_color: false,
        width: 100,
        ..ConsoleConfig::default()
      },
      style.into(),
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("\x1b["), "{output:?}");
    assert!(output.contains("hello-1.0"), "{output}");
  }
}

#[test]
fn preset_live_and_final_screens_have_footer_parity() {
  let state = styled_state();
  for style in [PresentationStyle::Compact, PresentationStyle::Verbose] {
    let live = render_graph_screen_with_options(
      100,
      20,
      &state.render_snapshot(),
      &TuiConfig::default(),
      style.into(),
    );
    let final_screen = render_final_graph_screen_with_options(
      100,
      &state,
      &TuiConfig::default(),
      style.into(),
    );
    assert_eq!(live.plain_text(), final_screen.plain_text());
  }
}

fn final_summary_state() -> State {
  let mut state = State::new();
  let now = current_time();
  state.start_time = now - 8.0;

  for (index, (host, status)) in [
    (Host::Localhost, true),
    (Host::Remote("ssh://builder.example".to_string()), false),
  ]
  .into_iter()
  .enumerate()
  {
    let derivation = Derivation::parse(&format!(
      "/nix/store/{:0<32}-summary-{index}.drv",
      index
    ))
    .unwrap();
    let id = state.get_or_create_derivation_id(derivation);
    let info = BuildInfo {
      start: now - 6.0,
      host,
      activity_id: None,
    };
    state.update_build_status(
      id,
      if status {
        BuildStatus::Built {
          info,
          end: now - 2.0,
        }
      } else {
        BuildStatus::Failed {
          info,
          fail: BuildFail {
            at:        now - 1.0,
            fail_type: FailType::BuildFailed(1),
          },
        }
      },
    );
  }

  for (index, upload) in [false, true].into_iter().enumerate() {
    let path =
      StorePath::parse(&format!("/nix/store/{:1<32}-transfer-{index}", index))
        .unwrap();
    let id = state.get_or_create_store_path_id(path);
    let transfer = CompletedTransferInfo {
      start:       now - 4.0,
      end:         now - 2.0,
      host:        if upload {
        Host::Localhost
      } else {
        Host::Remote("https://cache.example/nar".to_string())
      },
      total_bytes: if upload { 2_048 } else { 1_024 },
    };
    if upload {
      state.full_summary.completed_uploads.insert(id, transfer);
    } else {
      state.full_summary.completed_downloads.insert(id, transfer);
    }
  }
  state
}

#[test]
fn final_summary_presets_use_connected_for_live_presentation() {
  let state = styled_state();
  let snapshot = state.render_snapshot();
  let connected = render_graph_screen_with_options(
    100,
    20,
    &snapshot,
    &TuiConfig::default(),
    PresentationStyle::Connected.into(),
  );
  for style in [
    PresentationStyle::TableSummary,
    PresentationStyle::FullSummary,
  ] {
    let summary_live = render_graph_screen_with_options(
      100,
      20,
      &snapshot,
      &TuiConfig::default(),
      style.into(),
    );
    assert_eq!(summary_live, connected);
  }
}

#[test]
fn table_summary_is_content_sized_and_groups_final_work_by_host() {
  let state = final_summary_state();
  let screen = render_final_graph_screen_with_options(
    100,
    &state,
    &TuiConfig::default(),
    PresentationStyle::TableSummary.into(),
  );
  let text = screen.plain_text();
  for expected in [
    "HOST",
    "BUILT",
    "FAILED",
    "DOWNLOADED",
    "UPLOADED",
    "builder.example",
    "cache.example",
    "localhost",
    "Total",
    "Outcome",
    "Time",
  ] {
    assert!(text.contains(expected), "missing {expected}: {text}");
  }
  assert!(screen.height() < 20, "{text}");
}

#[test]
fn full_summary_writer_supports_color_and_plain_generic_writers() {
  let state = final_summary_state();
  for use_color in [false, true] {
    let mut writer = Cursor::new(Vec::new());
    write_final_graph_with_options(
      &mut writer,
      &state,
      ConsoleConfig {
        use_color,
        width: 100,
        ..ConsoleConfig::default()
      },
      PresentationStyle::FullSummary.into(),
    )
    .unwrap();
    let output = String::from_utf8(writer.into_inner()).unwrap();
    for label in [
      "Built",
      "Failed",
      "Downloaded",
      "Uploaded",
      "Nix errors",
      "Outcome",
      "Time",
    ] {
      assert!(output.contains(label), "missing {label}: {output}");
    }
    assert_eq!(output.contains("\x1b["), use_color, "{output:?}");
    assert!(!output.contains("Finished at"), "{output}");
  }
}

#[test]
fn full_summary_reports_structured_and_process_failures() {
  let mut state = final_summary_state();
  state.nix_errors.push("evaluation failed".to_string());
  let screen = render_final_graph_screen_with_options(
    100,
    &state,
    &TuiConfig {
      console: ConsoleConfig {
        process_exit_code: Some(17),
        ..ConsoleConfig::default()
      },
    },
    PresentationStyle::FullSummary.into(),
  );
  let text = screen.plain_text();
  assert!(text.contains("1 builds"), "{text}");
  assert!(text.contains("1 nix error"), "{text}");
  assert!(text.contains("evaluator status 17"), "{text}");
  assert!(text.contains("failed"), "{text}");
}

#[test]
fn connected_final_writer_remains_concise() {
  let mut output = Vec::new();
  write_final_graph_with_options(
    &mut output,
    &State::new(),
    ConsoleConfig {
      use_color: false,
      ..ConsoleConfig::default()
    },
    PresentationStyle::Connected.into(),
  )
  .unwrap();
  let output = String::from_utf8(output).unwrap();
  assert!(output.contains("Finished at"), "{output}");
  assert!(!output.contains("Outcome"), "{output}");
}

#[test]
fn monitor_constructor_overloads_accept_options() {
  let options = MonitorOptions::from(PresentationStyle::Compact);
  let direct =
    Monitor::new_with_options(Config::default(), options, Vec::new());
  assert!(direct.is_ok());
  let helper =
    create_monitor_with_options(Config::default(), options, Vec::new());
  assert!(helper.is_ok());

  let mut output = Vec::new();
  let streamed = monitor_stream_with_options(
    Config::default(),
    options,
    Cursor::new(""),
    &mut output,
  );
  assert!(streamed.is_ok());
  assert!(String::from_utf8(output).unwrap().contains("Finished at"));
}
