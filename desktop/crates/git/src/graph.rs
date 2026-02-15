use std::{
  collections::{BTreeMap, HashMap, HashSet},
  path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use git2::{
  BranchType, Delta, Diff, DiffDelta, DiffFindOptions, DiffOptions, Oid, Patch, Repository, Sort,
  Tree,
};

#[derive(Clone, Debug)]
pub struct CommitGraphNode {
  pub oid: String,
  pub short_oid: String,
  pub summary: String,
  pub author: String,
  pub parent_oids: Vec<String>,
  pub refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFileChangeKind {
  Added,
  Deleted,
  Modified,
  Renamed,
  Copied,
  Typechange,
  Conflicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitChangedFile {
  pub path: PathBuf,
  pub old_path: Option<PathBuf>,
  pub kind: CommitFileChangeKind,
}

#[derive(Clone, Debug)]
pub struct CommitFileDiff {
  pub file: CommitChangedFile,
  pub patch: String,
  pub content: String,
}

pub fn list_commit_graph(repo_root: &Path, limit: usize) -> Result<Vec<CommitGraphNode>> {
  if limit == 0 {
    return Ok(Vec::new());
  }

  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let mut refs_by_oid = refs_by_oid(&repo)?;

  let mut walk = repo.revwalk()?;
  // Match git graph-style traversal: strict topological order.
  // Combining with TIME (date-order) can interleave branches differently.
  walk.set_sorting(Sort::TOPOLOGICAL)?;

  let Some((head_oid, head_label)) = head_target(&repo) else {
    return Ok(Vec::new());
  };
  if walk.push(head_oid).is_err() {
    return Ok(Vec::new());
  }
  insert_ref_label(&mut refs_by_oid, head_oid, head_label);

  let mut rows = Vec::with_capacity(limit);
  for oid_result in walk.take(limit) {
    let oid = oid_result?;
    let Ok(commit) = repo.find_commit(oid) else {
      continue;
    };

    let mut refs = refs_by_oid.remove(&oid).unwrap_or_default();
    refs.sort();
    refs.dedup();

    let summary = commit
      .summary()
      .unwrap_or("No commit message")
      .replace(['\n', '\r'], "");
    let author = commit.author().name().unwrap_or("Unknown").to_string();
    let parent_oids = commit
      .parent_ids()
      .map(|parent| parent.to_string())
      .collect();

    rows.push(CommitGraphNode {
      oid: oid.to_string(),
      short_oid: short_oid(oid),
      summary,
      author,
      parent_oids,
      refs,
    });
  }

  Ok(rows)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphLaneSegment {
  pub up: bool,
  pub down: bool,
}

#[derive(Clone, Debug)]
pub struct CommitGraphRow {
  pub commit: CommitGraphNode,
  pub segments: Vec<GraphLaneSegment>,
  pub lane_branch_ids: Vec<Option<usize>>,
  pub commit_lane: usize,
  pub commit_branch_id: usize,
  pub lane_transitions: Vec<(usize, usize, usize)>,
  pub merge_parent_lanes: Vec<usize>,
  pub merge_parent_lane_branches: Vec<(usize, usize)>,
  pub branch_child_lanes: Vec<usize>,
  pub branch_child_lane_branches: Vec<(usize, usize)>,
  pub branch_pre_stubs: Vec<usize>,
  pub branch_pre_stub_lane_branches: Vec<(usize, usize)>,
  pub commit_lane_has_up: bool,
}

#[derive(Clone, Debug)]
struct GraphLaneState {
  oid: String,
  branch_id: usize,
}

pub fn build_commit_graph_rows(commits: &[CommitGraphNode]) -> Vec<CommitGraphRow> {
  let commits_by_oid = commits
    .iter()
    .map(|commit| (commit.oid.as_str(), commit))
    .collect::<HashMap<_, _>>();
  let mut mainline_oids = HashSet::new();
  if let Some(head) = commits.first() {
    let mut cursor = Some(head.oid.as_str());
    while let Some(oid) = cursor {
      if !mainline_oids.insert(oid.to_string()) {
        break;
      }
      cursor = commits_by_oid
        .get(oid)
        .and_then(|commit| commit.parent_oids.first())
        .map(String::as_str);
    }
  }

  let mut rows = Vec::with_capacity(commits.len());
  let mut active_lanes: Vec<Option<GraphLaneState>> = Vec::new();
  let mut next_branch_id = 0usize;
  let mut branch_seen_commit_count = HashMap::<usize, usize>::new();
  let mut pending_child_lanes_by_parent = HashMap::<String, Vec<usize>>::new();
  let branch_chain_length_to_target = |start_oid: &str, target_oid: Option<&str>| -> usize {
    let mut length = 0usize;
    let mut cursor = Some(start_oid);
    let mut seen = HashSet::<String>::new();
    while let Some(oid) = cursor {
      if Some(oid) == target_oid {
        break;
      }
      if !seen.insert(oid.to_string()) {
        break;
      }
      length += 1;
      cursor = commits_by_oid
        .get(oid)
        .and_then(|commit| commit.parent_oids.first())
        .map(String::as_str);
    }
    length
  };

  let trim_trailing_empty = |lanes: &mut Vec<Option<GraphLaneState>>| {
    while lanes.last().map(|lane| lane.is_none()).unwrap_or(false) {
      lanes.pop();
    }
  };

  for commit in commits {
    let pending_reserved_child_lanes = pending_child_lanes_by_parent
      .remove(commit.oid.as_str())
      .map(|lanes| lanes.into_iter().collect::<HashSet<_>>())
      .unwrap_or_default();
    let is_mainline_commit = mainline_oids.contains(commit.oid.as_str());
    let mut commit_lane_has_up = true;
    let mut commit_lane = if let Some(ix) = active_lanes
      .iter()
      .position(|lane| lane.as_ref().map(|state| state.oid.as_str()) == Some(commit.oid.as_str()))
    {
      ix
    } else {
      commit_lane_has_up = false;
      let search_start = if is_mainline_commit { 0 } else { 1 };
      let insert_lane = (search_start..active_lanes.len())
        .find(|ix| active_lanes[*ix].is_none())
        .unwrap_or(active_lanes.len());
      if insert_lane >= active_lanes.len() {
        active_lanes.resize(insert_lane + 1, None);
      }
      let branch_id = next_branch_id;
      next_branch_id += 1;
      active_lanes[insert_lane] = Some(GraphLaneState {
        oid: commit.oid.clone(),
        branch_id,
      });
      insert_lane
    };
    if is_mainline_commit && commit_lane != 0 {
      active_lanes.swap(0, commit_lane);
      commit_lane = 0;
    }
    let commit_branch_id = active_lanes
      .get(commit_lane)
      .and_then(|lane| lane.as_ref())
      .map(|lane| lane.branch_id)
      .unwrap_or_else(|| {
        let branch_id = next_branch_id;
        next_branch_id += 1;
        branch_id
      });
    *branch_seen_commit_count
      .entry(commit_branch_id)
      .or_insert(0) += 1;

    let lanes_before = active_lanes.clone();
    let mut reserved_child_lanes = pending_reserved_child_lanes;
    reserved_child_lanes.extend(
      lanes_before
        .iter()
        .enumerate()
        .filter_map(|(lane_ix, lane)| {
          lane.as_ref().and_then(|state| {
            (state.oid == commit.oid && lane_ix != commit_lane).then_some(lane_ix)
          })
        })
        .collect::<HashSet<_>>(),
    );
    let reserved_child_lanes_with_state = reserved_child_lanes
      .iter()
      .copied()
      .filter(|lane_ix| {
        lanes_before
          .get(*lane_ix)
          .and_then(|lane| lane.as_ref())
          .is_some()
      })
      .collect::<HashSet<_>>();
    let mut lanes_after = lanes_before.clone();
    let mut parent_lanes = Vec::new();
    let mut first_parent_lane = None;
    let mut vacated_first_parent_lane = None;
    lanes_after[commit_lane] = None;
    let mut lock_commit_lane_empty = false;

    if let Some(first_parent) = commit.parent_oids.first() {
      if let Some(existing_idx) = lanes_after.iter().position(|lane| {
        lane.as_ref().map(|state| state.oid.as_str()) == Some(first_parent.as_str())
      }) {
        if existing_idx == commit_lane || is_mainline_commit {
          lanes_after[commit_lane] = Some(GraphLaneState {
            oid: first_parent.clone(),
            branch_id: commit_branch_id,
          });
          if existing_idx != commit_lane {
            // For mainline commits, keep a concurrent side-branch lane alive when
            // it points to the same first-parent commit but carries a different
            // branch identity. This preserves the vertical continuity between
            // branch creation and merge rows.
            let existing_branch_id = lanes_after[existing_idx]
              .as_ref()
              .map(|state| state.branch_id);
            let should_clear_existing =
              !is_mainline_commit || existing_branch_id == Some(commit_branch_id);
            if should_clear_existing {
              lanes_after[existing_idx] = None;
              vacated_first_parent_lane = Some(existing_idx);
            }
          }
          if !parent_lanes.contains(&commit_lane) {
            parent_lanes.push(commit_lane);
          }
          first_parent_lane = Some(commit_lane);
        } else {
          if !parent_lanes.contains(&existing_idx) {
            parent_lanes.push(existing_idx);
          }
          first_parent_lane = Some(existing_idx);
          if existing_idx != commit_lane {
            lock_commit_lane_empty = true;
          }
        }
      } else {
        lanes_after[commit_lane] = Some(GraphLaneState {
          oid: first_parent.clone(),
          branch_id: commit_branch_id,
        });
        parent_lanes.push(commit_lane);
        first_parent_lane = Some(commit_lane);
      }
    } else {
      lock_commit_lane_empty = true;
    }

    let mut preferred_lane = commit_lane + 1;
    let first_parent_target = commit.parent_oids.first().map(String::as_str);
    let mut additional_parents = commit
      .parent_oids
      .iter()
      .skip(1)
      .cloned()
      .collect::<Vec<_>>();
    additional_parents.sort_by(|a, b| {
      let score_a = branch_chain_length_to_target(a.as_str(), first_parent_target);
      let score_b = branch_chain_length_to_target(b.as_str(), first_parent_target);
      score_b.cmp(&score_a).then_with(|| a.cmp(b))
    });
    for parent in additional_parents {
      let parent_priority = branch_chain_length_to_target(parent.as_str(), first_parent_target);
      if let Some(existing_idx) = lanes_after
        .iter()
        .position(|lane| lane.as_ref().map(|state| state.oid.as_str()) == Some(parent.as_str()))
      {
        if !parent_lanes.contains(&existing_idx) {
          parent_lanes.push(existing_idx);
        }
        preferred_lane = existing_idx + 1;
        continue;
      }

      let min_insert_lane = if reserved_child_lanes_with_state.len() > 1 {
        reserved_child_lanes_with_state
          .iter()
          .copied()
          .min()
          .unwrap_or(preferred_lane)
      } else if reserved_child_lanes_with_state.len() == 1 {
        let child_lane = *reserved_child_lanes_with_state
          .iter()
          .next()
          .unwrap_or(&preferred_lane);
        let child_priority = lanes_before
          .get(child_lane)
          .and_then(|lane| lane.as_ref())
          .map(|state| {
            let oid_priority =
              branch_chain_length_to_target(state.oid.as_str(), first_parent_target);
            let seen_priority = branch_seen_commit_count
              .get(&state.branch_id)
              .copied()
              .unwrap_or(0);
            oid_priority.max(seen_priority)
          })
          .unwrap_or(0);
        if parent_priority <= child_priority {
          preferred_lane.max(child_lane + 1)
        } else {
          preferred_lane
        }
      } else {
        preferred_lane
      };
      let mut insert_at = preferred_lane.max(min_insert_lane);
      loop {
        if reserved_child_lanes_with_state.contains(&insert_at) {
          insert_at += 1;
          continue;
        }

        if insert_at < lanes_after.len() {
          let lane_used_above = lanes_before
            .get(insert_at)
            .and_then(|lane| lane.as_ref())
            .is_some();
          let slot_is_available = lanes_after[insert_at].is_none()
            && (!lane_used_above || vacated_first_parent_lane == Some(insert_at))
            && (!lock_commit_lane_empty || insert_at != commit_lane);
          if slot_is_available {
            let vacated_lane_has_higher_or_equal_priority = lanes_before
              .get(insert_at)
              .and_then(|lane| lane.as_ref())
              .is_some_and(|state| {
                let oid_priority =
                  branch_chain_length_to_target(state.oid.as_str(), first_parent_target);
                let seen_priority = branch_seen_commit_count
                  .get(&state.branch_id)
                  .copied()
                  .unwrap_or(0);
                oid_priority.max(seen_priority) >= parent_priority
              });
            if vacated_lane_has_higher_or_equal_priority {
              insert_at += 1;
              continue;
            }

            let right_lane_count = lanes_before.len().max(lanes_after.len());
            let right_lane_has_higher_or_equal_priority =
              ((insert_at + 1)..right_lane_count).any(|lane_ix| {
                let candidate_state = lanes_after
                  .get(lane_ix)
                  .and_then(|lane| lane.as_ref())
                  .or_else(|| lanes_before.get(lane_ix).and_then(|lane| lane.as_ref()));
                candidate_state.is_some_and(|state| {
                  let oid_priority =
                    branch_chain_length_to_target(state.oid.as_str(), first_parent_target);
                  let seen_priority = branch_seen_commit_count
                    .get(&state.branch_id)
                    .copied()
                    .unwrap_or(0);
                  oid_priority.max(seen_priority) >= parent_priority
                })
              });
            if right_lane_has_higher_or_equal_priority {
              insert_at += 1;
              continue;
            }
            break;
          }
          insert_at += 1;
          continue;
        }

        break;
      }

      let branch_id = next_branch_id;
      next_branch_id += 1;
      if insert_at >= lanes_after.len() {
        lanes_after.resize(insert_at + 1, None);
      }
      lanes_after[insert_at] = Some(GraphLaneState {
        oid: parent.clone(),
        branch_id,
      });
      parent_lanes.push(insert_at);
      preferred_lane = insert_at + 1;
    }

    // Drop duplicate aliases of the current commit on side lanes.
    // They are useful to draw continuity up to this row, but must not
    // continue below, otherwise we render full-height stale columns.
    for (lane_ix, lane_state) in lanes_after.iter_mut().enumerate() {
      if lane_ix == commit_lane {
        continue;
      }
      if lane_state
        .as_ref()
        .is_some_and(|state| state.oid == commit.oid)
      {
        *lane_state = None;
      }
    }

    trim_trailing_empty(&mut lanes_after);

    let lane_count = lanes_before
      .len()
      .max(lanes_after.len())
      .max(commit_lane + 1);

    let segments = (0..lane_count)
      .map(|lane| GraphLaneSegment {
        up: lanes_before
          .get(lane)
          .and_then(|lane| lane.as_ref())
          .is_some(),
        down: lanes_after
          .get(lane)
          .and_then(|lane| lane.as_ref())
          .is_some(),
      })
      .collect::<Vec<_>>();
    let lane_branch_ids = (0..lane_count)
      .map(|lane| {
        lanes_before
          .get(lane)
          .and_then(|lane| lane.as_ref())
          .map(|lane| lane.branch_id)
          .or_else(|| {
            lanes_after
              .get(lane)
              .and_then(|lane| lane.as_ref())
              .map(|lane| lane.branch_id)
          })
      })
      .collect::<Vec<_>>();

    let merge_parent_lanes = if commit.parent_oids.len() > 1 {
      parent_lanes
        .iter()
        .copied()
        .filter(|lane| Some(*lane) != first_parent_lane)
        .collect::<Vec<_>>()
    } else {
      Vec::new()
    };
    let merge_parent_lane_branches = merge_parent_lanes
      .iter()
      .copied()
      .map(|lane| {
        let branch_id = lane_branch_ids
          .get(lane)
          .and_then(|branch_id| *branch_id)
          .unwrap_or(commit_branch_id);
        (lane, branch_id)
      })
      .collect::<Vec<_>>();
    rows.push(CommitGraphRow {
      commit: commit.clone(),
      segments,
      lane_branch_ids,
      commit_lane,
      commit_branch_id,
      lane_transitions: Vec::new(),
      merge_parent_lanes,
      merge_parent_lane_branches,
      branch_child_lanes: Vec::new(),
      branch_child_lane_branches: Vec::new(),
      branch_pre_stubs: Vec::new(),
      branch_pre_stub_lane_branches: Vec::new(),
      commit_lane_has_up,
    });
    active_lanes = lanes_after;
    trim_trailing_empty(&mut active_lanes);
    if commit.parent_oids.len() == 1
      && let Some(parent_oid) = commit.parent_oids.first()
      && commit_lane > 0
    {
      pending_child_lanes_by_parent
        .entry(parent_oid.clone())
        .or_default()
        .push(commit_lane);
    }
  }

  let max_lane_count = rows.iter().map(|row| row.segments.len()).max().unwrap_or(0);
  if max_lane_count > 0 {
    for row in &mut rows {
      row
        .segments
        .resize(max_lane_count, GraphLaneSegment::default());
      row.lane_branch_ids.resize(max_lane_count, None);
    }
  }

  let row_index_by_oid = rows
    .iter()
    .enumerate()
    .map(|(index, row)| (row.commit.oid.clone(), index))
    .collect::<HashMap<_, _>>();
  let mut branch_child_lanes_by_row = HashMap::<usize, Vec<(usize, usize)>>::new();
  let mut branch_pre_stubs_by_row = HashMap::<usize, Vec<(usize, usize)>>::new();
  let mut branch_tail_bridges_by_row = HashMap::<usize, Vec<(usize, usize)>>::new();
  let mut branch_tail_down_by_row = HashMap::<usize, Vec<(usize, usize)>>::new();

  for (child_row_index, child_row) in rows.iter().enumerate() {
    if child_row.commit.parent_oids.len() != 1 {
      continue;
    }
    let Some(parent_row_index) = child_row
      .commit
      .parent_oids
      .first()
      .and_then(|parent_oid| row_index_by_oid.get(parent_oid))
      .copied()
    else {
      continue;
    };

    let parent_lane = rows[parent_row_index].commit_lane;
    let child_lane = child_row.commit_lane;
    let child_branch_id = child_row.commit_branch_id;
    if child_lane != parent_lane {
      branch_child_lanes_by_row
        .entry(parent_row_index)
        .or_default()
        .push((child_lane, child_branch_id));

      let parent_has_up = rows[parent_row_index]
        .segments
        .get(child_lane)
        .map(|segment| segment.up)
        .unwrap_or(false);
      if !parent_has_up && parent_row_index > 0 {
        branch_pre_stubs_by_row
          .entry(parent_row_index - 1)
          .or_default()
          .push((child_lane, child_branch_id));
      }

      // Keep the child root lane connected down to its parent row, even when the
      // parent is immediately adjacent. Without this, the first commit on a
      // created branch can show a visual gap below the dot.
      branch_tail_down_by_row
        .entry(child_row_index)
        .or_default()
        .push((child_lane, child_branch_id));

      if parent_row_index > child_row_index + 1 {
        for row_ix in (child_row_index + 1)..parent_row_index {
          branch_tail_bridges_by_row
            .entry(row_ix)
            .or_default()
            .push((child_lane, child_branch_id));
        }
      }
    }
  }

  for (row_index, mut lanes) in branch_child_lanes_by_row {
    lanes.sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    lanes.dedup();
    if let Some(row) = rows.get_mut(row_index) {
      let mut unique_lanes = lanes.iter().map(|(lane, _)| *lane).collect::<Vec<_>>();
      unique_lanes.sort_unstable();
      unique_lanes.dedup();
      row.branch_child_lanes = unique_lanes;
      row.branch_child_lane_branches = lanes;
    }
  }

  for (row_index, mut lanes) in branch_pre_stubs_by_row {
    lanes.sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    lanes.dedup();
    if let Some(row) = rows.get_mut(row_index) {
      let mut unique_lanes = lanes.iter().map(|(lane, _)| *lane).collect::<Vec<_>>();
      unique_lanes.sort_unstable();
      unique_lanes.dedup();
      row.branch_pre_stubs = unique_lanes;
      row.branch_pre_stub_lane_branches = lanes;
    }
  }

  for (row_index, mut lanes) in branch_tail_bridges_by_row {
    lanes.sort_unstable_by_key(|(lane, _)| *lane);
    lanes.dedup_by_key(|(lane, _)| *lane);
    if let Some(row) = rows.get_mut(row_index) {
      for (lane, branch_id) in lanes {
        if lane >= row.segments.len() {
          row.segments.resize(lane + 1, GraphLaneSegment::default());
          row.lane_branch_ids.resize(lane + 1, None);
        }
        row.segments[lane].up = true;
        row.segments[lane].down = true;
        if row
          .lane_branch_ids
          .get(lane)
          .is_some_and(|branch| branch.is_none())
        {
          row.lane_branch_ids[lane] = Some(branch_id);
        }
      }
    }
  }

  for (row_index, mut lanes) in branch_tail_down_by_row {
    lanes.sort_unstable_by_key(|(lane, _)| *lane);
    lanes.dedup_by_key(|(lane, _)| *lane);
    if let Some(row) = rows.get_mut(row_index) {
      for (lane, branch_id) in lanes {
        if lane >= row.segments.len() {
          row.segments.resize(lane + 1, GraphLaneSegment::default());
          row.lane_branch_ids.resize(lane + 1, None);
        }
        row.segments[lane].down = true;
        if row
          .lane_branch_ids
          .get(lane)
          .is_some_and(|branch| branch.is_none())
        {
          row.lane_branch_ids[lane] = Some(branch_id);
        }
      }
    }
  }

  let compact_row_lane_holes = |row: &mut CommitGraphRow| {
    let lane_count = row.segments.len();
    if lane_count <= 1 {
      return;
    }
    let mut occupied = HashSet::<usize>::new();
    occupied.insert(0);
    occupied.insert(row.commit_lane);
    for lane in row.merge_parent_lanes.iter().copied() {
      occupied.insert(lane);
    }
    for lane in row.branch_child_lanes.iter().copied() {
      occupied.insert(lane);
    }
    for lane in row.branch_pre_stubs.iter().copied() {
      occupied.insert(lane);
    }
    for lane in 0..lane_count {
      if row
        .lane_branch_ids
        .get(lane)
        .and_then(|branch| *branch)
        .is_some()
        || row
          .segments
          .get(lane)
          .is_some_and(|segment| segment.up || segment.down)
      {
        occupied.insert(lane);
      }
    }
    if occupied.is_empty() {
      return;
    }
    let mut old_to_new = vec![usize::MAX; lane_count];
    let mut next_lane = 0usize;
    for old_lane in 0..lane_count {
      if occupied.contains(&old_lane) {
        old_to_new[old_lane] = next_lane;
        next_lane += 1;
      }
    }
    let needs_remap = occupied
      .iter()
      .copied()
      .any(|lane| old_to_new.get(lane).copied().unwrap_or(lane) != lane);
    if !needs_remap {
      return;
    }

    let new_lane_count = next_lane.max(1);
    let mut new_segments = vec![GraphLaneSegment::default(); new_lane_count];
    let mut new_lane_branch_ids = vec![None; new_lane_count];
    for old_lane in 0..lane_count {
      let new_lane = old_to_new[old_lane];
      if new_lane == usize::MAX {
        continue;
      }
      if let Some(segment) = row.segments.get(old_lane).copied() {
        new_segments[new_lane] = segment;
      }
      if let Some(branch_id) = row.lane_branch_ids.get(old_lane).and_then(|branch| *branch) {
        new_lane_branch_ids[new_lane] = Some(branch_id);
      }
    }

    let remap_lane = |lane: usize| -> Option<usize> {
      old_to_new
        .get(lane)
        .copied()
        .and_then(|mapped| (mapped != usize::MAX).then_some(mapped))
    };
    let remap_lanes = |lanes: &[usize]| -> Vec<usize> {
      let mut mapped = lanes
        .iter()
        .filter_map(|lane| remap_lane(*lane))
        .collect::<Vec<_>>();
      mapped.sort_unstable();
      mapped.dedup();
      mapped
    };
    let remap_lane_branches = |lanes: &[(usize, usize)]| -> Vec<(usize, usize)> {
      let mut mapped = lanes
        .iter()
        .filter_map(|(lane, branch_id)| {
          remap_lane(*lane).map(|mapped_lane| (mapped_lane, *branch_id))
        })
        .collect::<Vec<_>>();
      mapped.sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
      mapped.dedup();
      mapped
    };

    row.commit_lane = remap_lane(row.commit_lane).unwrap_or(0);
    row.merge_parent_lanes = remap_lanes(&row.merge_parent_lanes);
    row.merge_parent_lane_branches = remap_lane_branches(&row.merge_parent_lane_branches);
    row.branch_child_lanes = remap_lanes(&row.branch_child_lanes);
    row.branch_child_lane_branches = remap_lane_branches(&row.branch_child_lane_branches);
    row.branch_pre_stubs = remap_lanes(&row.branch_pre_stubs);
    row.branch_pre_stub_lane_branches = remap_lane_branches(&row.branch_pre_stub_lane_branches);
    row.segments = new_segments;
    row.lane_branch_ids = new_lane_branch_ids;
  };

  // Compact per-row lane holes so active branches stay visually tight.
  for row in rows.iter_mut() {
    compact_row_lane_holes(row);
  }

  let branch_total_commits = rows
    .iter()
    .fold(HashMap::<usize, usize>::new(), |mut counts, row| {
      *counts.entry(row.commit_branch_id).or_insert(0) += 1;
      counts
    });

  let remap_branch_lane_refs =
    |row: &mut CommitGraphRow, branch_id: usize, from_lane: usize, to_lane: usize| {
      if row.commit_branch_id == branch_id && row.commit_lane == from_lane {
        row.commit_lane = to_lane;
      }
      for (lane, candidate_branch_id) in row.merge_parent_lane_branches.iter_mut() {
        if *candidate_branch_id == branch_id && *lane == from_lane {
          *lane = to_lane;
        }
      }
      for (lane, candidate_branch_id) in row.branch_child_lane_branches.iter_mut() {
        if *candidate_branch_id == branch_id && *lane == from_lane {
          *lane = to_lane;
        }
      }
      for (lane, candidate_branch_id) in row.branch_pre_stub_lane_branches.iter_mut() {
        if *candidate_branch_id == branch_id && *lane == from_lane {
          *lane = to_lane;
        }
      }
    };
  let move_branch_lane =
    |row: &mut CommitGraphRow, branch_id: usize, from_lane: usize, to_lane: usize| {
      if from_lane == to_lane {
        return;
      }
      if from_lane >= row.segments.len() || from_lane >= row.lane_branch_ids.len() {
        return;
      }
      if to_lane >= row.segments.len() {
        row
          .segments
          .resize(to_lane + 1, GraphLaneSegment::default());
        row.lane_branch_ids.resize(to_lane + 1, None);
      }

      let moved_segment = row.segments[from_lane];
      row.segments[from_lane] = GraphLaneSegment::default();
      row.segments[to_lane].up |= moved_segment.up;
      row.segments[to_lane].down |= moved_segment.down;
      row.lane_branch_ids[from_lane] = None;
      row.lane_branch_ids[to_lane] = Some(branch_id);
      remap_branch_lane_refs(row, branch_id, from_lane, to_lane);
    };
  let swap_branch_lanes = |row: &mut CommitGraphRow,
                           left_branch_id: usize,
                           left_lane: usize,
                           right_branch_id: usize,
                           right_lane: usize| {
    if left_lane >= row.segments.len() || right_lane >= row.segments.len() {
      return;
    }
    if left_lane >= row.lane_branch_ids.len() || right_lane >= row.lane_branch_ids.len() {
      return;
    }
    let left_segment = row.segments[left_lane];
    let right_segment = row.segments[right_lane];
    row.segments[left_lane] = right_segment;
    row.segments[right_lane] = left_segment;
    row.lane_branch_ids[left_lane] = Some(right_branch_id);
    row.lane_branch_ids[right_lane] = Some(left_branch_id);
    remap_branch_lane_refs(row, left_branch_id, left_lane, right_lane);
    remap_branch_lane_refs(row, right_branch_id, right_lane, left_lane);
  };
  let normalize_row_lane_metadata = |row: &mut CommitGraphRow| {
    row
      .merge_parent_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.merge_parent_lane_branches.dedup();
    row.merge_parent_lanes = row
      .merge_parent_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.merge_parent_lanes.sort_unstable();
    row.merge_parent_lanes.dedup();

    row
      .branch_child_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_child_lane_branches.dedup();
    row.branch_child_lanes = row
      .branch_child_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_child_lanes.sort_unstable();
    row.branch_child_lanes.dedup();

    row
      .branch_pre_stub_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_pre_stub_lane_branches.dedup();
    row.branch_pre_stubs = row
      .branch_pre_stub_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_pre_stubs.sort_unstable();
    row.branch_pre_stubs.dedup();
  };

  // On a merge row, if a longer merge-parent branch is immediately to the right
  // of a shorter branch-child lane, swap them so the longer branch stays left.
  for row in rows.iter_mut() {
    loop {
      let child_branch_by_lane = row
        .branch_child_lane_branches
        .iter()
        .copied()
        .collect::<HashMap<_, _>>();
      let merge_parent_entries = row.merge_parent_lane_branches.clone();
      let mut swapped = false;
      for (merge_lane, merge_branch_id) in merge_parent_entries {
        if merge_lane == 0 {
          continue;
        }
        let left_lane = merge_lane - 1;
        let Some(child_branch_id) = child_branch_by_lane.get(&left_lane).copied() else {
          continue;
        };
        if row
          .lane_branch_ids
          .get(left_lane)
          .and_then(|branch| *branch)
          != Some(child_branch_id)
        {
          continue;
        }
        if row
          .lane_branch_ids
          .get(merge_lane)
          .and_then(|branch| *branch)
          != Some(merge_branch_id)
        {
          continue;
        }
        let merge_priority = branch_total_commits
          .get(&merge_branch_id)
          .copied()
          .unwrap_or(0);
        let child_priority = branch_total_commits
          .get(&child_branch_id)
          .copied()
          .unwrap_or(0);
        if merge_priority <= child_priority {
          continue;
        }
        swap_branch_lanes(row, child_branch_id, left_lane, merge_branch_id, merge_lane);
        normalize_row_lane_metadata(row);
        swapped = true;
        break;
      }
      if !swapped {
        break;
      }
    }
  }

  // Keep branch lanes stable across adjacent rows to avoid oscillating left/right
  // moves. Exception: right after a merge-parent row for that branch, allow one
  // left compaction so branch creation can land on the compact lane.
  for row_ix in 1..rows.len() {
    let (head, tail) = rows.split_at_mut(row_ix);
    let prev_row = &head[row_ix - 1];
    let row = &mut tail[0];

    let prev_merge_parent_branches = prev_row
      .merge_parent_lane_branches
      .iter()
      .map(|(_, branch_id)| *branch_id)
      .collect::<HashSet<_>>();
    let mut prev_down_lane_by_branch = HashMap::<usize, usize>::new();
    for (lane, branch_id) in prev_row.lane_branch_ids.iter().enumerate() {
      let Some(branch_id) = *branch_id else {
        continue;
      };
      if prev_row
        .segments
        .get(lane)
        .is_some_and(|segment| segment.up || segment.down)
      {
        prev_down_lane_by_branch.entry(branch_id).or_insert(lane);
      }
    }

    let mut prev_down_entries = prev_down_lane_by_branch.into_iter().collect::<Vec<_>>();
    prev_down_entries.sort_unstable_by(|(left_branch_id, left_lane), (right_branch_id, right_lane)| {
      left_lane.cmp(right_lane).then_with(|| {
        let left_priority = branch_total_commits
          .get(left_branch_id)
          .copied()
          .unwrap_or(0);
        let right_priority = branch_total_commits
          .get(right_branch_id)
          .copied()
          .unwrap_or(0);
        right_priority
          .cmp(&left_priority)
          .then_with(|| left_branch_id.cmp(right_branch_id))
      })
    });

    for (branch_id, prev_lane) in prev_down_entries {
      let branch_was_merge_parent = prev_merge_parent_branches.contains(&branch_id);
      let Some(cur_lane) = row
        .lane_branch_ids
        .iter()
        .enumerate()
        .find_map(|(lane, current_branch)| (*current_branch == Some(branch_id)).then_some(lane))
      else {
        continue;
      };
      if branch_was_merge_parent && cur_lane >= prev_lane {
        continue;
      }
      if cur_lane > prev_lane {
        let Some(existing_branch_id) = row
          .lane_branch_ids
          .get(prev_lane)
          .and_then(|branch| *branch)
        else {
          continue;
        };
        if existing_branch_id != branch_id
          && row
            .merge_parent_lane_branches
            .iter()
            .any(|(lane, merge_branch_id)| {
              *lane == prev_lane && *merge_branch_id == existing_branch_id
            })
          && !row
            .merge_parent_lane_branches
            .iter()
            .any(|(_, merge_branch_id)| *merge_branch_id == branch_id)
          && row
            .lane_branch_ids
            .get(cur_lane)
            .and_then(|branch| *branch)
            .is_some_and(|candidate| candidate == branch_id)
        {
          let current_priority = branch_total_commits.get(&branch_id).copied().unwrap_or(0);
          let existing_priority = branch_total_commits
            .get(&existing_branch_id)
            .copied()
            .unwrap_or(0);
          if current_priority > existing_priority {
            swap_branch_lanes(row, existing_branch_id, prev_lane, branch_id, cur_lane);
          }
        }
        continue;
      }
      if cur_lane >= prev_lane {
        continue;
      }
      if prev_lane >= row.segments.len() {
        row
          .segments
          .resize(prev_lane + 1, GraphLaneSegment::default());
        row.lane_branch_ids.resize(prev_lane + 1, None);
      }
      let prev_lane_has_branch = row
        .lane_branch_ids
        .get(prev_lane)
        .and_then(|branch| *branch)
        .is_some();
      let prev_lane_has_segment = row
        .segments
        .get(prev_lane)
        .is_some_and(|segment| segment.up || segment.down);
      // If the previous lane is truly empty on this row, keep the compacted
      // left position unless it would put a shorter branch left of a longer one.
      if !prev_lane_has_branch && !prev_lane_has_segment {
        let current_priority = branch_total_commits.get(&branch_id).copied().unwrap_or(0);
        let right_has_higher_priority =
          ((cur_lane + 1)..row.lane_branch_ids.len()).any(|lane_ix| {
            row
              .lane_branch_ids
              .get(lane_ix)
              .and_then(|branch| *branch)
              .is_some_and(|candidate_branch_id| {
                branch_total_commits
                  .get(&candidate_branch_id)
                  .copied()
                  .unwrap_or(0)
                  > current_priority
                })
          });
        let jump_distance = prev_lane.saturating_sub(cur_lane);
        if !branch_was_merge_parent && !right_has_higher_priority && jump_distance <= 1 {
          continue;
        }
      }
      let mut handled = false;
      if let Some(existing_branch_id) = row
        .lane_branch_ids
        .get(prev_lane)
        .and_then(|branch| *branch)
        && existing_branch_id != branch_id
      {
        let current_priority = branch_total_commits.get(&branch_id).copied().unwrap_or(0);
        let existing_priority = branch_total_commits
          .get(&existing_branch_id)
          .copied()
          .unwrap_or(0);
        let existing_is_merge_parent = row
          .merge_parent_lane_branches
          .iter()
          .any(|(lane, merge_branch_id)| {
            *lane == prev_lane && *merge_branch_id == existing_branch_id
          });
        let current_is_merge_parent = row
          .merge_parent_lane_branches
          .iter()
          .any(|(_, merge_branch_id)| *merge_branch_id == branch_id);
        if current_priority >= existing_priority {
          if existing_is_merge_parent && !current_is_merge_parent {
            let mut displaced_lane = prev_lane + 1;
            while row
              .lane_branch_ids
              .get(displaced_lane)
              .and_then(|branch| *branch)
              .is_some()
              || row
                .segments
                .get(displaced_lane)
                .is_some_and(|segment| segment.up || segment.down)
            {
              displaced_lane += 1;
            }
            move_branch_lane(row, existing_branch_id, prev_lane, displaced_lane);
            move_branch_lane(row, branch_id, cur_lane, prev_lane);
            normalize_row_lane_metadata(row);
          }
          continue;
        }
        // Preserve visual priority ordering: longer branches stay on the left.
        // If the current branch is at least as long as the one occupying its
        // previous lane, keep current placement instead of swapping right.
        if row
          .lane_branch_ids
          .get(cur_lane)
          .and_then(|branch| *branch)
          .is_some_and(|candidate| candidate == branch_id)
        {
          // Swap instead of displacing to keep a compact and visually stable order:
          // longer/older branch on the left, shorter branch on the right.
          swap_branch_lanes(row, existing_branch_id, prev_lane, branch_id, cur_lane);
          handled = true;
        } else {
          let mut displaced_lane = prev_lane + 1;
          while row
            .lane_branch_ids
            .get(displaced_lane)
            .and_then(|branch| *branch)
            .is_some()
            || row
              .segments
              .get(displaced_lane)
              .is_some_and(|segment| segment.up || segment.down)
          {
            displaced_lane += 1;
          }
          move_branch_lane(row, existing_branch_id, prev_lane, displaced_lane);
        }
      }
      if row
        .lane_branch_ids
        .get(prev_lane)
        .and_then(|branch| *branch)
        .is_some_and(|existing| existing != branch_id)
      {
        continue;
      }

      if !handled {
        move_branch_lane(row, branch_id, cur_lane, prev_lane);
      }
      normalize_row_lane_metadata(row);
    }
  }

  // Stabilization can leave new empty interior lanes after branches end; compact
  // again so side branches don't drift right when there is free space.
  for row in rows.iter_mut() {
    compact_row_lane_holes(row);
  }

  // Preserve relative ordering between continuing side branches across adjacent
  // rows. This avoids visual "lane flips" (A left of B, then B left of A) when
  // neither branch is splitting/merging on those rows.
  for row_ix in 1..rows.len() {
    let (head, tail) = rows.split_at_mut(row_ix);
    let prev_row = &head[row_ix - 1];
    let row = &mut tail[0];

    let prev_event_branches = prev_row
      .merge_parent_lane_branches
      .iter()
      .chain(prev_row.branch_child_lane_branches.iter())
      .map(|(_, branch_id)| *branch_id)
      .collect::<HashSet<_>>();
    let row_event_branches = row
      .merge_parent_lane_branches
      .iter()
      .chain(row.branch_child_lane_branches.iter())
      .map(|(_, branch_id)| *branch_id)
      .collect::<HashSet<_>>();

    // If compaction on the row right after a merge made a passive side branch
    // jump left of the merged branch, restore merged-branch precedence locally.
    if prev_row
      .merge_parent_lane_branches
      .iter()
      .any(|(_, branch_id)| *branch_id == row.commit_branch_id)
      && row.commit_lane > 1
    {
      let left_lane = row.commit_lane - 1;
      let left_branch_now = row
        .lane_branch_ids
        .get(left_lane)
        .and_then(|branch_id| *branch_id);
      let left_branch_prev = prev_row
        .lane_branch_ids
        .get(left_lane)
        .and_then(|branch_id| *branch_id);
      if let Some(left_branch_id) = left_branch_now
        && left_branch_prev != Some(left_branch_id)
        && !prev_event_branches.contains(&left_branch_id)
        && !row_event_branches.contains(&left_branch_id)
        && row
          .segments
          .get(left_lane)
          .is_some_and(|segment| segment.up || segment.down)
      {
        let merged_branch_id = row.commit_branch_id;
        let merged_lane = row.commit_lane;
        swap_branch_lanes(row, left_branch_id, left_lane, merged_branch_id, merged_lane);
        normalize_row_lane_metadata(row);
      }
    }

    loop {
      let mut prev_order = prev_row
        .lane_branch_ids
        .iter()
        .enumerate()
        .filter_map(|(lane, branch_id)| {
          branch_id.and_then(|branch_id| {
            (prev_row
              .segments
              .get(lane)
              .is_some_and(|segment| segment.down)
              && !prev_event_branches.contains(&branch_id)
              && !row_event_branches.contains(&branch_id))
            .then_some((lane, branch_id))
          })
        })
        .collect::<Vec<_>>();
      prev_order.sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));

      let row_lane_by_branch = row
        .lane_branch_ids
        .iter()
        .enumerate()
        .filter_map(|(lane, branch_id)| {
          branch_id.and_then(|branch_id| {
            (row
              .segments
              .get(lane)
              .is_some_and(|segment| segment.up)
              && !prev_event_branches.contains(&branch_id)
              && !row_event_branches.contains(&branch_id))
            .then_some((branch_id, lane))
          })
        })
        .collect::<HashMap<_, _>>();

      let mut swapped = false;
      'pairs: for left_ix in 0..prev_order.len() {
        for right_ix in (left_ix + 1)..prev_order.len() {
          let (_, left_branch_id) = prev_order[left_ix];
          let (_, right_branch_id) = prev_order[right_ix];
          let Some(left_lane_now) = row_lane_by_branch.get(&left_branch_id).copied() else {
            continue;
          };
          let Some(right_lane_now) = row_lane_by_branch.get(&right_branch_id).copied() else {
            continue;
          };
          if left_lane_now <= right_lane_now {
            continue;
          }
          if row
            .lane_branch_ids
            .get(left_lane_now)
            .and_then(|branch| *branch)
            != Some(left_branch_id)
            || row
              .lane_branch_ids
              .get(right_lane_now)
              .and_then(|branch| *branch)
              != Some(right_branch_id)
          {
            continue;
          }
          swap_branch_lanes(
            row,
            left_branch_id,
            left_lane_now,
            right_branch_id,
            right_lane_now,
          );
          normalize_row_lane_metadata(row);
          compact_row_lane_holes(row);
          swapped = true;
          break 'pairs;
        }
      }
      if !swapped {
        break;
      }
    }
  }

  // Lane stabilization can move child branches after split metadata has been
  // collected. Realign split lanes to final child commit lanes.
  let child_lane_by_parent_branch = rows
    .iter()
    .filter(|row| row.commit.parent_oids.len() == 1)
    .filter_map(|row| {
      row
        .commit
        .parent_oids
        .first()
        .map(|parent_oid| ((parent_oid.clone(), row.commit_branch_id), row.commit_lane))
    })
    .collect::<HashMap<_, _>>();

  for row in rows.iter_mut() {
    let parent_oid = row.commit.oid.clone();
    for (lane, branch_id) in row.branch_child_lane_branches.iter_mut() {
      if let Some(mapped_lane) = child_lane_by_parent_branch.get(&(parent_oid.clone(), *branch_id))
      {
        *lane = *mapped_lane;
      }
    }
    row
      .branch_child_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_child_lane_branches.dedup();
    row.branch_child_lanes = row
      .branch_child_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_child_lanes.sort_unstable();
    row.branch_child_lanes.dedup();
  }

  // Prefer the lane where the child branch is actually visible on the row right
  // above the parent. This keeps split curves connected after lane stabilization.
  for row_ix in 1..rows.len() {
    let row_above = rows[row_ix - 1].clone();
    let row = &mut rows[row_ix];
    for (lane, branch_id) in row.branch_child_lane_branches.iter_mut() {
      if let Some(visible_lane) = row_above
        .lane_branch_ids
        .iter()
        .enumerate()
        .find_map(|(candidate_lane, candidate_branch_id)| {
          (*candidate_branch_id == Some(*branch_id)
            && row_above
              .segments
              .get(candidate_lane)
              .is_some_and(|segment| segment.up || segment.down))
          .then_some(candidate_lane)
        })
      {
        *lane = visible_lane;
      } else {
        *lane = row_above
          .lane_branch_ids
          .iter()
          .enumerate()
          .find_map(|(candidate_lane, candidate_branch_id)| {
            (*candidate_branch_id == Some(*branch_id)).then_some(candidate_lane)
          })
          .unwrap_or(*lane);
      }
    }
    row
      .branch_child_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_child_lane_branches.dedup();
    row.branch_child_lanes = row
      .branch_child_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_child_lanes.sort_unstable();
    row.branch_child_lanes.dedup();
  }

  // Recompute pre-stub metadata from the final split lanes.
  for row in rows.iter_mut() {
    row.branch_pre_stubs.clear();
    row.branch_pre_stub_lane_branches.clear();
  }
  for parent_row_index in 0..rows.len() {
    let split_entries = rows[parent_row_index].branch_child_lane_branches.clone();
    for (child_lane, child_branch_id) in split_entries {
      if child_lane == rows[parent_row_index].commit_lane {
        continue;
      }
      let parent_has_up = rows[parent_row_index]
        .segments
        .get(child_lane)
        .map(|segment| segment.up)
        .unwrap_or(false);
      if !parent_has_up && parent_row_index > 0 {
        rows[parent_row_index - 1]
          .branch_pre_stub_lane_branches
          .push((child_lane, child_branch_id));
      }
    }
  }
  for row in rows.iter_mut() {
    row
      .branch_pre_stub_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_pre_stub_lane_branches.dedup();
    row.branch_pre_stubs = row
      .branch_pre_stub_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_pre_stubs.sort_unstable();
    row.branch_pre_stubs.dedup();
  }

  // Ensure lane vectors stay synchronized with lane+branch metadata tuples.
  for row in rows.iter_mut() {
    normalize_row_lane_metadata(row);
  }

  // Build transitions between adjacent rows when a branch lane changes.
  for row in rows.iter_mut() {
    row.lane_transitions.clear();
  }
  for row_ix in 0..rows.len().saturating_sub(1) {
    let (head, tail) = rows.split_at_mut(row_ix + 1);
    let row = &mut head[row_ix];
    let next_row = &tail[0];
    let mut next_up_lane_by_branch = HashMap::<usize, usize>::new();
    for (lane, branch_id) in next_row.lane_branch_ids.iter().enumerate() {
      let Some(branch_id) = *branch_id else {
        continue;
      };
      if next_row
        .segments
        .get(lane)
        .is_some_and(|segment| segment.up)
      {
        next_up_lane_by_branch.entry(branch_id).or_insert(lane);
      }
    }

    let mut transitions = Vec::<(usize, usize, usize)>::new();
    for (lane, branch_id) in row.lane_branch_ids.iter().enumerate() {
      let Some(branch_id) = *branch_id else {
        continue;
      };
      if !row.merge_parent_lane_branches.is_empty()
        && !row
          .merge_parent_lane_branches
          .iter()
          .any(|(_, merge_branch_id)| *merge_branch_id == branch_id)
      {
        continue;
      }
      if !row.segments.get(lane).is_some_and(|segment| segment.down) {
        continue;
      }
      let Some(next_lane) = next_up_lane_by_branch.get(&branch_id).copied() else {
        continue;
      };
      if lane == next_lane {
        continue;
      }
      if next_lane == row.commit_lane {
        continue;
      }
      transitions.push((lane, next_lane, branch_id));
    }
    transitions
      .sort_unstable_by_key(|(from_lane, to_lane, branch_id)| (*from_lane, *to_lane, *branch_id));
    transitions.dedup();
    row.lane_transitions = transitions;
  }

  let max_lane_count = rows.iter().map(|row| row.segments.len()).max().unwrap_or(0);
  if max_lane_count > 0 {
    for row in &mut rows {
      row
        .segments
        .resize(max_lane_count, GraphLaneSegment::default());
      row.lane_branch_ids.resize(max_lane_count, None);
    }
  }

  // If a lane already has a downward segment on the row before a split, a pre-stub
  // is redundant and creates a visible double-stroke artifact.
  for row in rows.iter_mut() {
    let lane_has_down = row.segments.iter().map(|segment| segment.down).collect::<Vec<_>>();
    row
      .branch_pre_stub_lane_branches
      .retain(|(lane, _)| !lane_has_down.get(*lane).copied().unwrap_or(false));
    row
      .branch_pre_stub_lane_branches
      .sort_unstable_by_key(|(lane, branch_id)| (*lane, *branch_id));
    row.branch_pre_stub_lane_branches.dedup();
    row.branch_pre_stubs = row
      .branch_pre_stub_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    row.branch_pre_stubs.sort_unstable();
    row.branch_pre_stubs.dedup();
  }

  rows
}

pub fn list_commit_changed_files(
  repo_root: &Path,
  commit_oid: &str,
) -> Result<Vec<CommitChangedFile>> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let commit = parse_commit(&repo, commit_oid)?;
  let commit_tree = commit.tree().context("read commit tree")?;
  let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

  let mut options = DiffOptions::new();
  let mut diff = repo
    .diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), Some(&mut options))
    .context("compute commit diff")?;
  enable_rename_detection(&mut diff);

  Ok(
    diff
      .deltas()
      .filter_map(|delta| commit_changed_file_from_delta(&delta))
      .collect(),
  )
}

