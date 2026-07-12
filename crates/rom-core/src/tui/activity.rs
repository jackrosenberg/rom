use std::collections::{HashMap, HashSet};

mod row;
use row::{
  ActivityLine,
  RenderedActivityLine,
  activity_line,
  transfer_activity_line,
};

use crate::{
  console::ConsoleConfig,
  state::{
    BuildStatus,
    DerivationId,
    RenderSnapshot,
    StorePathId,
    TransferInfo,
    current_time,
  },
  tui::screen::Line,
};

struct ActivityNode {
  drv_id:         DerivationId,
  children:       Vec<Self>,
  collapsed_deps: CollapsedDependencies,
  path_required:  bool,
}

struct RenderedActivityTree {
  row:      RenderedActivityLine,
  children: Vec<Self>,
}

#[derive(Clone, Copy, Debug, Default)]
struct CollapsedDependencies {
  built:   usize,
  waiting: usize,
  shared:  usize,
}

#[derive(Default)]
struct ActivityBuildResult {
  node:           Option<ActivityNode>,
  collapsed_deps: CollapsedDependencies,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferKind {
  Download,
  Upload,
}

#[derive(Clone)]
enum TransferActivity {
  Running {
    kind:     TransferKind,
    path_id:  StorePathId,
    transfer: TransferInfo,
  },
  PlannedDownload {
    path_id: StorePathId,
  },
}

#[derive(Default)]
struct TransferLookup {
  by_derivation:   HashMap<DerivationId, Vec<TransferActivity>>,
  displayed_paths: HashSet<StorePathId>,
}

#[derive(Clone)]
struct TransferLine {
  line:     Line,
  required: bool,
  order:    (u8, u64, StorePathId),
}

impl CollapsedDependencies {
  fn add(&mut self, other: Self) {
    self.built += other.built;
    self.waiting += other.waiting;
    self.shared += other.shared;
  }
}

impl TransferActivity {
  fn path_id(&self) -> StorePathId {
    match self {
      TransferActivity::Running { path_id, .. }
      | TransferActivity::PlannedDownload { path_id } => *path_id,
    }
  }
}

impl TransferLookup {
  fn from_state(state: &RenderSnapshot) -> Self {
    let mut lookup = Self::default();

    for (path_id, transfer) in &state.full_summary.running_downloads {
      lookup.insert_path_activity(state, *path_id, TransferActivity::Running {
        kind:     TransferKind::Download,
        path_id:  *path_id,
        transfer: transfer.clone(),
      });
    }

    for (path_id, transfer) in &state.full_summary.running_uploads {
      lookup.insert_path_activity(state, *path_id, TransferActivity::Running {
        kind:     TransferKind::Upload,
        path_id:  *path_id,
        transfer: transfer.clone(),
      });
    }

    for path_id in &state.full_summary.planned_downloads {
      lookup.insert_path_activity(
        state,
        *path_id,
        TransferActivity::PlannedDownload { path_id: *path_id },
      );
    }

    lookup.finalize();
    lookup
  }

  fn finalize(&mut self) {
    for activities in self.by_derivation.values_mut() {
      activities.sort_by_key(transfer_activity_order_key);
      activities.dedup_by_key(|activity| activity.path_id());
      if let Some(activity) = activities.first() {
        self.displayed_paths.insert(activity.path_id());
      }
    }
  }

  fn derivation_ids(&self) -> impl Iterator<Item = DerivationId> + '_ {
    self.by_derivation.keys().copied()
  }

  fn insert_path_activity(
    &mut self,
    state: &RenderSnapshot,
    path_id: StorePathId,
    activity: TransferActivity,
  ) {
    for drv_id in derivation_ids_for_transfer_path(state, path_id) {
      self.insert(drv_id, activity.clone());
    }
  }

  fn insert(&mut self, drv_id: DerivationId, activity: TransferActivity) {
    self.by_derivation.entry(drv_id).or_default().push(activity);
  }
}

fn transfer_activity_order_key(
  activity: &TransferActivity,
) -> (u8, u64, u8, StorePathId) {
  match activity {
    TransferActivity::Running {
      kind,
      path_id,
      transfer,
    } => {
      let kind = match kind {
        TransferKind::Download => 0,
        TransferKind::Upload => 1,
      };
      (
        0,
        (transfer.start.max(0.0) * 1_000.0) as u64,
        kind,
        *path_id,
      )
    },
    TransferActivity::PlannedDownload { path_id } => (1, 0, 0, *path_id),
  }
}

