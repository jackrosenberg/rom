use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use cognos::{Host, OutputName};
use indexmap::IndexMap;

use super::{
  ActivityId,
  ActivityStatus,
  BuildStatus,
  CompletedTransferInfo,
  DependencySummary,
  Derivation,
  DerivationId,
  EvalInfo,
  InputDerivation,
  State,
  StorePathId,
  StorePathInfo,
};

const MAX_RENDER_SNAPSHOT_ROOTS: usize = 256;
const MAX_RENDER_SNAPSHOT_DERIVATIONS: usize = 2_048;

/// Lightweight derivation projection used by live rendering.
#[derive(Debug, Clone)]
pub struct RenderDerivationInfo {
  pub name:               Derivation,
  pub platform:           Option<String>,
  pub input_derivations:  Vec<InputDerivation>,
  pub derivation_parents: HashSet<DerivationId>,
  pub outputs:            HashMap<OutputName, StorePathId>,
  pub input_sources:      HashSet<StorePathId>,
  pub build_status:       BuildStatus,
  pub dependency_built:   usize,
  pub dependency_waiting: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderTransferHostSummary {
  pub(crate) host:        Host,
  pub(crate) completed:   usize,
  pub(crate) total_bytes: u64,
}

/// Pruned, read-only state model for live rendering.
///
/// This intentionally does not carry mutation indexes, caches, traces, or
/// final-error diagnostics. Code that needs the full build state should keep
/// using [`State`].
#[derive(Debug, Clone)]
pub struct RenderSnapshot {
  pub derivation_infos: IndexMap<DerivationId, RenderDerivationInfo>,
  pub store_path_infos: IndexMap<StorePathId, StorePathInfo>,
  pub full_summary:     DependencySummary,
  pub forest_roots:     Vec<DerivationId>,
  pub total_root_count: usize,
  pub start_time:       f64,
  pub activities:       HashMap<ActivityId, ActivityStatus>,
  pub evaluation_state: EvalInfo,

  derivation_name_index:               HashMap<String, HashSet<DerivationId>>,
  sort_keys: HashMap<DerivationId, crate::update::BuildSortKey>,
  dependency_owners:                   HashMap<DerivationId, DerivationId>,
  pub(crate) completed_download_hosts: Vec<RenderTransferHostSummary>,
  pub(crate) completed_upload_hosts:   Vec<RenderTransferHostSummary>,
}

impl State {
  #[must_use]
  pub fn render_snapshot(&self) -> RenderSnapshot {
    let focus_ids = self.render_focus_derivations();
    let forest_roots = self.render_forest_roots(&focus_ids);
    let derivation_infos =
      self.render_derivation_infos(&focus_ids, &forest_roots);
    let derivation_name_index = derivation_name_index(&derivation_infos);
    let dependency_owners = dependency_owners(&derivation_infos);
    let sort_keys = derivation_infos
      .keys()
      .map(|id| (*id, crate::update::sort_key(self, *id)))
      .collect();
    let activities = self.render_activities(&derivation_infos);

    RenderSnapshot {
      derivation_infos,
      store_path_infos: self.render_store_path_infos(),
      full_summary: self.render_dependency_summary(),
      forest_roots,
      total_root_count: self.render_root_count(),
      start_time: self.start_time,
      derivation_name_index,
      sort_keys,
      dependency_owners,
      activities,
      evaluation_state: self.evaluation_state.clone(),
      completed_download_hosts: render_completed_transfer_hosts(
        self.full_summary.completed_downloads.values(),
      ),
      completed_upload_hosts: render_completed_transfer_hosts(
        self.full_summary.completed_uploads.values(),
      ),
    }
  }

  fn render_root_count(&self) -> usize {
    if !self.forest_roots.is_empty() {
      return self.forest_roots.len();
    }
    let mut roots = HashSet::new();
    roots.extend(self.full_summary.failed_builds.keys().copied());
    roots.extend(self.full_summary.running_builds.keys().copied());
    roots.extend(self.full_summary.planned_builds.iter().copied());
    roots.len()
  }

  fn render_dependency_summary(&self) -> DependencySummary {
    DependencySummary {
      planned_builds:      self.full_summary.planned_builds.clone(),
      running_builds:      self.full_summary.running_builds.clone(),
      completed_builds:    self.full_summary.completed_builds.clone(),
      failed_builds:       self.full_summary.failed_builds.clone(),
      planned_downloads:   self.full_summary.planned_downloads.clone(),
      completed_downloads: HashMap::new(),
      completed_uploads:   HashMap::new(),
      running_downloads:   self.full_summary.running_downloads.clone(),
      running_uploads:     self.full_summary.running_uploads.clone(),
    }
  }