pub fn load_commit_file_diff(
  repo_root: &Path,
  commit_oid: &str,
  rel_path: &Path,
) -> Result<CommitFileDiff> {
  let repo =
    Repository::open(repo_root).with_context(|| format!("open repo at {:?}", repo_root))?;
  let commit = parse_commit(&repo, commit_oid)?;
  let commit_tree = commit.tree().context("read commit tree")?;
  let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

  let mut options = DiffOptions::new();
  let mut diff = repo
    .diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), Some(&mut options))
    .context("compute commit diff")?;
  enable_rename_detection(&mut diff);

  let target_path = normalize_path(rel_path);
  for (delta_ix, delta) in diff.deltas().enumerate() {
    let Some(file) = commit_changed_file_from_delta(&delta) else {
      continue;
    };
    if normalize_path(&file.path) != target_path {
      continue;
    }

    let patch = patch_for_delta(&diff, delta_ix)?;
    let content = load_commit_file_content(&repo, &commit_tree, &file.path)?;
    return Ok(CommitFileDiff {
      file,
      patch,
      content,
    });
  }

  bail!("commit {commit_oid} does not change path {:?}", rel_path);
}

fn refs_by_oid(repo: &Repository) -> Result<BTreeMap<Oid, Vec<String>>> {
  let mut by_oid = BTreeMap::new();

  for branch in repo.branches(Some(BranchType::Local))? {
    let (branch, _) = branch?;
    let name = branch.name()?.unwrap_or("").to_string();
    if name.is_empty() {
      continue;
    }
    if let Some(oid) = branch.get().target() {
      insert_ref_label(&mut by_oid, oid, name);
    }
  }

  for branch in repo.branches(Some(BranchType::Remote))? {
    let (branch, _) = branch?;
    let name = branch.name()?.unwrap_or("").to_string();
    if name.is_empty() || name.ends_with("/HEAD") {
      continue;
    }
    if let Some(oid) = branch.get().target() {
      insert_ref_label(&mut by_oid, oid, name);
    }
  }

  Ok(by_oid)
}