pub(super) fn minimum_required_activity_rows(state: &RenderSnapshot) -> usize {
  let transfers = TransferLookup::from_state(state);
  let mut required = HashSet::new();
  required.extend(state.full_summary.failed_builds.keys().copied());
  required.extend(state.full_summary.running_builds.keys().copied());
  for (drv_id, activities) in &transfers.by_derivation {
    if activities
      .iter()
      .any(|activity| matches!(activity, TransferActivity::Running { .. }))
    {
      required.insert(*drv_id);
    }
  }

  let mut stack = required.iter().copied().collect::<Vec<_>>();
  while let Some(drv_id) = stack.pop() {
    if let Some(parent_id) = state.dependency_owner(drv_id)
      && required.insert(parent_id)
    {
      stack.push(parent_id);
    }
  }

  let connected_rows = required
    .into_iter()
    .filter(|id| state.get_derivation_info(*id).is_some())
    .count();
  let orphan_transfers = state
    .full_summary
    .running_downloads
    .keys()
    .chain(state.full_summary.running_uploads.keys())
    .filter(|path_id| !transfers.displayed_paths.contains(path_id))
    .count();
  connected_rows + orphan_transfers
}

pub(super) struct ActivityGraph {
  pub(super) lines: Vec<Line>,
}

pub(super) fn render_activity_graph_lines(
  state: &RenderSnapshot,
  display: ConsoleConfig,
  max_lines: usize,
  width: usize,
) -> ActivityGraph {
  let now = current_time();
  let transfer_lookup = TransferLookup::from_state(state);
  let transfer_lines =
    transfer_line_candidates(state, &transfer_lookup, now, width);
  let orphan_budget =
    standalone_transfer_budget(max_lines).min(transfer_lines.len());
  let forest = build_activity_forest(
    state,
    display.max_tree_depth,
    max_lines.saturating_sub(orphan_budget),
    &transfer_lookup,
  );
  let render_ctx = ActivityRenderCtx {
    state,
    transfer_lookup: &transfer_lookup,
    now,
    width,
  };
  let rendered_forest = forest
    .iter()
    .flat_map(|node| render_activity_node(&render_ctx, node, 0))
    .collect::<Vec<_>>();
  let tree_lines = flatten_activity_forest(rendered_forest);

  ActivityGraph {
    lines: combine_activity_lines(tree_lines, &transfer_lines, max_lines),
  }
}

fn combine_activity_lines(
  tree_lines: Vec<RenderedActivityLine>,
  transfer_lines: &[TransferLine],
  max_lines: usize,
) -> Vec<Line> {
  let required_transfers = transfer_lines.iter().filter(|line| line.required);
  let optional_transfers = transfer_lines.iter().filter(|line| !line.required);
  let required_count = required_transfers.clone().count();
  let optional_budget =
    max_lines.saturating_sub(required_count + tree_lines.len());

  let mut lines =
    Vec::with_capacity(required_count + optional_budget + tree_lines.len());
  lines.extend(required_transfers.map(|line| line.line.clone()));
  lines.extend(
    optional_transfers
      .take(optional_budget)
      .map(|line| line.line.clone()),
  );
  lines.extend(tree_lines.iter().map(RenderedActivityLine::to_line));
  lines
}

fn standalone_transfer_budget(max_lines: usize) -> usize {
  if max_lines <= 3 {
    return 1;
  }
  (max_lines / 4).clamp(1, 6).min(max_lines - 3)
}

fn transfer_line_candidates(
  state: &RenderSnapshot,
  transfer_lookup: &TransferLookup,
  now: f64,
  width: usize,
) -> Vec<TransferLine> {
  let mut lines = Vec::new();
  for (path_id, transfer) in &state.full_summary.running_downloads {
    if transfer_lookup.displayed_paths.contains(path_id) {
      continue;
    }
    if let Some(line) = transfer_activity_line(
      state,
      &TransferActivity::Running {
        kind:     TransferKind::Download,
        path_id:  *path_id,
        transfer: transfer.clone(),
      },
      now,
      width,
    ) {
      lines.push(TransferLine {
        line,
        required: true,
        order: (0, (transfer.start.max(0.0) * 1_000.0) as u64, *path_id),
      });
    }
  }

  for (path_id, transfer) in &state.full_summary.running_uploads {
    if transfer_lookup.displayed_paths.contains(path_id) {
      continue;
    }
    if let Some(line) = transfer_activity_line(
      state,
      &TransferActivity::Running {
        kind:     TransferKind::Upload,
        path_id:  *path_id,
        transfer: transfer.clone(),
      },
      now,
      width,
    ) {
      lines.push(TransferLine {
        line,
        required: true,
        order: (1, (transfer.start.max(0.0) * 1_000.0) as u64, *path_id),
      });
    }
  }

  for path_id in &state.full_summary.planned_downloads {
    if transfer_lookup.displayed_paths.contains(path_id) {
      continue;
    }
    if let Some(line) = transfer_activity_line(
      state,
      &TransferActivity::PlannedDownload { path_id: *path_id },
      now,
      width,
    ) {
      lines.push(TransferLine {
        line,
        required: false,
        order: (2, 0, *path_id),
      });
    }
  }

  lines.sort_by_key(|line| line.order);
  lines
}