  fn render_focus_derivations(&self) -> HashSet<DerivationId> {
    let mut focus = HashSet::new();

    focus.extend(self.full_summary.failed_builds.keys().copied());
    focus.extend(self.full_summary.running_builds.keys().copied());

    for path_id in self
      .full_summary
      .running_downloads
      .keys()
      .chain(self.full_summary.running_uploads.keys())
      .chain(self.full_summary.planned_downloads.iter())
    {
      if let Some(path) = self.store_path_infos.get(path_id) {
        focus.extend(path.producer);
        focus.extend(path.input_for.iter().copied());
      }
    }

    let mut stack = focus.iter().copied().collect::<Vec<_>>();
    while let Some(drv_id) = stack.pop() {
      let Some(info) = self.derivation_infos.get(&drv_id) else {
        continue;
      };
      for parent_id in &info.derivation_parents {
        if focus.insert(*parent_id) {
          stack.push(*parent_id);
        }
      }
    }

    focus
  }

  fn render_forest_roots(
    &self,
    focus_ids: &HashSet<DerivationId>,
  ) -> Vec<DerivationId> {
    let mut roots = if self.forest_roots.is_empty() {
      let mut roots = Vec::new();
      roots.extend(self.full_summary.failed_builds.keys().copied());
      roots.extend(self.full_summary.running_builds.keys().copied());
      roots.extend(self.full_summary.planned_builds.iter().copied());
      roots
    } else {
      self.forest_roots.clone()
    };

    let mut disconnected_focus_roots = focus_ids
      .iter()
      .copied()
      .filter(|id| {
        self
          .derivation_infos
          .get(id)
          .is_some_and(|info| info.derivation_parents.is_empty())
      })
      .collect::<Vec<_>>();
    disconnected_focus_roots.sort_unstable();
    roots.extend(disconnected_focus_roots);
    dedup_derivation_ids(&mut roots);
    roots.sort_by_key(|id| crate::update::sort_key(self, *id));
    if roots.len() <= MAX_RENDER_SNAPSHOT_ROOTS {
      return roots;
    }

    let mut selected = roots
      .iter()
      .filter(|id| focus_ids.contains(id))
      .copied()
      .collect::<Vec<_>>();

    let remaining = MAX_RENDER_SNAPSHOT_ROOTS.saturating_sub(selected.len());
    if remaining > 0 {
      let front_len = remaining.div_ceil(2).min(roots.len());
      let tail_len = remaining.saturating_sub(front_len);
      let tail_start = roots.len().saturating_sub(tail_len);
      selected.extend(roots.iter().take(front_len).copied());
      selected.extend(roots.iter().skip(tail_start).copied());
    }

    dedup_derivation_ids(&mut selected);
    selected
  }

  fn render_derivation_infos(
    &self,
    focus_ids: &HashSet<DerivationId>,
    forest_roots: &[DerivationId],
  ) -> IndexMap<DerivationId, RenderDerivationInfo> {
    let mut render_ids = focus_ids.clone();
    render_ids.extend(forest_roots.iter().copied());

    // Keep every mandatory path, then expand optional, state-sorted branches
    // breadth-first up to the normal snapshot cap.
    let mut snapshot_ids = render_ids.clone();
    let mut queued = snapshot_ids.clone();
    let mut candidates = VecDeque::new();
    for drv_id in &render_ids {
      self.queue_render_children(*drv_id, &mut queued, &mut candidates);
    }

    while snapshot_ids.len() < MAX_RENDER_SNAPSHOT_DERIVATIONS {
      let Some(drv_id) = candidates.pop_front() else {
        break;
      };
      snapshot_ids.insert(drv_id);
      self.queue_render_children(drv_id, &mut queued, &mut candidates);
    }

    let mut ids = snapshot_ids.iter().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    ids
      .into_iter()
      .filter_map(|drv_id| {
        let info = self.derivation_infos.get(&drv_id)?;
        let mut input_derivations = info
          .input_derivations
          .iter()
          .filter(|input| snapshot_ids.contains(&input.derivation))
          .cloned()
          .collect::<Vec<_>>();
        input_derivations
          .sort_by_key(|input| crate::update::sort_key(self, input.derivation));
        Some((drv_id, RenderDerivationInfo {
          name: info.name.clone(),
          platform: info.platform.clone(),
          input_derivations,
          derivation_parents: info
            .derivation_parents
            .iter()
            .filter(|parent_id| snapshot_ids.contains(parent_id))
            .copied()
            .collect(),
          outputs: info.outputs.clone(),
          input_sources: info.input_sources.clone(),
          build_status: info.build_status.clone(),
          dependency_built: info.dependency_summary.completed_builds.len(),
          dependency_waiting: info.dependency_summary.planned_builds.len(),
        }))
      })
      .collect()
  }

  fn queue_render_children(
    &self,
    drv_id: DerivationId,
    queued: &mut HashSet<DerivationId>,
    candidates: &mut VecDeque<DerivationId>,
  ) {
    let Some(info) = self.derivation_infos.get(&drv_id) else {
      return;
    };
    for child_id in info.input_derivations.iter().map(|input| input.derivation)
    {
      if queued.insert(child_id) {
        candidates.push_back(child_id);
      }
    }
  }

