use crate::state::{
  BuildStatus,
  CompletedTransferInfo,
  DerivationId,
  State,
  StorePathId,
  current_time,
};

fn build_sort_order(state: &State, drv_id: DerivationId) -> (u8, i64) {
  let Some(info) = state.get_derivation_info(drv_id) else {
    return (9, 0);
  };
  match &info.build_status {
    BuildStatus::Failed { fail, .. } => {
      return (0, (fail.at * 1_000_000.0) as i64);
    },
    BuildStatus::Building(build_info) => {
      return (1, (build_info.start * 1_000_000.0) as i64);
    },
    BuildStatus::Planned | BuildStatus::Built { .. } | BuildStatus::Unknown => {
    },
  }

  let mut outputs = info.outputs.values();
  if let Some(transfer) = outputs
    .clone()
    .filter_map(|path| state.full_summary.running_downloads.get(path))
    .min_by_key(|transfer| (transfer.start * 1_000_000.0) as i64)
  {
    return (2, (transfer.start * 1_000_000.0) as i64);
  }
  if let Some(transfer) = outputs
    .clone()
    .filter_map(|path| state.full_summary.running_uploads.get(path))
    .min_by_key(|transfer| (transfer.start * 1_000_000.0) as i64)
  {
    return (3, (transfer.start * 1_000_000.0) as i64);
  }
  match &info.build_status {
    BuildStatus::Planned => return (4, 0),
    BuildStatus::Built { end, .. } => {
      return (6, -(*end * 1_000_000.0) as i64);
    },
    BuildStatus::Unknown
    | BuildStatus::Building(_)
    | BuildStatus::Failed { .. } => {},
  }
  if outputs.any(|path| state.full_summary.planned_downloads.contains(path)) {
    return (5, 0);
  }

  (9, 0)
}

fn subtree_sort_order(state: &State, drv_id: DerivationId) -> (u8, i64) {
  let Some(info) = state.get_derivation_info(drv_id) else {
    return (9, 0);
  };
  let summary = &info.dependency_summary;

  if let Some(fail) = summary
    .failed_builds
    .values()
    .min_by_key(|fail| (fail.end * 1_000_000.0) as i64)
  {
    return (0, (fail.end * 1_000_000.0) as i64);
  }

  if let Some(build) = summary
    .running_builds
    .values()
    .min_by_key(|build| (build.start * 1_000_000.0) as i64)
  {
    return (1, (build.start * 1_000_000.0) as i64);
  }
  if let Some(transfer) = summary
    .running_downloads
    .values()
    .min_by_key(|transfer| (transfer.start * 1_000_000.0) as i64)
  {
    return (2, (transfer.start * 1_000_000.0) as i64);
  }
  if let Some(transfer) = summary
    .running_uploads
    .values()
    .min_by_key(|transfer| (transfer.start * 1_000_000.0) as i64)
  {
    return (3, (transfer.start * 1_000_000.0) as i64);
  }
  if !summary.planned_builds.is_empty() {
    return (4, 0);
  }
  if !summary.planned_downloads.is_empty() {
    return (5, 0);
  }
  if !summary.completed_builds.is_empty() {
    return (6, 0);
  }

  build_sort_order(state, drv_id)
}

pub(crate) type BuildSortKey = (u8, i64, u8, i64, DerivationId);

pub(crate) fn sort_key(state: &State, drv_id: DerivationId) -> BuildSortKey {
  let (own_a, own_b) = build_sort_order(state, drv_id);
  let (sub_a, sub_b) = subtree_sort_order(state, drv_id);

  (own_a, own_b, sub_a, sub_b, drv_id)
}

pub fn detect_local_completed_builds(state: &mut State, now: f64) -> bool {
  let local_building: Vec<DerivationId> = state
    .full_summary
    .running_builds
    .iter()
    .filter(|(_, info)| info.host == cognos::Host::Localhost)
    .map(|(id, _)| *id)
    .collect();

  let mut any_completed = false;

  for drv_id in local_building {
    let output_paths: Vec<std::path::PathBuf> = state
      .get_derivation_info(drv_id)
      .map(|info| {
        info
          .outputs
          .values()
          .filter_map(|&sp_id| {
            state
              .get_store_path_info(sp_id)
              .map(|sp_info| sp_info.name.path.clone())
          })
          .collect()
      })
      .unwrap_or_default();

    let all_exist =
      !output_paths.is_empty() && output_paths.iter().all(|p| p.exists());
    if all_exist {
      let build_info = state.get_derivation_info(drv_id).and_then(|info| {
        if let BuildStatus::Building(build) = &info.build_status {
          Some(build.clone())
        } else {
          None
        }
      });

      if let Some(build_info) = build_info {
        state.update_build_status(drv_id, BuildStatus::Built {
          info: build_info,
          end:  now,
        });
        any_completed = true;
      }
    }
  }

  any_completed
}

fn complete_build_success(state: &mut State, drv_id: DerivationId, now: f64) {
  let build_info = state.get_derivation_info(drv_id).and_then(|info| {
    if let BuildStatus::Building(build_info) = &info.build_status {
      Some(build_info.clone())
    } else {
      None
    }
  });

  if let Some(build_info) = build_info {
    state.update_build_status(drv_id, BuildStatus::Built {
      info: build_info,
      end:  now,
    });
  }
}

pub fn finish_state(state: &mut State) {
  let building: Vec<DerivationId> = state
    .derivation_infos
    .iter()
    .filter_map(|(drv_id, info)| {
      if matches!(info.build_status, BuildStatus::Building(_)) {
        Some(*drv_id)
      } else {
        None
      }
    })
    .collect();

  for drv_id in building {
    complete_build_success(state, drv_id, current_time());
  }

  let downloading: Vec<StorePathId> = state
    .full_summary
    .running_downloads
    .keys()
    .copied()
    .collect();
  for path_id in downloading {
    if let Some(transfer) =
      state.full_summary.running_downloads.remove(&path_id)
    {
      state.full_summary.completed_downloads.insert(
        path_id,
        CompletedTransferInfo {
          start:       transfer.start,
          end:         current_time(),
          host:        transfer.host,
          total_bytes: transfer.total_bytes.unwrap_or(0),
        },
      );
      state.refresh_store_path_summary(path_id);
    }
  }

  let uploading: Vec<StorePathId> =
    state.full_summary.running_uploads.keys().copied().collect();
  for path_id in uploading {
    if let Some(transfer) = state.full_summary.running_uploads.remove(&path_id)
    {
      state.full_summary.completed_uploads.insert(
        path_id,
        CompletedTransferInfo {
          start:       transfer.start,
          end:         current_time(),
          host:        transfer.host,
          total_bytes: transfer.total_bytes.unwrap_or(0),
        },
      );
      state.refresh_store_path_summary(path_id);
    }
  }
}