fn build_activity_forest(
  state: &RenderSnapshot,
  max_depth: usize,
  max_lines: usize,
  transfer_lookup: &TransferLookup,
) -> Vec<ActivityNode> {
  let mut roots = state.forest_roots.clone();
  if roots.is_empty() {
    roots.extend(state.full_summary.failed_builds.keys().copied());
    roots.extend(state.full_summary.running_builds.keys().copied());
    roots.extend(state.full_summary.planned_builds.iter().copied());
    roots.sort_unstable();
    roots.dedup();
  }
  let focus_ids = activity_focus_ids(state, transfer_lookup);
  let ctx = ActivityBuildCtx {
    state,
    transfer_lookup,
    max_depth,
    focus_ids: &focus_ids,
  };
  let mut visited = HashSet::new();
  let forest = roots
    .into_iter()
    .filter_map(|drv_id| {
      build_activity_node(&ctx, drv_id, 0, &mut visited).node
    })
    .collect::<Vec<_>>();
  rank_visible_forest(&ctx, forest, max_lines)
}

struct ActivityBuildCtx<'a> {
  state:           &'a RenderSnapshot,
  transfer_lookup: &'a TransferLookup,
  max_depth:       usize,
  focus_ids:       &'a HashSet<DerivationId>,
}

fn build_activity_node(
  ctx: &ActivityBuildCtx<'_>,
  drv_id: DerivationId,
  depth: usize,
  visited: &mut HashSet<DerivationId>,
) -> ActivityBuildResult {
  let Some(info) = ctx.state.get_derivation_info(drv_id) else {
    return ActivityBuildResult::default();
  };

  if !visited.insert(drv_id) {
    return ActivityBuildResult {
      node:           None,
      collapsed_deps: shared_or_collapsed_dependency(info),
    };
  }

  let mut children = Vec::new();
  let mut collapsed_deps = CollapsedDependencies::default();
  let mut visible_input_ids = Vec::new();
  for input in &info.input_derivations {
    let input_id = input.derivation;
    if ctx
      .state
      .dependency_owner(input_id)
      .is_some_and(|owner| owner != drv_id)
    {
      if let Some(shared) = ctx.state.get_derivation_info(input_id) {
        collapsed_deps.add(shared_or_collapsed_dependency(shared));
      }
      continue;
    }
    let within_optional_depth = depth < ctx.max_depth;
    let on_mandatory_path = ctx.focus_ids.contains(&input_id);
    if (within_optional_depth || on_mandatory_path)
      && should_traverse_activity_child(
        ctx.state,
        ctx.transfer_lookup,
        input_id,
        ctx.focus_ids,
      )
    {
      visible_input_ids.push(input_id);
    } else {
      collapsed_deps.add(collapsed_inactive_dependency(ctx.state, input_id));
    }
  }
  for input_id in visible_input_ids {
    let result = build_activity_node(ctx, input_id, depth + 1, visited);
    if let Some(child) = result.node {
      children.push(child);
    }
    collapsed_deps.add(result.collapsed_deps);
  }

  let has_transfer_activity =
    derivation_transfer_activity(ctx.transfer_lookup, drv_id).is_some();
  let should_render = visible_activity_status(&info.build_status)
    || has_transfer_activity
    || !children.is_empty();

  if !should_render {
    return ActivityBuildResult {
      node:           None,
      collapsed_deps: collapsed_inactive_dependency(ctx.state, drv_id),
    };
  }

  if children.is_empty()
    && collapsed_deps.built == 0
    && collapsed_deps.waiting == 0
    && collapsed_deps.shared == 0
  {
    collapsed_deps.add(collapsed_descendant_summary(info));
  }
  ActivityBuildResult {
    node:           Some(ActivityNode {
      drv_id,
      children,
      collapsed_deps,
      path_required: false,
    }),
    collapsed_deps: CollapsedDependencies::default(),
  }
}