fn head_target(repo: &Repository) -> Option<(Oid, String)> {
  let head = repo.head().ok()?;
  let oid = head
    .target()
    .or_else(|| head.peel_to_commit().ok().map(|commit| commit.id()))?;
  let label = if head.is_branch() {
    match head.shorthand() {
      Some(name) if !name.is_empty() => format!("HEAD -> {name}"),
      _ => "HEAD".to_string(),
    }
  } else {
    "HEAD".to_string()
  };

  Some((oid, label))
}

fn short_oid(oid: Oid) -> String {
  let value = oid.to_string();
  value.chars().take(7).collect()
}

fn insert_ref_label(by_oid: &mut BTreeMap<Oid, Vec<String>>, oid: Oid, label: String) {
  if label.is_empty() {
    return;
  }
  by_oid.entry(oid).or_default().push(label);
}

fn parse_commit<'repo>(repo: &'repo Repository, commit_oid: &str) -> Result<git2::Commit<'repo>> {
  let oid = Oid::from_str(commit_oid).with_context(|| format!("invalid oid {commit_oid}"))?;
  repo
    .find_commit(oid)
    .with_context(|| format!("find commit {commit_oid}"))
}

fn enable_rename_detection(diff: &mut Diff<'_>) {
  let mut find_options = DiffFindOptions::new();
  find_options.renames(true).copies(true);
  let _ = diff.find_similar(Some(&mut find_options));
}