  fn render_activities(
    &self,
    derivation_infos: &IndexMap<DerivationId, RenderDerivationInfo>,
  ) -> HashMap<ActivityId, ActivityStatus> {
    let mut ids = HashSet::new();
    for info in derivation_infos.values() {
      match &info.build_status {
        BuildStatus::Building(build)
        | BuildStatus::Built { info: build, .. }
        | BuildStatus::Failed { info: build, .. } => {
          if let Some(activity_id) = build.activity_id {
            ids.insert(activity_id);
          }
        },
        BuildStatus::Unknown | BuildStatus::Planned => {},
      }
    }

    ids
      .into_iter()
      .filter_map(|id| {
        self.activities.get(&id).cloned().map(|status| (id, status))
      })
      .collect()
  }

  fn render_store_path_infos(&self) -> IndexMap<StorePathId, StorePathInfo> {
    let summary = &self.full_summary;
    let mut ids = HashSet::new();
    ids.extend(summary.planned_downloads.iter().copied());
    ids.extend(summary.running_downloads.keys().copied());
    ids.extend(summary.running_uploads.keys().copied());

    ids
      .into_iter()
      .filter_map(|id| {
        self
          .store_path_infos
          .get(&id)
          .map(|info| (id, info.clone()))
      })
      .collect()
  }
}

impl RenderSnapshot {
  #[must_use]
  pub fn get_derivation_info(
    &self,
    id: DerivationId,
  ) -> Option<&RenderDerivationInfo> {
    self.derivation_infos.get(&id)
  }

  #[must_use]
  pub fn get_store_path_info(&self, id: StorePathId) -> Option<&StorePathInfo> {
    self.store_path_infos.get(&id)
  }

  #[must_use]
  pub(crate) fn sort_key(
    &self,
    id: DerivationId,
  ) -> crate::update::BuildSortKey {
    self
      .sort_keys
      .get(&id)
      .copied()
      .unwrap_or((u8::MAX, 0, u8::MAX, 0, id))
  }

  pub(crate) fn dependency_owner(
    &self,
    id: DerivationId,
  ) -> Option<DerivationId> {
    self.dependency_owners.get(&id).copied()
  }

  pub fn derivation_ids_with_name(&self, name: &str) -> Vec<DerivationId> {
    self
      .derivation_name_index
      .get(name)
      .into_iter()
      .flat_map(|ids| ids.iter().copied())
      .collect()
  }
}

fn render_completed_transfer_hosts<'a>(
  transfers: impl Iterator<Item = &'a CompletedTransferInfo>,
) -> Vec<RenderTransferHostSummary> {
  let mut hosts: BTreeMap<String, RenderTransferHostSummary> = BTreeMap::new();
  for transfer in transfers {
    let Host::Remote(host) = &transfer.host else {
      continue;
    };
    let summary = hosts.entry(host.clone()).or_insert_with(|| {
      RenderTransferHostSummary {
        host:        Host::Remote(host.clone()),
        completed:   0,
        total_bytes: 0,
      }
    });
    summary.completed = summary.completed.saturating_add(1);
    summary.total_bytes =
      summary.total_bytes.saturating_add(transfer.total_bytes);
  }
  hosts.into_values().collect()
}

fn dependency_owners(
  infos: &IndexMap<DerivationId, RenderDerivationInfo>,
) -> HashMap<DerivationId, DerivationId> {
  let mut depths = HashMap::new();
  let mut visiting = HashSet::new();
  for id in infos.keys().copied() {
    derivation_depth(infos, id, &mut depths, &mut visiting);
  }

  infos
    .iter()
    .filter_map(|(child_id, child)| {
      child
        .derivation_parents
        .iter()
        .copied()
        .max_by_key(|parent_id| {
          (
            depths.get(parent_id).copied().unwrap_or(0),
            usize::MAX.saturating_sub(*parent_id),
          )
        })
        .map(|owner| (*child_id, owner))
    })
    .collect()
}

fn derivation_depth(
  infos: &IndexMap<DerivationId, RenderDerivationInfo>,
  id: DerivationId,
  depths: &mut HashMap<DerivationId, usize>,
  visiting: &mut HashSet<DerivationId>,
) -> usize {
  if let Some(depth) = depths.get(&id) {
    return *depth;
  }
  if !visiting.insert(id) {
    return 0;
  }
  let depth = infos.get(&id).map_or(0, |info| {
    info
      .derivation_parents
      .iter()
      .map(|parent| {
        derivation_depth(infos, *parent, depths, visiting).saturating_add(1)
      })
      .max()
      .unwrap_or(0)
  });
  visiting.remove(&id);
  depths.insert(id, depth);
  depth
}

fn derivation_name_index(
  derivation_infos: &IndexMap<DerivationId, RenderDerivationInfo>,
) -> HashMap<String, HashSet<DerivationId>> {
  let mut index: HashMap<String, HashSet<DerivationId>> = HashMap::new();
  for (id, info) in derivation_infos {
    index.entry(info.name.name.clone()).or_default().insert(*id);
  }
  index
}

fn dedup_derivation_ids(ids: &mut Vec<DerivationId>) {
  let mut seen = HashSet::new();
  ids.retain(|id| seen.insert(*id));
}