fn should_traverse_activity_child(
  state: &RenderSnapshot,
  transfer_lookup: &TransferLookup,
  drv_id: DerivationId,
  focus_ids: &HashSet<DerivationId>,
) -> bool {
  focus_ids.contains(&drv_id)
    || state.get_derivation_info(drv_id).is_some_and(|info| {
      visible_activity_status(&info.build_status)
        || derivation_transfer_activity(transfer_lookup, drv_id).is_some()
    })
}

fn activity_focus_ids(
  state: &RenderSnapshot,
  transfer_lookup: &TransferLookup,
) -> HashSet<DerivationId> {
  let mut focus = HashSet::new();

  focus.extend(state.full_summary.failed_builds.keys().copied());
  focus.extend(state.full_summary.running_builds.keys().copied());
  focus.extend(transfer_lookup.derivation_ids());

  let mut stack = focus.iter().copied().collect::<Vec<_>>();
  while let Some(drv_id) = stack.pop() {
    if let Some(parent_id) = state.dependency_owner(drv_id)
      && focus.insert(parent_id)
    {
      stack.push(parent_id);
    }
  }

  focus
}

fn derivation_ids_for_transfer_path(
  state: &RenderSnapshot,
  path_id: crate::state::StorePathId,
) -> Vec<DerivationId> {
  let Some(store_path) = state.get_store_path_info(path_id) else {
    return Vec::new();
  };

  if let Some(producer) = store_path.producer {
    return vec![producer];
  }
  store_path
    .input_for
    .iter()
    .copied()
    .min()
    .into_iter()
    .collect()
}

fn derivation_transfer_activity(
  transfer_lookup: &TransferLookup,
  drv_id: DerivationId,
) -> Option<TransferActivity> {
  transfer_lookup
    .by_derivation
    .get(&drv_id)
    .and_then(|activities| activities.first())
    .cloned()
}

fn shared_or_collapsed_dependency(
  info: &crate::state::RenderDerivationInfo,
) -> CollapsedDependencies {
  if active_activity_status(&info.build_status)
    || matches!(info.build_status, BuildStatus::Planned)
  {
    CollapsedDependencies {
      built:   0,
      waiting: 0,
      shared:  1,
    }
  } else {
    collapsed_self_dependency(info)
  }
}

fn collapsed_descendant_summary(
  info: &crate::state::RenderDerivationInfo,
) -> CollapsedDependencies {
  CollapsedDependencies {
    built:   info.dependency_built.saturating_sub(usize::from(matches!(
      info.build_status,
      BuildStatus::Built { .. }
    ))),
    waiting: info.dependency_waiting.saturating_sub(usize::from(matches!(
      info.build_status,
      BuildStatus::Planned
    ))),
    shared:  0,
  }
}

fn collapsed_inactive_dependency(
  state: &RenderSnapshot,
  drv_id: DerivationId,
) -> CollapsedDependencies {
  let Some(info) = state.get_derivation_info(drv_id) else {
    return CollapsedDependencies::default();
  };

  let mut deps = CollapsedDependencies {
    built:   info.dependency_built,
    waiting: info.dependency_waiting,
    shared:  0,
  };

  let own = collapsed_self_dependency(info);
  if deps.built == 0 && deps.waiting == 0 {
    deps.add(own);
  }

  deps
}

fn collapsed_self_dependency(
  info: &crate::state::RenderDerivationInfo,
) -> CollapsedDependencies {
  match &info.build_status {
    BuildStatus::Planned => {
      CollapsedDependencies {
        built:   0,
        waiting: 1,
        shared:  0,
      }
    },
    BuildStatus::Built { .. } => {
      CollapsedDependencies {
        built:   1,
        waiting: 0,
        shared:  0,
      }
    },
    _ => CollapsedDependencies::default(),
  }
}

fn active_activity_status(status: &BuildStatus) -> bool {
  matches!(
    status,
    BuildStatus::Building(_) | BuildStatus::Failed { .. }
  )
}

fn visible_activity_status(status: &BuildStatus) -> bool {
  !matches!(status, BuildStatus::Unknown)
}