fn commit_changed_file_from_delta(delta: &DiffDelta<'_>) -> Option<CommitChangedFile> {
  let kind = commit_change_kind(delta.status())?;
  let old_path = delta.old_file().path().map(Path::to_path_buf);
  let new_path = delta.new_file().path().map(Path::to_path_buf);
  let path = if kind == CommitFileChangeKind::Deleted {
    old_path.clone().or(new_path.clone())?
  } else {
    new_path.clone().or(old_path.clone())?
  };
  let old_path = old_path.filter(|old| old != &path);

  Some(CommitChangedFile {
    path,
    old_path,
    kind,
  })
}

fn commit_change_kind(status: Delta) -> Option<CommitFileChangeKind> {
  match status {
    Delta::Added => Some(CommitFileChangeKind::Added),
    Delta::Deleted => Some(CommitFileChangeKind::Deleted),
    Delta::Modified => Some(CommitFileChangeKind::Modified),
    Delta::Renamed => Some(CommitFileChangeKind::Renamed),
    Delta::Copied => Some(CommitFileChangeKind::Copied),
    Delta::Typechange => Some(CommitFileChangeKind::Typechange),
    Delta::Conflicted => Some(CommitFileChangeKind::Conflicted),
    Delta::Ignored | Delta::Unreadable | Delta::Untracked | Delta::Unmodified => None,
  }
}