fn rank_visible_forest(
  ctx: &ActivityBuildCtx<'_>,
  forest: Vec<ActivityNode>,
  max_lines: usize,
) -> Vec<ActivityNode> {
  let mut parents = HashMap::new();
  let mut ids = Vec::new();
  for root in &forest {
    index_activity_tree(root, None, &mut parents, &mut ids);
  }

  let mut required = ids
    .iter()
    .copied()
    .filter(|id| activity_is_required(ctx, *id))
    .collect::<Vec<_>>();
  let mut optional = ids
    .iter()
    .copied()
    .filter(|id| !activity_is_required(ctx, *id))
    .collect::<Vec<_>>();
  required.sort_by_key(|id| ctx.state.sort_key(*id));
  optional.sort_by_key(|id| ctx.state.sort_key(*id));

  let mut selected = HashSet::new();
  let mut required_paths = HashSet::new();
  let mut rendered_count = 0;
  for id in required {
    let path = unselected_activity_path(id, &parents, &selected);
    rendered_count += path.len();
    required_paths.extend(path.iter().copied());
    selected.extend(path);
  }
  for id in optional {
    let path = unselected_activity_path(id, &parents, &selected);
    let added_rows = path
      .iter()
      .filter(|candidate| activity_renders(ctx, **candidate))
      .count();
    if rendered_count + added_rows > max_lines {
      continue;
    }
    rendered_count += added_rows;
    selected.extend(path);
    if rendered_count == max_lines {
      break;
    }
  }

  forest
    .into_iter()
    .filter_map(|node| {
      prune_activity_node(ctx, node, &selected, &required_paths)
    })
    .collect()
}

fn index_activity_tree(
  node: &ActivityNode,
  parent: Option<DerivationId>,
  parents: &mut HashMap<DerivationId, DerivationId>,
  ids: &mut Vec<DerivationId>,
) {
  ids.push(node.drv_id);
  if let Some(parent) = parent {
    parents.insert(node.drv_id, parent);
  }
  for child in &node.children {
    index_activity_tree(child, Some(node.drv_id), parents, ids);
  }
}

fn unselected_activity_path(
  id: DerivationId,
  parents: &HashMap<DerivationId, DerivationId>,
  selected: &HashSet<DerivationId>,
) -> Vec<DerivationId> {
  let mut path = Vec::new();
  let mut cursor = Some(id);
  while let Some(id) = cursor
    && !selected.contains(&id)
  {
    path.push(id);
    cursor = parents.get(&id).copied();
  }
  path
}

fn activity_renders(ctx: &ActivityBuildCtx<'_>, id: DerivationId) -> bool {
  ctx.state.get_derivation_info(id).is_some_and(|info| {
    visible_activity_status(&info.build_status)
      || derivation_transfer_activity(ctx.transfer_lookup, id).is_some()
  })
}

fn activity_is_required(ctx: &ActivityBuildCtx<'_>, id: DerivationId) -> bool {
  ctx.state.get_derivation_info(id).is_some_and(|info| {
    matches!(
      info.build_status,
      BuildStatus::Building(_) | BuildStatus::Failed { .. }
    ) || matches!(
      derivation_transfer_activity(ctx.transfer_lookup, id),
      Some(TransferActivity::Running { .. })
    )
  })
}

fn prune_activity_node(
  ctx: &ActivityBuildCtx<'_>,
  mut node: ActivityNode,
  selected: &HashSet<DerivationId>,
  required_paths: &HashSet<DerivationId>,
) -> Option<ActivityNode> {
  if !selected.contains(&node.drv_id) {
    return None;
  }

  node.path_required = required_paths.contains(&node.drv_id);
  let old_children = std::mem::take(&mut node.children);
  for child in old_children {
    if selected.contains(&child.drv_id) {
      if let Some(child) =
        prune_activity_node(ctx, child, selected, required_paths)
      {
        node.children.push(child);
      }
    } else {
      node.collapsed_deps.add(child.collapsed_deps);
      if let Some(info) = ctx.state.get_derivation_info(child.drv_id) {
        node.collapsed_deps.add(collapsed_self_dependency(info));
      }
    }
  }
  Some(node)
}

struct ActivityRenderCtx<'a> {
  state:           &'a RenderSnapshot,
  transfer_lookup: &'a TransferLookup,
  now:             f64,
  width:           usize,
}

fn render_activity_node(
  ctx: &ActivityRenderCtx<'_>,
  node: &ActivityNode,
  depth: usize,
) -> Vec<RenderedActivityTree> {
  let Some(info) = ctx.state.get_derivation_info(node.drv_id) else {
    return Vec::new();
  };

  let renders_self = node.path_required
    || visible_activity_status(&info.build_status)
    || derivation_transfer_activity(ctx.transfer_lookup, node.drv_id).is_some();
  let child_depth = depth + usize::from(renders_self);
  let children = node
    .children
    .iter()
    .flat_map(|child| render_activity_node(ctx, child, child_depth))
    .collect::<Vec<_>>();

  if !renders_self {
    return children;
  }

  vec![RenderedActivityTree {
    row: activity_line(ActivityLine {
      state: ctx.state,
      transfer_lookup: ctx.transfer_lookup,
      drv_id: node.drv_id,
      info,
      collapsed_deps: node.collapsed_deps,
      depth,
      now: ctx.now,
      width: ctx.width,
    }),
    children,
  }]
}

fn flatten_activity_forest(
  forest: Vec<RenderedActivityTree>,
) -> Vec<RenderedActivityLine> {
  let mut lines = flatten_activity_forest_preorder(forest, false);
  lines.reverse();
  lines
}

fn flatten_activity_forest_preorder(
  forest: Vec<RenderedActivityTree>,
  indent: bool,
) -> Vec<RenderedActivityLine> {
  let tree_count = forest.len();
  let mut lines = Vec::new();
  for (index, tree) in forest.into_iter().enumerate() {
    let mut tree_lines = vec![tree.row];
    tree_lines.extend(flatten_activity_forest_preorder(tree.children, true));

    if indent {
      let last_tree = index + 1 == tree_count;
      tree_lines = tree_lines
        .into_iter()
        .enumerate()
        .map(|(line_index, line)| {
          let prefix = match (last_tree, line_index == 0) {
            (true, true) => "┌─ ",
            (true, false) => "   ",
            (false, true) => "├─ ",
            (false, false) => "│  ",
          };
          line.with_prefix(prefix)
        })
        .collect();
    }
    lines.extend(tree_lines);
  }
  lines
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    state::{Derivation, InputDerivation, State},
    tui::screen::Span,
  };

  fn row(label: &str) -> RenderedActivityLine {
    RenderedActivityLine {
      line: Line::from(Span::raw(label)),
    }
  }

  fn text(line: &RenderedActivityLine) -> String {
    line
      .line
      .spans
      .iter()
      .map(|span| span.content.as_str())
      .collect()
  }

  #[test]
  fn pruning_shared_dag_counts_each_owned_dependency_once() {
    let mut state = State::new();
    let mut add = |name: &str| {
      state.get_or_create_derivation_id(
        Derivation::parse(&format!(
          "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-{name}.drv"
        ))
        .expect("valid derivation"),
      )
    };
    let root = add("root");
    let first = add("first");
    let second = add("second");
    let shared = add("shared");

    for (parent, child) in [
      (root, first),
      (root, second),
      (first, shared),
      (second, shared),
    ] {
      state
        .get_derivation_info_mut(parent)
        .expect("parent")
        .input_derivations
        .push(InputDerivation {
          derivation: child,
          outputs:    HashSet::new(),
        });
      state
        .get_derivation_info_mut(child)
        .expect("child")
        .derivation_parents
        .insert(parent);
    }
    for id in [shared, first, second, root] {
      state.update_build_status(id, BuildStatus::Planned);
    }
    state.forest_roots.push(root);

    let graph = render_activity_graph_lines(
      &state.render_snapshot(),
      ConsoleConfig::default(),
      1,
      80,
    );
    let rendered = graph.lines[0]
      .spans
      .iter()
      .map(|span| span.content.as_str())
      .collect::<String>();
    assert!(rendered.contains("waiting 2 · shared 1"), "{rendered}");
    assert!(!rendered.contains("waiting 4"), "{rendered}");
  }

  #[test]
  fn flattening_matches_nom_reverse_forest_connectors() {
    let forest = vec![RenderedActivityTree {
      row:      row("root"),
      children: vec![
        RenderedActivityTree {
          row:      row("parent"),
          children: vec![RenderedActivityTree {
            row:      row("nested"),
            children: Vec::new(),
          }],
        },
        RenderedActivityTree {
          row:      row("sibling"),
          children: Vec::new(),
        },
      ],
    }];

    let rendered = flatten_activity_forest(forest)
      .iter()
      .map(text)
      .collect::<Vec<_>>();
    assert_eq!(rendered, [
      "┌─ sibling",
      "│  ┌─ nested",
      "├─ parent",
      "root",
    ]);
  }
}