fn patch_for_delta(diff: &Diff<'_>, delta_ix: usize) -> Result<String> {
  let Some(mut patch) = Patch::from_diff(diff, delta_ix).context("build patch from diff")? else {
    return Ok(String::new());
  };
  let patch_buf = patch.to_buf().context("serialize patch")?;
  Ok(String::from_utf8_lossy(patch_buf.as_ref()).into_owned())
}

fn load_commit_file_content(
  repo: &Repository,
  commit_tree: &Tree<'_>,
  rel_path: &Path,
) -> Result<String> {
  let Ok(entry) = commit_tree.get_path(rel_path) else {
    return Ok(String::new());
  };
  let blob = repo
    .find_blob(entry.id())
    .context("load blob from commit")?;
  Ok(String::from_utf8_lossy(blob.content()).into_owned())
}

fn normalize_path(path: &Path) -> String {
  path.to_string_lossy().replace('\\', "/")
}
#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashMap;

  fn make_commit(oid: &str, parents: &[&str]) -> CommitGraphNode {
    CommitGraphNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: Vec::new(),
    }
  }

  fn make_commit_with_refs(oid: &str, parents: &[&str], refs: &[&str]) -> CommitGraphNode {
    CommitGraphNode {
      oid: oid.to_string(),
      short_oid: oid.chars().take(7).collect(),
      summary: format!("commit-{oid}"),
      author: "author".to_string(),
      parent_oids: parents.iter().map(|parent| parent.to_string()).collect(),
      refs: refs.iter().map(|label| label.to_string()).collect(),
    }
  }

  fn row_has_internal_lane_hole(row: &CommitGraphRow) -> bool {
    let Some(last_occupied_lane) = row
      .lane_branch_ids
      .iter()
      .rposition(|branch_id| branch_id.is_some())
    else {
      return false;
    };
    row.lane_branch_ids[..=last_occupied_lane]
      .iter()
      .any(|branch_id| branch_id.is_none())
  }

  fn assert_row_lane_metadata_consistent(row: &CommitGraphRow) {
    let mut expected_merge_lanes = row
      .merge_parent_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    expected_merge_lanes.sort_unstable();
    expected_merge_lanes.dedup();
    assert_eq!(
      row.merge_parent_lanes, expected_merge_lanes,
      "merge_parent_lanes must match merge_parent_lane_branches for {}",
      row.commit.oid
    );

    let mut expected_child_lanes = row
      .branch_child_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    expected_child_lanes.sort_unstable();
    expected_child_lanes.dedup();
    assert_eq!(
      row.branch_child_lanes, expected_child_lanes,
      "branch_child_lanes must match branch_child_lane_branches for {}",
      row.commit.oid
    );

    let mut expected_pre_stub_lanes = row
      .branch_pre_stub_lane_branches
      .iter()
      .map(|(lane, _)| *lane)
      .collect::<Vec<_>>();
    expected_pre_stub_lanes.sort_unstable();
    expected_pre_stub_lanes.dedup();
    assert_eq!(
      row.branch_pre_stubs, expected_pre_stub_lanes,
      "branch_pre_stubs must match branch_pre_stub_lane_branches for {}",
      row.commit.oid
    );
  }

  #[test]
  fn build_graph_rows_linear_history_stays_on_single_lane() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);

    assert_eq!(rows[0].commit_lane, 0);
    assert!(rows[0].branch_child_lanes.is_empty());
    assert_eq!(rows[0].segments.len(), 1);
    assert!(rows[0].segments[0].up);
    assert!(rows[0].segments[0].down);

    assert_eq!(rows[2].commit_lane, 0);
    assert!(rows[2].segments[0].up);
    assert!(!rows[2].segments[0].down);
  }

  #[test]
  fn build_graph_rows_linear_history_keeps_single_branch_id() {
    let commits = vec![
      make_commit("c3", &["c2"]),
      make_commit("c2", &["c1"]),
      make_commit("c1", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);
    let anchor_branch_id = rows[0].commit_branch_id;

    for row in &rows {
      assert_eq!(row.commit_branch_id, anchor_branch_id);
      assert_eq!(
        row.lane_branch_ids.get(0).and_then(|branch_id| *branch_id),
        Some(anchor_branch_id)
      );
    }
  }

  #[test]
  fn build_graph_rows_merge_keeps_cross_lane_connection() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 4);

    assert_eq!(rows[0].commit_lane, 0);
    assert_eq!(rows[0].segments.len(), 2);
    assert_eq!(rows[0].merge_parent_lanes, vec![1]);

    assert_eq!(rows[2].commit_lane, 1);
    assert!(rows[2].branch_child_lanes.is_empty());
  }

  #[test]
  fn build_graph_rows_merge_row_carries_merged_branch_id_on_parent_lane() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 4);

    let merged_branch_id = rows[2].commit_branch_id;
    assert_eq!(
      rows[0]
        .lane_branch_ids
        .get(1)
        .and_then(|branch_id| *branch_id),
      Some(merged_branch_id)
    );
  }

  #[test]
  fn build_graph_rows_new_lane_starts_without_up_segment() {
    let commits = vec![
      make_commit("a", &["p"]),
      make_commit("x", &["y"]),
      make_commit("p", &[]),
      make_commit("y", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].commit_lane, 1);
    assert!(!rows[1].commit_lane_has_up);
    assert!(rows[1].segments[1].up);
    assert!(rows[1].segments[1].down);
  }

  #[test]
  fn build_graph_rows_keeps_lane_order_stable_when_gap_exists() {
    let commits = vec![
      make_commit("m", &["a", "b", "c"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("d", &["q"]),
      make_commit("c", &["p"]),
      make_commit("p", &[]),
      make_commit("q", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 7);

    assert_eq!(rows[2].commit_lane, 1);
    assert_eq!(rows[3].commit_lane, 1);
    assert!(!rows[3].commit_lane_has_up);
    assert_eq!(rows[4].commit_lane, 2);
  }

  #[test]
  fn build_graph_rows_is_deterministic_for_same_input() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows_a = build_commit_graph_rows(&commits);
    let rows_b = build_commit_graph_rows(&commits);

    let lanes_a = rows_a
      .iter()
      .map(|row| {
        let mut merge_parent_lanes = row.merge_parent_lanes.clone();
        merge_parent_lanes.sort_unstable();
        let mut branch_child_lanes = row.branch_child_lanes.clone();
        branch_child_lanes.sort_unstable();
        (
          row.commit_lane,
          merge_parent_lanes,
          branch_child_lanes,
          row.commit_lane_has_up,
        )
      })
      .collect::<Vec<_>>();
    let lanes_b = rows_b
      .iter()
      .map(|row| {
        let mut merge_parent_lanes = row.merge_parent_lanes.clone();
        merge_parent_lanes.sort_unstable();
        let mut branch_child_lanes = row.branch_child_lanes.clone();
        branch_child_lanes.sort_unstable();
        (
          row.commit_lane,
          merge_parent_lanes,
          branch_child_lanes,
          row.commit_lane_has_up,
        )
      })
      .collect::<Vec<_>>();

    assert_eq!(lanes_a, lanes_b);
  }

  #[test]
  fn build_graph_rows_prefers_head_first_parent_chain_as_anchor_lane() {
    let commits = vec![
      make_commit("merge", &["f2", "m2"]),
      make_commit("f2", &["f1"]),
      make_commit_with_refs("m2", &["m1"], &["origin/trunk"]),
      make_commit("f1", &["base"]),
      make_commit("m1", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let lane_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row.commit_lane))
      .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(lane_by_oid.get("f2"), Some(&0));
    assert_eq!(lane_by_oid.get("f1"), Some(&0));
  }

  #[test]
  fn build_graph_rows_marks_branch_creation_from_left_parent_to_right_lane() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].commit_lane, 0);
    assert_eq!(rows[2].branch_child_lanes, vec![1]);
  }

  #[test]
  fn build_graph_rows_branch_creation_adds_pre_stub_on_previous_row() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);
    // Split curve metadata lives on parent row.
    assert_eq!(rows[2].branch_child_lanes, vec![1]);
    // The pre-stub is painted on the row just before that parent row.
    assert_eq!(rows[1].branch_pre_stubs, vec![1]);
    assert!(rows[2].branch_pre_stubs.is_empty());
  }

  #[test]
  fn build_graph_rows_branch_creation_keeps_tail_through_intermediate_rows() {
    let commits = vec![
      make_commit("head", &["merge_auth"]),
      make_commit("merge_auth", &["merge_hotfix", "auth_3"]),
      make_commit("auth_3", &["auth_2"]),
      make_commit("auth_2", &["auth_1"]),
      make_commit("auth_1", &["base"]),
      make_commit("merge_hotfix", &["base", "hotfix_1"]),
      make_commit("hotfix_1", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    for row in rows.iter() {
      assert_row_lane_metadata_consistent(row);
    }
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();

    let auth_lane = row_by_oid.get("auth_1").copied().unwrap().commit_lane;
    let auth_branch_id = row_by_oid.get("auth_1").copied().unwrap().commit_branch_id;
    let auth_root_row = row_by_oid.get("auth_1").copied().unwrap();

    assert!(
      auth_root_row
        .segments
        .get(auth_lane)
        .is_some_and(|segment| segment.up && segment.down)
    );
    assert_eq!(
      auth_root_row
        .lane_branch_ids
        .get(auth_lane)
        .and_then(|branch| *branch),
      Some(auth_branch_id)
    );

    let merge_hotfix_row = row_by_oid.get("merge_hotfix").copied().unwrap();
    assert!(
      merge_hotfix_row
        .segments
        .get(auth_lane)
        .is_some_and(|segment| segment.up && segment.down)
    );
    assert_eq!(
      merge_hotfix_row
        .lane_branch_ids
        .get(auth_lane)
        .and_then(|branch| *branch),
      Some(auth_branch_id)
    );

    let hotfix_row = row_by_oid.get("hotfix_1").copied().unwrap();
    assert!(
      hotfix_row
        .segments
        .get(auth_lane)
        .is_some_and(|segment| segment.up && segment.down)
    );
    assert_eq!(
      hotfix_row
        .lane_branch_ids
        .get(auth_lane)
        .and_then(|branch| *branch),
      Some(auth_branch_id)
    );
  }

  #[test]
  fn build_graph_rows_branch_creation_carries_child_branch_id_on_right_lane() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);

    let child_branch_id = rows[1].commit_branch_id;
    assert_ne!(child_branch_id, rows[2].commit_branch_id);
    assert_eq!(
      rows[2]
        .branch_child_lane_branches
        .iter()
        .find_map(|(lane, branch_id)| (*lane == 1).then_some(*branch_id)),
      Some(child_branch_id)
    );
  }

  #[test]
  fn build_graph_rows_branch_creation_keeps_child_root_down_when_parent_is_adjacent() {
    let commits = vec![
      make_commit("tip", &["left"]),
      make_commit("right", &["left"]),
      make_commit("left", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 3);

    let right_row = &rows[1];
    let right_lane = right_row.commit_lane;
    assert!(
      right_row
        .segments
        .get(right_lane)
        .is_some_and(|segment| segment.down),
      "expected child root commit lane to continue down toward adjacent parent row"
    );
  }

  #[test]
  fn build_graph_rows_allows_compact_merge_split_on_same_placeholder_lane() {
    let commits = vec![
      make_commit("head", &["merge_api"]),
      make_commit("notif_2", &["merge_api"]),
      make_commit("merge_api", &["base", "api_2"]),
      make_commit("api_2", &["api_1"]),
      make_commit("api_1", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    for row in rows.iter() {
      assert_row_lane_metadata_consistent(row);
    }
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();

    let merge_api_row = row_by_oid.get("merge_api").copied().unwrap();
    let (merge_lane, merge_branch_id) = merge_api_row
      .merge_parent_lane_branches
      .first()
      .copied()
      .expect("merge lane should exist");
    let (split_lane, split_branch_id) = merge_api_row
      .branch_child_lane_branches
      .first()
      .copied()
      .expect("split lane should exist");

    assert_eq!(merge_lane, split_lane);
    assert_ne!(merge_branch_id, split_branch_id);
  }

  #[test]
  fn build_graph_rows_compacts_placeholder_split_lane_without_extra_gap() {
    let commits = vec![
      make_commit("head", &["merge_api"]),
      make_commit("notif_2", &["merge_api"]),
      make_commit("merge_api", &["base", "api_2"]),
      make_commit("api_2", &["api_1"]),
      make_commit("api_1", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();
    let merge_api_row = row_by_oid.get("merge_api").copied().unwrap();

    let merge_lane = merge_api_row
      .merge_parent_lane_branches
      .first()
      .map(|(lane, _)| *lane)
      .expect("merge lane should exist");
    let split_lane = merge_api_row
      .branch_child_lane_branches
      .first()
      .map(|(lane, _)| *lane)
      .expect("split lane should exist");

    assert_eq!(merge_lane, split_lane);
    assert!(!row_has_internal_lane_hole(merge_api_row));
  }

  #[test]
  fn build_graph_rows_prefers_longer_merged_branch_on_left_lane() {
    let commits = vec![
      make_commit("m", &["base", "short_tip", "long_tip"]),
      make_commit("short_tip", &["base"]),
      make_commit("long_tip", &["long_mid"]),
      make_commit("long_mid", &["long_root"]),
      make_commit("long_root", &["base"]),
      make_commit("base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();
    let merge_row = row_by_oid.get("m").copied().unwrap();
    let short_branch_id = row_by_oid
      .get("short_tip")
      .copied()
      .unwrap()
      .commit_branch_id;
    let long_branch_id = row_by_oid
      .get("long_tip")
      .copied()
      .unwrap()
      .commit_branch_id;

    let short_lane = merge_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == short_branch_id).then_some(*lane))
      .expect("short branch lane should exist");
    let long_lane = merge_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == long_branch_id).then_some(*lane))
      .expect("long branch lane should exist");

    assert!(long_lane < short_lane);
  }

  #[test]
  fn build_graph_rows_prefers_longer_active_branch_on_left_lane_in_playground_shape() {
    // Reproduces the feature/i18n playground section:
    // merge notifications sits above merge api on the first-parent chain.
    let commits = vec![
      make_commit("i18n_tip", &["i18n_prev"]),
      make_commit("i18n_prev", &["merge_main_back"]),
      make_commit("merge_main_back", &["merge_notifications", "main_sync"]),
      make_commit("main_sync", &["main_base"]),
      make_commit("main_base", &[]),
      make_commit("merge_notifications", &["merge_api", "notif_2"]),
      make_commit("notif_2", &["notif_1"]),
      make_commit("notif_1", &["merge_dashboard"]),
      make_commit("merge_api", &["merge_dashboard", "api_2"]),
      make_commit("api_2", &["api_1"]),
      make_commit("api_1", &["merge_dashboard"]),
      make_commit("merge_dashboard", &["develop_base"]),
      make_commit("develop_base", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();

    let merge_api_row = row_by_oid.get("merge_api").copied().unwrap();
    let notif_branch_id = row_by_oid.get("notif_2").copied().unwrap().commit_branch_id;
    let api_branch_id = row_by_oid.get("api_2").copied().unwrap().commit_branch_id;
    let notif_lane = merge_api_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(notif_branch_id)).then_some(lane))
      .expect("notif branch should be visible on merge_api row");
    let api_lane = merge_api_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == api_branch_id).then_some(*lane))
      .expect("api merge parent lane should be visible on merge_api row");

    assert!(notif_lane < api_lane);
  }

  #[test]
  fn build_graph_rows_playground_feature_i18n_keeps_notifications_lane_left_of_api_lane() {
    // Commit order mirrors `git log --topo-order` from /Users/joris/workspace/git-playground
    // on branch feature/i18n around the develop merges.
    let commits = vec![
      make_commit("i18n_4", &["i18n_3"]),
      make_commit("i18n_3", &["i18n_2"]),
      make_commit("i18n_2", &["i18n_1"]),
      make_commit("i18n_1", &["merge_main_back"]),
      make_commit(
        "merge_main_back",
        &["merge_notifications", "merge_release_into_main"],
      ),
      make_commit(
        "merge_release_into_main",
        &["merge_auth_into_main", "release_finalize"],
      ),
      make_commit("release_finalize", &["release_docs"]),
      make_commit("release_docs", &["release_rc1"]),
      make_commit("release_rc1", &["merge_notifications"]),
      make_commit("merge_notifications", &["merge_api", "notif_2"]),
      make_commit("notif_2", &["notif_1"]),
      make_commit("notif_1", &["merge_dashboard"]),
      make_commit("merge_api", &["merge_dashboard", "api_2"]),
      make_commit("api_2", &["api_1"]),
      make_commit("api_1", &["merge_dashboard"]),
      make_commit(
        "merge_dashboard",
        &["develop_dev_bump", "dashboard_after_merge"],
      ),
      make_commit("develop_dev_bump", &["merge_auth_into_main"]),
      make_commit(
        "merge_auth_into_main",
        &["merge_hotfix_into_main", "auth_3"],
      ),
      make_commit("merge_hotfix_into_main", &["main_cleanup", "hotfix_1"]),
      make_commit("hotfix_1", &["main_cleanup"]),
      make_commit("dashboard_after_merge", &["dashboard_merge_widgets"]),
      make_commit("dashboard_merge_widgets", &["dashboard_style", "widgets_2"]),
      make_commit("widgets_2", &["widgets_1"]),
      make_commit("widgets_1", &["dashboard_style"]),
      make_commit("dashboard_style", &["dashboard_1"]),
      make_commit("dashboard_1", &["main_cleanup"]),
      make_commit("auth_3", &["auth_2"]),
      make_commit("auth_2", &["auth_1"]),
      make_commit("auth_1", &["main_cleanup"]),
      make_commit("main_cleanup", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();

    let merge_api_row = row_by_oid.get("merge_api").copied().unwrap();
    let notif_row = row_by_oid.get("notif_2").copied().unwrap();
    let notif_prev_row = row_by_oid.get("notif_1").copied().unwrap();
    let api_2_row = row_by_oid.get("api_2").copied().unwrap();
    let api_1_row = row_by_oid.get("api_1").copied().unwrap();
    let merge_release_row = row_by_oid.get("merge_release_into_main").copied().unwrap();
    let dashboard_after_merge_row = row_by_oid.get("dashboard_after_merge").copied().unwrap();
    let merge_dashboard_row = row_by_oid.get("merge_dashboard").copied().unwrap();
    let merge_auth_row = row_by_oid.get("merge_auth_into_main").copied().unwrap();
    let merge_hotfix_row = row_by_oid.get("merge_hotfix_into_main").copied().unwrap();
    let auth_1_row = row_by_oid.get("auth_1").copied().unwrap();
    let main_cleanup_row = row_by_oid.get("main_cleanup").copied().unwrap();
    let notif_branch_id = row_by_oid.get("notif_2").copied().unwrap().commit_branch_id;
    let api_branch_id = row_by_oid.get("api_2").copied().unwrap().commit_branch_id;
    let dashboard_branch_id = dashboard_after_merge_row.commit_branch_id;
    let release_branch_id = row_by_oid
      .get("release_finalize")
      .copied()
      .unwrap()
      .commit_branch_id;
    let notif_lane = merge_api_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(notif_branch_id)).then_some(lane))
      .expect("notif branch should be visible on merge_api row");
    let api_lane = merge_api_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == api_branch_id).then_some(*lane))
      .expect("api merge parent lane should be visible on merge_api row");

    assert!(
      notif_lane < api_lane,
      "expected notifications lane ({notif_lane}) to stay left of api lane ({api_lane}) on merge_api row"
    );
    assert_eq!(
      api_lane,
      notif_lane + 1,
      "expected api lane ({api_lane}) immediately on the right of notifications lane ({notif_lane}) on merge_api row"
    );
    assert!(!row_has_internal_lane_hole(notif_row));
    assert!(!row_has_internal_lane_hole(merge_api_row));
    let notif_lane = notif_row.commit_lane;
    assert_eq!(
      notif_prev_row
        .lane_branch_ids
        .get(notif_lane)
        .and_then(|branch_id| *branch_id),
      Some(notif_branch_id),
      "expected notifications branch to occupy its lane on notif_1 row"
    );
    assert_eq!(
      api_2_row
        .lane_branch_ids
        .get(notif_lane)
        .and_then(|branch_id| *branch_id),
      Some(notif_branch_id),
      "expected notifications branch lane to remain visible while api branch is rendered"
    );
    assert_eq!(
      api_1_row
        .lane_branch_ids
        .get(notif_lane)
        .and_then(|branch_id| *branch_id),
      Some(notif_branch_id),
      "expected notifications branch lane continuity through api branch rows"
    );
    let dashboard_start_lane = dashboard_after_merge_row.commit_lane;
    assert!(
      dashboard_start_lane >= 1,
      "expected dashboard branch to stay on a side lane after merge_dashboard",
    );
    let dashboard_merge_parent_lane = merge_dashboard_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == dashboard_branch_id).then_some(*lane))
      .expect("dashboard merge-parent lane should be visible on merge_dashboard row");
    let notif_child_lane_on_merge_dashboard = merge_dashboard_row
      .branch_child_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == notif_branch_id).then_some(*lane))
      .expect("notifications child lane should be visible on merge_dashboard row");
    assert!(
      dashboard_merge_parent_lane <= notif_child_lane_on_merge_dashboard,
      "expected longer dashboard merge-parent lane ({dashboard_merge_parent_lane}) to stay left of or share compact placeholder lane with shorter notifications child lane ({notif_child_lane_on_merge_dashboard}) on merge_dashboard row",
    );
    let dashboard_lane_on_merge_auth = merge_auth_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(dashboard_branch_id)).then_some(lane))
      .expect("dashboard branch should be visible on merge_auth row");
    let main_branch_id = merge_release_row.commit_branch_id;
    let main_lane_on_merge_auth = merge_auth_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(main_branch_id)).then_some(lane))
      .expect("main branch should still be visible on merge_auth row");
    assert!(
      merge_auth_row
        .segments
        .get(main_lane_on_merge_auth)
        .is_some_and(|segment| segment.up && !segment.down),
      "expected main lane ({main_lane_on_merge_auth}) to terminate on merge_auth row without a downward continuation",
    );
    let auth_3_row = row_by_oid.get("auth_3").copied().unwrap();
    let auth_branch_id = auth_3_row.commit_branch_id;
    assert_ne!(
      auth_3_row
        .lane_branch_ids
        .get(main_lane_on_merge_auth)
        .and_then(|branch_id| *branch_id),
      Some(main_branch_id),
      "expected main branch to stop on merge_auth row and not continue on lane {main_lane_on_merge_auth} below it",
    );
    let hotfix_row = row_by_oid.get("hotfix_1").copied().unwrap();
    let hotfix_branch_id = hotfix_row.commit_branch_id;
    let auth_lane_on_merge_auth = merge_auth_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == auth_branch_id).then_some(*lane))
      .expect("auth merge parent lane should be visible on merge_auth row");
    assert!(
      dashboard_lane_on_merge_auth < auth_lane_on_merge_auth,
      "expected dashboard branch lane ({dashboard_lane_on_merge_auth}) to stay left of auth merge-parent lane ({auth_lane_on_merge_auth}) on merge_auth row",
    );
    assert!(
      dashboard_lane_on_merge_auth == dashboard_start_lane
        || dashboard_lane_on_merge_auth == dashboard_start_lane + 1,
      "expected dashboard branch on merge_auth row to either stay on its start lane ({dashboard_start_lane}) or shift right by one ({})",
      dashboard_start_lane + 1
    );
    assert!(
      auth_3_row.commit_lane >= 1,
      "expected first auth commit to stay on a side lane on auth branch rows"
    );
    let dashboard_lane_on_auth_3 = auth_3_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(dashboard_branch_id)).then_some(lane))
      .expect("dashboard branch should remain visible on auth_3 row");
    assert!(
      dashboard_lane_on_auth_3 < auth_3_row.commit_lane,
      "expected shorter auth branch lane ({}) to stay right of longer dashboard lane ({dashboard_lane_on_auth_3}) on auth_3 row",
      auth_3_row.commit_lane
    );
    let dashboard_lane_on_merge_hotfix = merge_hotfix_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(dashboard_branch_id)).then_some(lane))
      .expect("dashboard branch should be visible on merge_hotfix row");
    assert!(
      dashboard_lane_on_merge_hotfix <= dashboard_lane_on_merge_auth,
      "expected dashboard branch to stay on the same lane or compact left on merge_hotfix row",
    );
    let auth_lane_on_merge_hotfix = merge_hotfix_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(auth_branch_id)).then_some(lane))
      .expect("auth branch should be visible on merge_hotfix row");
    assert!(
      merge_auth_row
        .lane_transitions
        .iter()
        .all(|(_, _, branch_id)| *branch_id == auth_branch_id),
      "expected merge_auth row transitions to only contain the merged auth branch",
    );
    let hotfix_lane_on_merge_hotfix = merge_hotfix_row
      .merge_parent_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == hotfix_branch_id).then_some(*lane))
      .expect("hotfix merge-parent lane should be visible on merge_hotfix row");
    assert!(
      auth_lane_on_merge_hotfix < hotfix_lane_on_merge_hotfix,
      "expected shorter hotfix merge-parent lane ({hotfix_lane_on_merge_hotfix}) to stay right of longer auth lane ({auth_lane_on_merge_hotfix}) on merge_hotfix row",
    );
    let auth_lane_on_hotfix_row = hotfix_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(auth_branch_id)).then_some(lane))
      .expect("hotfix row should still show auth branch lane");
    assert_eq!(
      auth_lane_on_hotfix_row, auth_lane_on_merge_hotfix,
      "expected auth branch lane to stay stable from merge_hotfix row to hotfix commit row",
    );
    assert!(
      !merge_auth_row
        .branch_child_lane_branches
        .iter()
        .any(|(_, branch_id)| *branch_id == release_branch_id),
      "merge_auth row must not expose synthetic release branch creation curve",
    );
    assert!(
      !hotfix_row
        .branch_pre_stub_lane_branches
        .iter()
        .any(|(_, branch_id)| *branch_id == auth_branch_id),
      "expected hotfix row to avoid redundant auth pre-stub when lane already has down continuity",
    );
    let auth_split_lane = main_cleanup_row
      .branch_child_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == auth_branch_id).then_some(*lane))
      .expect("main_cleanup should expose auth split lane");
    assert_eq!(
      auth_split_lane, auth_lane_on_hotfix_row,
      "expected auth split lane on main_cleanup to match auth lane visible right above the parent row",
    );
    let hotfix_split_lane = main_cleanup_row
      .branch_child_lane_branches
      .iter()
      .find_map(|(lane, branch_id)| (*branch_id == hotfix_branch_id).then_some(*lane))
      .expect("main_cleanup should expose hotfix split lane");
    let hotfix_lane_on_row_above = auth_1_row
      .lane_branch_ids
      .iter()
      .enumerate()
      .find_map(|(lane, branch_id)| (*branch_id == Some(hotfix_branch_id)).then_some(lane));
    assert_eq!(
      hotfix_split_lane,
      hotfix_lane_on_row_above.unwrap_or(hotfix_row.commit_lane),
      "expected hotfix split lane on main_cleanup to match hotfix lane visible right above the parent row",
    );

    let rows_second = build_commit_graph_rows(&commits);
    let snapshot = |graph_rows: &[CommitGraphRow]| {
      graph_rows
        .iter()
        .map(|row| {
          (
            row.commit.oid.clone(),
            row.commit_lane,
            row.lane_branch_ids.clone(),
            row.merge_parent_lane_branches.clone(),
            row.branch_child_lane_branches.clone(),
            row.lane_transitions.clone(),
          )
        })
        .collect::<Vec<_>>()
    };
    assert_eq!(
      snapshot(&rows),
      snapshot(&rows_second),
      "playground graph rows must be deterministic across rebuilds",
    );
  }

  #[test]
  fn build_graph_rows_playground_feature_i18n_keeps_release_branch_creation_curve_metadata() {
    let commits = vec![
      make_commit("i18n_4", &["i18n_3"]),
      make_commit("i18n_3", &["i18n_2"]),
      make_commit("i18n_2", &["i18n_1"]),
      make_commit("i18n_1", &["merge_main_back"]),
      make_commit(
        "merge_main_back",
        &["merge_notifications", "merge_release_into_main"],
      ),
      make_commit(
        "merge_release_into_main",
        &["merge_auth_into_main", "release_finalize"],
      ),
      make_commit("release_finalize", &["release_docs"]),
      make_commit("release_docs", &["release_rc1"]),
      make_commit("release_rc1", &["merge_notifications"]),
      make_commit("merge_notifications", &["merge_api", "notif_2"]),
      make_commit("notif_2", &["notif_1"]),
      make_commit("notif_1", &["merge_dashboard"]),
      make_commit("merge_api", &["merge_dashboard", "api_2"]),
      make_commit("api_2", &["api_1"]),
      make_commit("api_1", &["merge_dashboard"]),
      make_commit(
        "merge_dashboard",
        &["develop_dev_bump", "dashboard_after_merge"],
      ),
      make_commit("develop_dev_bump", &["merge_auth_into_main"]),
      make_commit(
        "merge_auth_into_main",
        &["merge_hotfix_into_main", "auth_3"],
      ),
      make_commit("merge_hotfix_into_main", &["main_cleanup", "hotfix_1"]),
      make_commit("hotfix_1", &["main_cleanup"]),
      make_commit("dashboard_after_merge", &["dashboard_merge_widgets"]),
      make_commit("dashboard_merge_widgets", &["dashboard_style", "widgets_2"]),
      make_commit("widgets_2", &["widgets_1"]),
      make_commit("widgets_1", &["dashboard_style"]),
      make_commit("dashboard_style", &["dashboard_1"]),
      make_commit("dashboard_1", &["main_cleanup"]),
      make_commit("auth_3", &["auth_2"]),
      make_commit("auth_2", &["auth_1"]),
      make_commit("auth_1", &["main_cleanup"]),
      make_commit("main_cleanup", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    let row_by_oid = rows
      .iter()
      .map(|row| (row.commit.oid.as_str(), row))
      .collect::<HashMap<_, _>>();

    let release_row = row_by_oid.get("release_rc1").copied().unwrap();
    let merge_notifications_row = row_by_oid.get("merge_notifications").copied().unwrap();

    assert_eq!(
      merge_notifications_row
        .branch_child_lane_branches
        .iter()
        .find_map(|(lane, branch_id)| {
          (*branch_id == release_row.commit_branch_id).then_some(*lane)
        }),
      Some(release_row.commit_lane),
      "expected merge_notifications row to expose a split curve lane for release branch creation",
    );
    assert!(
      release_row
        .branch_pre_stub_lane_branches
        .iter()
        .any(|(lane, branch_id)| {
          *lane == release_row.commit_lane && *branch_id == release_row.commit_branch_id
        }),
      "expected release row to expose pre-stub metadata so split curve visually connects",
    );
  }

  #[test]
  fn build_graph_rows_merge_row_has_merge_curve_without_split_metadata() {
    let commits = vec![
      make_commit("m", &["a", "b"]),
      make_commit("a", &["p"]),
      make_commit("b", &["p"]),
      make_commit("p", &[]),
    ];

    let rows = build_commit_graph_rows(&commits);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].merge_parent_lanes, vec![1]);
    assert!(rows[0].branch_child_lanes.is_empty());
    assert!(rows[0].branch_pre_stubs.is_empty());
  }
}
