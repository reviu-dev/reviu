use std::{
  collections::{HashMap, HashSet},
  ops::Range,
  sync::Arc,
};

use blake3::Hasher;
use git::{DiffHunk, DiffLine, DiffLineKind, FileDiff};

const GAP_THRESHOLD_LINES: usize = 6;
pub const NO_NEWLINE_MARKER_TEXT: &str = "\\ No newline at end of file";
const REVIEW_COMMENT_COLLAPSED_LINES: usize = 2;
pub const REVIEW_COMMENT_HEADER_HEIGHT_LINES: f32 = 2.0;
pub const REVIEW_COMMENT_CARD_BORDER_PX: f32 = 1.0;
pub const REVIEW_COMMENT_CARD_PADDING_X_PX: f32 = 12.0;
pub const REVIEW_COMMENT_CARD_CONTENT_GAP_PX: f32 = 4.0;
pub const REVIEW_COMMENT_FIRST_MESSAGE_BOTTOM_PADDING_PX: f32 = 16.0;
pub const REVIEW_COMMENT_REPLY_VERTICAL_PADDING_PX: f32 = 20.0;
pub const REVIEW_COMMENT_REPLY_HEADER_BODY_GAP_PX: f32 = 12.0;
pub const REVIEW_COMMENT_REPLY_BORDER_TOP_PX: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkState {
  Staged,
  Unstaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
  Context,
  Added,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCommentSide {
  Left,
  Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCommentBackground {
  Added,
  Removed,
}

#[derive(Clone, Debug)]
pub struct ReviewComment {
  pub id: u64,
  pub in_reply_to_id: Option<u64>,
  pub line: usize,
  pub side: ReviewCommentSide,
  pub author: Arc<str>,
  pub avatar_url: Option<Arc<str>>,
  pub line_label: Option<Arc<str>>,
  pub body: Arc<str>,
  pub created_at: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GapId {
  pub start: usize,
  pub end: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GapReveal {
  pub head: usize,
  pub tail: usize,
}

#[derive(Clone, Debug)]
pub enum DisplayLine {
  Doc {
    doc_line: usize,
    old_line: Option<usize>,
    change: Option<ChangeKind>,
    hunk: Option<HunkState>,
    group_id: Option<Arc<str>>,
    secondary: bool,
  },
  Modified {
    old_text: Arc<str>,
    doc_line: usize,
    old_line: usize,
    hunk: HunkState,
    group_id: Option<Arc<str>>,
    secondary: bool,
  },
  Removed {
    text: Arc<str>,
    anchor_line: usize,
    old_line: usize,
    hunk: HunkState,
    group_id: Option<Arc<str>>,
    secondary: bool,
  },
  Gap {
    id: GapId,
    hidden_range: Range<usize>,
  },
  NoNewline {
    hunk: Option<HunkState>,
    group_id: Option<Arc<str>>,
    secondary: bool,
  },
  ReviewComment {
    id: u64,
    side: ReviewCommentSide,
    group_id: Option<Arc<str>>,
    background: Option<ReviewCommentBackground>,
    secondary: bool,
    text: Arc<str>,
    is_header: bool,
  },
}

#[derive(Clone, Debug)]
pub struct Projection {
  pub lines: Vec<DisplayLine>,
  pub display_to_doc: Vec<Option<usize>>,
  pub doc_to_display: Vec<Option<usize>>,
  pub visible_doc_lines: Vec<usize>,
  pub start_gap: Option<GapId>,
  pub end_gap: Option<GapId>,
  pub groups: HashMap<Arc<str>, ChangeGroup>,
}

#[derive(Clone, Debug)]
pub struct ChangeGroup {
  pub id: Arc<str>,
  pub state: HunkState,
  pub hunk: DiffHunk,
  pub signature: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LineKeyKind {
  Add,
  Remove,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LineKey {
  kind: LineKeyKind,
  line: usize,
  content: Arc<str>,
  occurrence: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LineOccurrenceKey {
  kind: LineKeyKind,
  line: usize,
  content: Arc<str>,
}

#[derive(Clone, Default)]
struct LineKeyBuilder {
  occurrences: HashMap<LineOccurrenceKey, u32>,
}

impl LineKeyBuilder {
  fn line_key(&mut self, kind: LineKeyKind, line: usize, content: &str) -> LineKey {
    let content: Arc<str> = Arc::from(content);
    let occurrence_key = LineOccurrenceKey {
      kind,
      line,
      content: content.clone(),
    };
    let entry = self.occurrences.entry(occurrence_key).or_insert(0);
    let occurrence = *entry;
    *entry = entry.saturating_add(1);
    LineKey {
      kind,
      line,
      content,
      occurrence,
    }
  }
}

struct GroupBuilder {
  start_old_line: usize,
  start_new_line: usize,
  old_lines: usize,
  new_lines: usize,
  lines: Vec<DiffLine>,
  keys: Vec<LineKey>,
}

struct PendingStagedGroup {
  builder: GroupBuilder,
  display_indices: Vec<usize>,
  signature: Arc<str>,
}

impl Projection {
  pub fn from_diffs(
    doc_line_count: usize,
    uncommitted: &FileDiff,
    unstaged: &FileDiff,
    staged: &FileDiff,
    expanded_gaps: &HashMap<GapId, GapReveal>,
    align_modified: bool,
  ) -> Self {
    let (mut groups, unstaged_line_to_group) = collect_groups(unstaged, HunkState::Unstaged);
    let (staged_groups, _) = collect_groups(staged, HunkState::Staged);
    let mut staged_groups_by_signature: HashMap<Arc<str>, Vec<Arc<str>>> = HashMap::new();
    for (id, group) in &staged_groups {
      staged_groups_by_signature
        .entry(group.signature.clone())
        .or_default()
        .push(id.clone());
    }

    let mut hunks = Vec::new();
    let mut key_builder = LineKeyBuilder::default();
    for hunk in &uncommitted.hunks {
      let display = if align_modified {
        build_hunk_display_split(hunk, &mut key_builder, &unstaged_line_to_group)
      } else {
        build_hunk_display_inline(hunk, &mut key_builder, &unstaged_line_to_group)
      };
      hunks.push(display);
    }

    if hunks.is_empty() {
      return Projection::full(doc_line_count);
    }

    hunks.sort_by_key(|hunk| hunk.sort_key());

    let mut lines = Vec::new();
    let mut pending_staged = Vec::new();
    let mut last_visible_doc_line: Option<usize> = None;
    let mut start_gap: Option<GapId> = None;
    let mut end_gap: Option<GapId> = None;

    let mut push_gap = |gap_start: usize,
                        gap_end: usize,
                        reveal: GapReveal,
                        old_offset: isize,
                        lines: &mut Vec<DisplayLine>| {
      if gap_end <= gap_start {
        return;
      }

      let gap_len = gap_end - gap_start;
      let gap_id = GapId {
        start: gap_start,
        end: gap_end,
      };
      let skip_marker = gap_start == 0 || gap_end == doc_line_count;

      if gap_len <= GAP_THRESHOLD_LINES {
        for doc_line in gap_start..gap_end {
          let old_line = (doc_line as isize + old_offset).max(0) as usize;
          lines.push(DisplayLine::Doc {
            doc_line,
            old_line: Some(old_line),
            change: None,
            hunk: None,
            group_id: None,
            secondary: false,
          });
        }
        return;
      }

      let head = reveal.head.min(gap_len);
      let tail = reveal.tail.min(gap_len.saturating_sub(head));
      let head_end = gap_start.saturating_add(head).min(gap_end);
      let tail_start = gap_end.saturating_sub(tail);

      if head_end >= tail_start {
        for doc_line in gap_start..gap_end {
          let old_line = (doc_line as isize + old_offset).max(0) as usize;
          lines.push(DisplayLine::Doc {
            doc_line,
            old_line: Some(old_line),
            change: None,
            hunk: None,
            group_id: None,
            secondary: false,
          });
        }
        return;
      }

      for doc_line in gap_start..head_end {
        let old_line = (doc_line as isize + old_offset).max(0) as usize;
        lines.push(DisplayLine::Doc {
          doc_line,
          old_line: Some(old_line),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        });
      }

      let remaining = tail_start.saturating_sub(head_end);
      if remaining > GAP_THRESHOLD_LINES {
        if skip_marker {
          if gap_start == 0 {
            start_gap = Some(gap_id);
          }
          if gap_end == doc_line_count {
            end_gap = Some(gap_id);
          }
        } else {
          lines.push(DisplayLine::Gap {
            id: gap_id,
            hidden_range: head_end..tail_start,
          });
        }
      } else {
        for doc_line in head_end..tail_start {
          let old_line = (doc_line as isize + old_offset).max(0) as usize;
          lines.push(DisplayLine::Doc {
            doc_line,
            old_line: Some(old_line),
            change: None,
            hunk: None,
            group_id: None,
            secondary: false,
          });
        }
      }

      for doc_line in tail_start..gap_end {
        let old_line = (doc_line as isize + old_offset).max(0) as usize;
        lines.push(DisplayLine::Doc {
          doc_line,
          old_line: Some(old_line),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        });
      }
    };

    let mut old_line_offset: isize = 0;

    for hunk in hunks {
      let anchor_line = hunk
        .first_doc_line
        .or(Some(hunk.start_line))
        .unwrap_or(0)
        .min(doc_line_count);

      let gap_start = last_visible_doc_line.map(|line| line + 1).unwrap_or(0);
      let gap_end = anchor_line.min(doc_line_count);

      let reveal = expanded_gaps
        .get(&GapId {
          start: gap_start,
          end: gap_end,
        })
        .copied()
        .unwrap_or_default();
      push_gap(gap_start, gap_end, reveal, old_line_offset, &mut lines);

      let offset = lines.len();
      for mut pending in hunk.pending_groups {
        for index in &mut pending.display_indices {
          *index = index.saturating_add(offset);
        }
        pending_staged.push(pending);
      }

      lines.extend(hunk.lines);

      old_line_offset = old_line_offset.saturating_add(hunk.delta);

      if let Some(last_line) = hunk.last_doc_line {
        last_visible_doc_line = Some(last_line);
      } else if anchor_line > 0 {
        last_visible_doc_line = Some(anchor_line.saturating_sub(1));
      }
    }

    if let Some(last_line) = last_visible_doc_line {
      let gap_start = last_line.saturating_add(1);
      let gap_end = doc_line_count;
      let reveal = expanded_gaps
        .get(&GapId {
          start: gap_start,
          end: gap_end,
        })
        .copied()
        .unwrap_or_default();
      push_gap(gap_start, gap_end, reveal, old_line_offset, &mut lines);
    }

    for pending in pending_staged {
      let mut matched_group_id = None;
      if let Some(ids) = staged_groups_by_signature.get_mut(&pending.signature) {
        matched_group_id = ids.pop();
      }

      if let Some(group_id) = matched_group_id
        && let Some(group) = staged_groups.get(&group_id)
      {
        groups.insert(group_id.clone(), group.clone());
        assign_group_id(&mut lines, &pending.display_indices, &group_id);
        continue;
      }

      let group_id = group_id_for_keys(&pending.builder.keys);
      let signature = pending.signature.clone();
      let hunk = DiffHunk {
        id: group_id.to_string(),
        old_start: pending.builder.start_old_line,
        old_lines: pending.builder.old_lines,
        new_start: pending.builder.start_new_line,
        new_lines: pending.builder.new_lines,
        lines: pending.builder.lines,
      };
      let group = ChangeGroup {
        id: group_id.clone(),
        state: HunkState::Staged,
        hunk,
        signature,
      };
      groups.insert(group_id.clone(), group);
      assign_group_id(&mut lines, &pending.display_indices, &group_id);
    }

    Projection::from_lines(doc_line_count, lines, groups, start_gap, end_gap)
  }

  pub fn full(doc_line_count: usize) -> Self {
    let mut lines = Vec::with_capacity(doc_line_count);
    for doc_line in 0..doc_line_count {
      lines.push(DisplayLine::Doc {
        doc_line,
        old_line: Some(doc_line),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      });
    }
    Projection::from_lines(doc_line_count, lines, HashMap::new(), None, None)
  }

  pub fn display_to_doc_line(&self, display_line: usize) -> Option<usize> {
    self
      .display_to_doc
      .get(display_line)
      .and_then(|value| *value)
  }

  pub fn doc_to_display_line(&self, doc_line: usize) -> Option<usize> {
    self.doc_to_display.get(doc_line).and_then(|value| *value)
  }

  pub fn previous_visible_doc_line(&self, doc_line: usize) -> Option<usize> {
    match self.visible_doc_lines.binary_search(&doc_line) {
      Ok(idx) => idx
        .checked_sub(1)
        .and_then(|i| self.visible_doc_lines.get(i).copied()),
      Err(idx) => idx
        .checked_sub(1)
        .and_then(|i| self.visible_doc_lines.get(i).copied()),
    }
  }

  pub fn next_visible_doc_line(&self, doc_line: usize) -> Option<usize> {
    match self.visible_doc_lines.binary_search(&doc_line) {
      Ok(idx) => self.visible_doc_lines.get(idx + 1).copied(),
      Err(idx) => self.visible_doc_lines.get(idx).copied(),
    }
  }

  fn from_lines(
    doc_line_count: usize,
    lines: Vec<DisplayLine>,
    groups: HashMap<Arc<str>, ChangeGroup>,
    start_gap: Option<GapId>,
    end_gap: Option<GapId>,
  ) -> Self {
    let mut display_to_doc = Vec::with_capacity(lines.len());
    let mut doc_to_display = vec![None; doc_line_count];
    let mut visible_doc_lines = Vec::new();

    for (display_idx, line) in lines.iter().enumerate() {
      if let DisplayLine::Doc { doc_line, .. } | DisplayLine::Modified { doc_line, .. } = line {
        display_to_doc.push(Some(*doc_line));
        if *doc_line < doc_line_count && doc_to_display[*doc_line].is_none() {
          doc_to_display[*doc_line] = Some(display_idx);
        }
        visible_doc_lines.push(*doc_line);
      } else {
        display_to_doc.push(None);
      }
    }

    visible_doc_lines.sort_unstable();
    visible_doc_lines.dedup();

    Projection {
      lines,
      display_to_doc,
      doc_to_display,
      visible_doc_lines,
      start_gap,
      end_gap,
      groups,
    }
  }

  pub fn with_review_comments(
    self,
    comments: &[ReviewComment],
    collapsed: &HashSet<u64>,
    _wrap_columns: usize,
    editor_line_height_px: f32,
    markdown_line_height_px: f32,
    comment_body_heights_px: &HashMap<u64, f32>,
  ) -> Self {
    if comments.is_empty() {
      return self;
    }

    let doc_line_count = self.doc_to_display.len();
    let lines = insert_review_comments(
      self.lines,
      comments,
      collapsed,
      _wrap_columns,
      editor_line_height_px,
      markdown_line_height_px,
      comment_body_heights_px,
    );
    Projection::from_lines(
      doc_line_count,
      lines,
      self.groups,
      self.start_gap,
      self.end_gap,
    )
  }
}

fn resolve_review_comment_thread_root(
  comment: &ReviewComment,
  comments_by_id: &HashMap<u64, &ReviewComment>,
) -> u64 {
  let mut root_id = comment.id;
  let mut parent = comment.in_reply_to_id;
  for _ in 0..64 {
    let Some(parent_id) = parent else {
      break;
    };
    if parent_id == root_id {
      break;
    }
    root_id = parent_id;
    parent = comments_by_id
      .get(&parent_id)
      .and_then(|value| value.in_reply_to_id);
  }
  root_id
}

fn review_comment_header_text(comment: &ReviewComment) -> String {
  let line_label = comment
    .line_label
    .as_deref()
    .unwrap_or("")
    .trim()
    .to_string();
  if line_label.is_empty() {
    format!("{} {}", comment.author, comment.created_at)
  } else {
    format!("{} {} {}", comment.author, line_label, comment.created_at)
  }
}

fn required_extra_lines(extra_px: f32, line_height_px: f32) -> usize {
  if extra_px <= 0.0 || line_height_px <= 0.0 {
    return 0;
  }
  (extra_px / line_height_px).ceil() as usize
}

fn estimated_expanded_thread_height_px(
  thread_comments: &[&ReviewComment],
  editor_line_height_px: f32,
  comment_body_heights_px: &HashMap<u64, f32>,
  fallback_markdown_height_px: f32,
) -> f32 {
  if thread_comments.is_empty() {
    return editor_line_height_px * REVIEW_COMMENT_HEADER_HEIGHT_LINES
      + REVIEW_COMMENT_CARD_BORDER_PX * 2.0;
  }

  let mut total_px = REVIEW_COMMENT_CARD_BORDER_PX * 2.0;
  total_px += editor_line_height_px * REVIEW_COMMENT_HEADER_HEIGHT_LINES;
  total_px += REVIEW_COMMENT_CARD_CONTENT_GAP_PX;

  let first_message = thread_comments[0];
  total_px += comment_body_heights_px
    .get(&first_message.id)
    .copied()
    .unwrap_or(fallback_markdown_height_px);
  total_px += REVIEW_COMMENT_FIRST_MESSAGE_BOTTOM_PADDING_PX;

  for reply in thread_comments.iter().skip(1) {
    total_px += REVIEW_COMMENT_REPLY_VERTICAL_PADDING_PX;
    total_px += REVIEW_COMMENT_REPLY_BORDER_TOP_PX;
    total_px += REVIEW_COMMENT_REPLY_HEADER_BODY_GAP_PX;
    total_px += editor_line_height_px;
    total_px += comment_body_heights_px
      .get(&reply.id)
      .copied()
      .unwrap_or(fallback_markdown_height_px);
  }

  total_px
}

fn insert_review_comments(
  lines: Vec<DisplayLine>,
  comments: &[ReviewComment],
  collapsed: &HashSet<u64>,
  _wrap_columns: usize,
  editor_line_height_px: f32,
  markdown_line_height_px: f32,
  comment_body_heights_px: &HashMap<u64, f32>,
) -> Vec<DisplayLine> {
  #[derive(Clone)]
  struct ThreadInsertion<'a> {
    thread_id: u64,
    comments: Vec<&'a ReviewComment>,
  }

  if comments.is_empty() {
    return lines;
  }

  let mut new_line_to_display: HashMap<usize, usize> = HashMap::new();
  let mut old_line_to_display: HashMap<usize, usize> = HashMap::new();

  for (idx, line) in lines.iter().enumerate() {
    match line {
      DisplayLine::Doc {
        doc_line, old_line, ..
      } => {
        new_line_to_display.entry(*doc_line).or_insert(idx);
        if let Some(old_line) = old_line {
          old_line_to_display.entry(*old_line).or_insert(idx);
        }
      }
      DisplayLine::Modified {
        doc_line, old_line, ..
      } => {
        new_line_to_display.entry(*doc_line).or_insert(idx);
        old_line_to_display.entry(*old_line).or_insert(idx);
      }
      DisplayLine::Removed { old_line, .. } => {
        old_line_to_display.entry(*old_line).or_insert(idx);
      }
      _ => {}
    }
  }

  let comments_by_id: HashMap<u64, &ReviewComment> = comments
    .iter()
    .map(|comment| (comment.id, comment))
    .collect();
  let mut comment_targets: HashMap<u64, usize> = HashMap::new();
  for comment in comments {
    let target = match comment.side {
      ReviewCommentSide::Left => old_line_to_display.get(&comment.line),
      ReviewCommentSide::Right => new_line_to_display.get(&comment.line),
    };
    if let Some(&display_idx) = target {
      comment_targets.insert(comment.id, display_idx);
    }
  }

  if comment_targets.is_empty() {
    return lines;
  }

  let mut thread_order = Vec::new();
  let mut comments_by_thread: HashMap<u64, Vec<&ReviewComment>> = HashMap::new();
  for comment in comments {
    let root_id = resolve_review_comment_thread_root(comment, &comments_by_id);
    if !comments_by_thread.contains_key(&root_id) {
      thread_order.push(root_id);
    }
    comments_by_thread.entry(root_id).or_default().push(comment);
  }

  let mut threads_by_display: HashMap<usize, Vec<ThreadInsertion<'_>>> = HashMap::new();
  for thread_id in thread_order {
    let Some(thread_comments) = comments_by_thread.get(&thread_id) else {
      continue;
    };
    let root_target = thread_comments
      .iter()
      .find(|comment| comment.id == thread_id)
      .and_then(|comment| comment_targets.get(&comment.id).copied());
    let target = root_target.or_else(|| {
      thread_comments
        .iter()
        .find_map(|comment| comment_targets.get(&comment.id).copied())
    });
    let Some(display_idx) = target else {
      continue;
    };
    threads_by_display
      .entry(display_idx)
      .or_default()
      .push(ThreadInsertion {
        thread_id,
        comments: thread_comments.clone(),
      });
  }

  if threads_by_display.is_empty() {
    return lines;
  }
  let editor_line_height_px = editor_line_height_px.max(1.0);
  let markdown_line_height_px = markdown_line_height_px.max(1.0);

  let mut result = Vec::with_capacity(lines.len() + comments.len().saturating_mul(2));
  for (idx, line) in lines.into_iter().enumerate() {
    result.push(line);

    let Some(threads) = threads_by_display.get(&idx) else {
      continue;
    };

    let (group_id, secondary, background) = match result.last() {
      Some(DisplayLine::Doc {
        change,
        group_id,
        secondary,
        ..
      }) => (
        group_id.clone(),
        *secondary,
        match change {
          Some(ChangeKind::Added) => Some(ReviewCommentBackground::Added),
          _ => None,
        },
      ),
      Some(DisplayLine::Modified {
        group_id,
        secondary,
        ..
      }) => (
        group_id.clone(),
        *secondary,
        Some(ReviewCommentBackground::Added),
      ),
      Some(DisplayLine::Removed {
        group_id,
        secondary,
        ..
      }) => (
        group_id.clone(),
        *secondary,
        Some(ReviewCommentBackground::Removed),
      ),
      Some(DisplayLine::NoNewline {
        group_id,
        secondary,
        ..
      }) => (group_id.clone(), *secondary, None),
      _ => (None, false, None),
    };

    for thread in threads {
      let Some(first_comment) = thread.comments.first().copied() else {
        continue;
      };
      let header_comment = thread
        .comments
        .iter()
        .find(|comment| comment.id == thread.thread_id)
        .copied()
        .unwrap_or(first_comment);
      let thread_is_collapsed = !thread.comments.is_empty()
        && thread
          .comments
          .iter()
          .all(|comment| collapsed.contains(&comment.id));

      let comment_background = match (header_comment.side, background) {
        (ReviewCommentSide::Left, Some(ReviewCommentBackground::Added)) => {
          Some(ReviewCommentBackground::Removed)
        }
        (ReviewCommentSide::Right, Some(ReviewCommentBackground::Removed)) => {
          Some(ReviewCommentBackground::Added)
        }
        (_, value) => value,
      };
      let reserved_lines = if thread_is_collapsed {
        REVIEW_COMMENT_COLLAPSED_LINES
      } else {
        let expanded_height_px = estimated_expanded_thread_height_px(
          &thread.comments,
          editor_line_height_px,
          comment_body_heights_px,
          markdown_line_height_px,
        );
        required_extra_lines(expanded_height_px, editor_line_height_px)
          .max(REVIEW_COMMENT_COLLAPSED_LINES)
      }
      .max(1);

      let header = review_comment_header_text(header_comment);
      for line_idx in 0..reserved_lines {
        result.push(DisplayLine::ReviewComment {
          id: header_comment.id,
          side: header_comment.side,
          group_id: group_id.clone(),
          background: comment_background,
          secondary,
          text: Arc::from(if line_idx == 0 { header.as_str() } else { "" }),
          is_header: line_idx == 0,
        });
      }
    }
  }

  result
}

struct HunkDisplay {
  start_line: usize,
  first_doc_line: Option<usize>,
  last_doc_line: Option<usize>,
  delta: isize,
  lines: Vec<DisplayLine>,
  pending_groups: Vec<PendingStagedGroup>,
}

impl HunkDisplay {
  fn sort_key(&self) -> usize {
    self.first_doc_line.unwrap_or(self.start_line)
  }
}

struct StagedGroupBuilder {
  group: GroupBuilder,
  display_indices: Vec<usize>,
}

fn collect_groups(
  diff: &FileDiff,
  state: HunkState,
) -> (HashMap<Arc<str>, ChangeGroup>, HashMap<LineKey, Arc<str>>) {
  let mut groups = HashMap::new();
  let mut line_to_group = HashMap::new();
  let mut key_builder = LineKeyBuilder::default();

  for hunk in &diff.hunks {
    if hunk.lines.is_empty() {
      continue;
    }

    let mut old_line = hunk.old_start;
    let mut new_line = hunk.new_start;
    let mut old_line_at = Vec::with_capacity(hunk.lines.len());
    let mut new_line_at = Vec::with_capacity(hunk.lines.len());

    for line in &hunk.lines {
      old_line_at.push(old_line);
      new_line_at.push(new_line);
      match line.kind {
        DiffLineKind::Context => {
          old_line = old_line.saturating_add(1);
          new_line = new_line.saturating_add(1);
        }
        DiffLineKind::Add => {
          new_line = new_line.saturating_add(1);
        }
        DiffLineKind::Remove => {
          old_line = old_line.saturating_add(1);
        }
      }
    }

    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < hunk.lines.len() {
      if matches!(hunk.lines[idx].kind, DiffLineKind::Context) {
        idx += 1;
        continue;
      }
      let start = idx;
      while idx < hunk.lines.len() && !matches!(hunk.lines[idx].kind, DiffLineKind::Context) {
        idx += 1;
      }
      let end = idx.saturating_sub(1);
      blocks.push((start, end));
    }

    for (block_idx, (block_start, block_end)) in blocks.iter().copied().enumerate() {
      let include_start = if block_idx == 0 {
        0
      } else {
        blocks[block_idx - 1].1.saturating_add(1)
      };
      let include_end = if block_idx + 1 >= blocks.len() {
        hunk.lines.len().saturating_sub(1)
      } else {
        blocks[block_idx + 1].0.saturating_sub(1)
      };

      let mut builder = GroupBuilder {
        start_old_line: old_line_at[include_start],
        start_new_line: new_line_at[include_start],
        old_lines: 0,
        new_lines: 0,
        lines: Vec::new(),
        keys: Vec::new(),
      };

      for (line_idx, line) in hunk
        .lines
        .iter()
        .enumerate()
        .take(include_end + 1)
        .skip(include_start)
      {
        builder.lines.push(line.clone());
        match line.kind {
          DiffLineKind::Context => {
            builder.old_lines = builder.old_lines.saturating_add(1);
            builder.new_lines = builder.new_lines.saturating_add(1);
          }
          DiffLineKind::Add => {
            builder.new_lines = builder.new_lines.saturating_add(1);
            if line_idx >= block_start && line_idx <= block_end {
              let doc_line = new_line_at[line_idx].saturating_sub(1);
              let key = key_builder.line_key(LineKeyKind::Add, doc_line, &line.content);
              builder.keys.push(key);
            }
          }
          DiffLineKind::Remove => {
            builder.old_lines = builder.old_lines.saturating_add(1);
            if line_idx >= block_start && line_idx <= block_end {
              let anchor_line = new_line_at[line_idx].saturating_sub(1);
              let key = key_builder.line_key(LineKeyKind::Remove, anchor_line, &line.content);
              builder.keys.push(key);
            }
          }
        }
      }

      finalize_group(builder, state, &mut groups, &mut line_to_group);
    }
  }

  (groups, line_to_group)
}

fn build_hunk_display_inline(
  hunk: &DiffHunk,
  key_builder: &mut LineKeyBuilder,
  unstaged_line_to_group: &HashMap<LineKey, Arc<str>>,
) -> HunkDisplay {
  let (computed_old_lines, computed_new_lines) = count_hunk_line_counts(hunk);
  let mut new_line = hunk.new_start.saturating_sub(1);
  let mut old_line = hunk.old_start.saturating_sub(1);
  let mut first_doc_line = None;
  let mut last_doc_line = None;
  let mut lines = Vec::new();
  let mut pending_groups = Vec::new();
  let mut staged_group: Option<StagedGroupBuilder> = None;

  let finalize_staged = |builder: StagedGroupBuilder,
                         pending_groups: &mut Vec<PendingStagedGroup>| {
    if builder.group.keys.is_empty() {
      return;
    }
    let signature = group_signature_for_lines(&builder.group.lines);
    pending_groups.push(PendingStagedGroup {
      builder: builder.group,
      display_indices: builder.display_indices,
      signature,
    });
  };

  for line in &hunk.lines {
    match line.kind {
      DiffLineKind::Context => {
        if let Some(builder) = staged_group.take() {
          finalize_staged(builder, &mut pending_groups);
        }

        let doc_line = new_line;
        lines.push(DisplayLine::Doc {
          doc_line,
          old_line: Some(old_line),
          change: Some(ChangeKind::Context),
          hunk: None,
          group_id: None,
          secondary: false,
        });
        first_doc_line.get_or_insert(doc_line);
        last_doc_line = Some(doc_line);
        old_line = old_line.saturating_add(1);
        new_line = new_line.saturating_add(1);
      }
      DiffLineKind::Add => {
        let doc_line = new_line;
        let key = key_builder.line_key(LineKeyKind::Add, doc_line, &line.content);
        if let Some(group_id) = unstaged_line_to_group.get(&key) {
          if let Some(builder) = staged_group.take() {
            finalize_staged(builder, &mut pending_groups);
          }
          lines.push(DisplayLine::Doc {
            doc_line,
            old_line: None,
            change: Some(ChangeKind::Added),
            hunk: Some(HunkState::Unstaged),
            group_id: Some(group_id.clone()),
            secondary: false,
          });
        } else {
          let builder = staged_group.get_or_insert_with(|| StagedGroupBuilder {
            group: GroupBuilder {
              start_old_line: diff_start(old_line, hunk.old_start),
              start_new_line: diff_start(new_line, hunk.new_start),
              old_lines: 0,
              new_lines: 0,
              lines: Vec::new(),
              keys: Vec::new(),
            },
            display_indices: Vec::new(),
          });
          builder.group.lines.push(line.clone());
          builder.group.keys.push(key);
          builder.group.new_lines = builder.group.new_lines.saturating_add(1);
          let index = lines.len();
          builder.display_indices.push(index);
          lines.push(DisplayLine::Doc {
            doc_line,
            old_line: None,
            change: Some(ChangeKind::Added),
            hunk: Some(HunkState::Staged),
            group_id: None,
            secondary: true,
          });
        }
        first_doc_line.get_or_insert(doc_line);
        last_doc_line = Some(doc_line);
        new_line = new_line.saturating_add(1);
      }
      DiffLineKind::Remove => {
        let anchor_line = new_line;
        let key = key_builder.line_key(LineKeyKind::Remove, anchor_line, &line.content);
        if let Some(group_id) = unstaged_line_to_group.get(&key) {
          if let Some(builder) = staged_group.take() {
            finalize_staged(builder, &mut pending_groups);
          }
          lines.push(DisplayLine::Removed {
            text: line.content.clone(),
            anchor_line,
            old_line,
            hunk: HunkState::Unstaged,
            group_id: Some(group_id.clone()),
            secondary: false,
          });
        } else {
          let builder = staged_group.get_or_insert_with(|| StagedGroupBuilder {
            group: GroupBuilder {
              start_old_line: diff_start(old_line, hunk.old_start),
              start_new_line: diff_start(new_line, hunk.new_start),
              old_lines: 0,
              new_lines: 0,
              lines: Vec::new(),
              keys: Vec::new(),
            },
            display_indices: Vec::new(),
          });
          builder.group.lines.push(line.clone());
          builder.group.keys.push(key);
          builder.group.old_lines = builder.group.old_lines.saturating_add(1);
          let index = lines.len();
          builder.display_indices.push(index);
          lines.push(DisplayLine::Removed {
            text: line.content.clone(),
            anchor_line,
            old_line,
            hunk: HunkState::Staged,
            group_id: None,
            secondary: true,
          });
        }
        old_line = old_line.saturating_add(1);
      }
    }
  }

  if let Some(builder) = staged_group {
    finalize_staged(builder, &mut pending_groups);
  }

  HunkDisplay {
    start_line: hunk.new_start.saturating_sub(1),
    first_doc_line,
    last_doc_line,
    delta: computed_old_lines as isize - computed_new_lines as isize,
    lines,
    pending_groups,
  }
}

fn build_hunk_display_split(
  hunk: &DiffHunk,
  key_builder: &mut LineKeyBuilder,
  unstaged_line_to_group: &HashMap<LineKey, Arc<str>>,
) -> HunkDisplay {
  build_hunk_display_split_inner(hunk, key_builder, unstaged_line_to_group)
}

fn build_hunk_display_split_inner(
  hunk: &DiffHunk,
  key_builder: &mut LineKeyBuilder,
  unstaged_line_to_group: &HashMap<LineKey, Arc<str>>,
) -> HunkDisplay {
  let (computed_old_lines, computed_new_lines) = count_hunk_line_counts(hunk);
  #[derive(Clone)]
  struct PendingLine {
    content: Arc<str>,
    old_line: usize,
    new_line: usize,
    anchor_line: usize,
    group_id: Option<Arc<str>>,
    secondary: bool,
  }

  let mut new_line = hunk.new_start.saturating_sub(1);
  let mut old_line = hunk.old_start.saturating_sub(1);
  let mut first_doc_line = None;
  let mut last_doc_line = None;
  let mut lines = Vec::new();
  let mut pending_groups = Vec::new();
  let mut staged_group: Option<StagedGroupBuilder> = None;
  let mut remove_queue = std::collections::VecDeque::new();
  let mut add_queue = std::collections::VecDeque::new();

  let finalize_staged = |builder: StagedGroupBuilder,
                         pending_groups: &mut Vec<PendingStagedGroup>| {
    if builder.group.keys.is_empty() {
      return;
    }
    let signature = group_signature_for_lines(&builder.group.lines);
    pending_groups.push(PendingStagedGroup {
      builder: builder.group,
      display_indices: builder.display_indices,
      signature,
    });
  };

  let flush_pending = |remove_queue: &mut std::collections::VecDeque<PendingLine>,
                       add_queue: &mut std::collections::VecDeque<PendingLine>,
                       lines: &mut Vec<DisplayLine>,
                       staged_group: &mut Option<StagedGroupBuilder>,
                       first_doc_line: &mut Option<usize>,
                       last_doc_line: &mut Option<usize>| {
    let state_for_secondary = |secondary: bool| {
      if secondary {
        HunkState::Staged
      } else {
        HunkState::Unstaged
      }
    };

    while remove_queue.front().is_some() && add_queue.front().is_some() {
      let remove = remove_queue.pop_front().expect("remove line");
      let add = add_queue.pop_front().expect("add line");
      let secondary = remove.secondary && add.secondary;
      let state = state_for_secondary(secondary);
      let group_id = if !secondary {
        if !remove.secondary {
          remove.group_id.clone()
        } else {
          None
        }
        .or_else(|| {
          if !add.secondary {
            add.group_id.clone()
          } else {
            None
          }
        })
        .or_else(|| remove.group_id.clone())
        .or_else(|| add.group_id.clone())
      } else {
        remove.group_id.clone().or_else(|| add.group_id.clone())
      };

      let index = lines.len();
      lines.push(DisplayLine::Modified {
        old_text: remove.content.clone(),
        doc_line: add.new_line,
        old_line: remove.old_line,
        hunk: state,
        group_id: group_id.clone(),
        secondary,
      });

      if state == HunkState::Staged
        && let Some(builder) = staged_group.as_mut()
      {
        builder.display_indices.push(index);
      }

      first_doc_line.get_or_insert(add.new_line);
      *last_doc_line = Some(add.new_line);
    }

    while let Some(remove) = remove_queue.pop_front() {
      let state = state_for_secondary(remove.secondary);
      let group_id = remove.group_id.clone();
      let secondary = remove.secondary;
      let index = lines.len();
      lines.push(DisplayLine::Removed {
        text: remove.content.clone(),
        anchor_line: remove.anchor_line,
        old_line: remove.old_line,
        hunk: state,
        group_id: group_id.clone(),
        secondary,
      });

      if state == HunkState::Staged
        && let Some(builder) = staged_group.as_mut()
      {
        builder.display_indices.push(index);
      }
    }

    while let Some(add) = add_queue.pop_front() {
      let state = state_for_secondary(add.secondary);
      let group_id = add.group_id.clone();
      let secondary = add.secondary;
      let index = lines.len();
      lines.push(DisplayLine::Doc {
        doc_line: add.new_line,
        old_line: None,
        change: Some(ChangeKind::Added),
        hunk: Some(state),
        group_id: group_id.clone(),
        secondary,
      });

      if state == HunkState::Staged
        && let Some(builder) = staged_group.as_mut()
      {
        builder.display_indices.push(index);
      }

      first_doc_line.get_or_insert(add.new_line);
      *last_doc_line = Some(add.new_line);
    }
  };

  for line in &hunk.lines {
    match line.kind {
      DiffLineKind::Context => {
        flush_pending(
          &mut remove_queue,
          &mut add_queue,
          &mut lines,
          &mut staged_group,
          &mut first_doc_line,
          &mut last_doc_line,
        );

        if let Some(builder) = staged_group.take() {
          finalize_staged(builder, &mut pending_groups);
        }

        let doc_line = new_line;
        lines.push(DisplayLine::Doc {
          doc_line,
          old_line: Some(old_line),
          change: Some(ChangeKind::Context),
          hunk: None,
          group_id: None,
          secondary: false,
        });
        first_doc_line.get_or_insert(doc_line);
        last_doc_line = Some(doc_line);
        old_line = old_line.saturating_add(1);
        new_line = new_line.saturating_add(1);
      }
      DiffLineKind::Add => {
        let doc_line = new_line;
        let key = key_builder.line_key(LineKeyKind::Add, doc_line, &line.content);
        let (state, group_id, secondary) = if let Some(group_id) = unstaged_line_to_group.get(&key)
        {
          (HunkState::Unstaged, Some(group_id.clone()), false)
        } else {
          (HunkState::Staged, None, true)
        };

        if state == HunkState::Staged {
          let builder = staged_group.get_or_insert_with(|| StagedGroupBuilder {
            group: GroupBuilder {
              start_old_line: diff_start(old_line, hunk.old_start),
              start_new_line: diff_start(new_line, hunk.new_start),
              old_lines: 0,
              new_lines: 0,
              lines: Vec::new(),
              keys: Vec::new(),
            },
            display_indices: Vec::new(),
          });
          builder.group.lines.push(line.clone());
          builder.group.keys.push(key);
          builder.group.new_lines = builder.group.new_lines.saturating_add(1);
        }

        add_queue.push_back(PendingLine {
          content: line.content.clone(),
          old_line,
          new_line: doc_line,
          anchor_line: doc_line,
          group_id,
          secondary,
        });

        new_line = new_line.saturating_add(1);
      }
      DiffLineKind::Remove => {
        let anchor_line = new_line;
        let key = key_builder.line_key(LineKeyKind::Remove, anchor_line, &line.content);
        let (state, group_id, secondary) = if let Some(group_id) = unstaged_line_to_group.get(&key)
        {
          (HunkState::Unstaged, Some(group_id.clone()), false)
        } else {
          (HunkState::Staged, None, true)
        };

        if state == HunkState::Staged {
          let builder = staged_group.get_or_insert_with(|| StagedGroupBuilder {
            group: GroupBuilder {
              start_old_line: diff_start(old_line, hunk.old_start),
              start_new_line: diff_start(new_line, hunk.new_start),
              old_lines: 0,
              new_lines: 0,
              lines: Vec::new(),
              keys: Vec::new(),
            },
            display_indices: Vec::new(),
          });
          builder.group.lines.push(line.clone());
          builder.group.keys.push(key);
          builder.group.old_lines = builder.group.old_lines.saturating_add(1);
        }

        remove_queue.push_back(PendingLine {
          content: line.content.clone(),
          old_line,
          new_line,
          anchor_line,
          group_id,
          secondary,
        });

        old_line = old_line.saturating_add(1);
      }
    }
  }

  flush_pending(
    &mut remove_queue,
    &mut add_queue,
    &mut lines,
    &mut staged_group,
    &mut first_doc_line,
    &mut last_doc_line,
  );

  if let Some(builder) = staged_group.take() {
    finalize_staged(builder, &mut pending_groups);
  }

  HunkDisplay {
    start_line: hunk.new_start.saturating_sub(1),
    first_doc_line,
    last_doc_line,
    delta: computed_old_lines as isize - computed_new_lines as isize,
    lines,
    pending_groups,
  }
}

fn finalize_group(
  builder: GroupBuilder,
  state: HunkState,
  groups: &mut HashMap<Arc<str>, ChangeGroup>,
  line_to_group: &mut HashMap<LineKey, Arc<str>>,
) {
  if builder.keys.is_empty() {
    return;
  }

  let group_id = group_id_for_keys(&builder.keys);
  let signature = group_signature_for_lines(&builder.lines);
  let hunk = DiffHunk {
    id: group_id.to_string(),
    old_start: builder.start_old_line,
    old_lines: builder.old_lines,
    new_start: builder.start_new_line,
    new_lines: builder.new_lines,
    lines: builder.lines,
  };
  let group = ChangeGroup {
    id: group_id.clone(),
    state,
    hunk,
    signature,
  };
  groups.insert(group_id.clone(), group);
  for key in builder.keys {
    line_to_group.insert(key, group_id.clone());
  }
}

fn assign_group_id(lines: &mut [DisplayLine], indices: &[usize], group_id: &Arc<str>) {
  for index in indices {
    if let Some(line) = lines.get_mut(*index) {
      match line {
        DisplayLine::Doc { group_id: id, .. } => {
          *id = Some(group_id.clone());
        }
        DisplayLine::Modified { group_id: id, .. } => {
          *id = Some(group_id.clone());
        }
        DisplayLine::Removed { group_id: id, .. } => {
          *id = Some(group_id.clone());
        }
        DisplayLine::NoNewline { group_id: id, .. } => {
          *id = Some(group_id.clone());
        }
        _ => {}
      }
    }
  }
}

fn group_signature_for_lines(lines: &[DiffLine]) -> Arc<str> {
  let mut hasher = Hasher::new();
  for line in lines {
    let prefix = match line.kind {
      DiffLineKind::Context => b' ',
      DiffLineKind::Add => b'+',
      DiffLineKind::Remove => b'-',
    };
    if matches!(line.kind, DiffLineKind::Context) {
      continue;
    }
    hasher.update(&[prefix]);
    hasher.update(line.content.as_bytes());
    if line.no_newline {
      hasher.update(b"\\ No newline at end of file");
    }
    hasher.update(b"\n");
  }
  Arc::from(hasher.finalize().to_hex().to_string())
}

fn group_id_for_keys(keys: &[LineKey]) -> Arc<str> {
  let mut hasher = Hasher::new();
  for key in keys {
    match key.kind {
      LineKeyKind::Add => hasher.update(b"+"),
      LineKeyKind::Remove => hasher.update(b"-"),
    };
    let line = key.line as u64;
    hasher.update(&line.to_le_bytes());
    hasher.update(&key.occurrence.to_le_bytes());
    hasher.update(key.content.as_bytes());
    hasher.update(b"\n");
  }
  Arc::from(hasher.finalize().to_hex().to_string())
}

fn diff_start(line_zero: usize, base_start: usize) -> usize {
  if base_start == 0 {
    line_zero
  } else {
    line_zero.saturating_add(1)
  }
}

fn count_hunk_line_counts(hunk: &DiffHunk) -> (usize, usize) {
  let mut old_lines: usize = 0;
  let mut new_lines: usize = 0;
  for line in &hunk.lines {
    match line.kind {
      DiffLineKind::Context => {
        old_lines = old_lines.saturating_add(1);
        new_lines = new_lines.saturating_add(1);
      }
      DiffLineKind::Add => {
        new_lines = new_lines.saturating_add(1);
      }
      DiffLineKind::Remove => {
        old_lines = old_lines.saturating_add(1);
      }
    }
  }
  (old_lines, new_lines)
}

#[cfg(test)]
mod tests {
  use super::*;
  use git::{GitFileBases, compute_buffer_diffs};
  use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
  };

  fn diffs_from(base: &str, buffer: &str) -> git::DiffSet {
    let bases = GitFileBases {
      head: Some(base.to_string()),
      index: Some(base.to_string()),
    };
    compute_buffer_diffs(&bases, buffer, Path::new("test.txt")).expect("diffs")
  }

  fn diffs_from_versions(head: &str, index: &str, buffer: &str) -> git::DiffSet {
    let bases = GitFileBases {
      head: Some(head.to_string()),
      index: Some(index.to_string()),
    };
    compute_buffer_diffs(&bases, buffer, Path::new("test.txt")).expect("diffs")
  }

  fn projection_from(base: &str, buffer: &str, align_modified: bool) -> Projection {
    let diffs = diffs_from(base, buffer);
    let doc_line_count = buffer.split('\n').count();
    Projection::from_diffs(
      doc_line_count,
      &diffs.uncommitted,
      &diffs.unstaged,
      &diffs.staged,
      &HashMap::new(),
      align_modified,
    )
  }

  fn review_comment(id: u64, body: &str) -> ReviewComment {
    ReviewComment {
      id,
      in_reply_to_id: None,
      line: 0,
      side: ReviewCommentSide::Right,
      author: Arc::from("joris"),
      avatar_url: None,
      line_label: Some(Arc::from("L1")),
      body: Arc::from(body.to_string()),
      created_at: Arc::from("2026-02-12"),
    }
  }

  fn count_review_comment_lines(projection: &Projection, id: u64) -> usize {
    projection
      .lines
      .iter()
      .filter(
        |line| matches!(line, DisplayLine::ReviewComment { id: line_id, .. } if *line_id == id),
      )
      .count()
  }

  #[test]
  fn split_trailing_newline_removed_keeps_doc_lines_contiguous() {
    let base = "/// ref\n\n\nwsdasdasd\n";
    let buffer = "/// ref\n\n\nwsdasdasd";
    let projection = projection_from(base, buffer, true);

    let doc_lines: Vec<usize> = projection
      .lines
      .iter()
      .filter_map(|line| match line {
        DisplayLine::Doc { doc_line, .. } => Some(*doc_line),
        DisplayLine::Modified { doc_line, .. } => Some(*doc_line),
        _ => None,
      })
      .collect();

    assert_eq!(doc_lines, vec![0, 1, 2, 3]);

    let last = projection.lines.last().expect("last line");
    assert!(matches!(
      last,
      DisplayLine::Removed { text, .. } if text.is_empty()
    ));
  }

  #[test]
  fn projection_keeps_staged_and_unstaged_groups_visible_together() {
    let head = "alpha\nbeta\ngamma\n";
    let index = "alpha staged\nbeta\ngamma\n";
    let buffer = "alpha staged\nbeta current\ngamma\n";
    let diffs = diffs_from_versions(head, index, buffer);
    let projection = Projection::from_diffs(
      buffer.split('\n').count(),
      &diffs.uncommitted,
      &diffs.unstaged,
      &diffs.staged,
      &HashMap::new(),
      true,
    );

    let has_staged = projection.lines.iter().any(|line| {
      matches!(
        line,
        DisplayLine::Doc {
          hunk: Some(HunkState::Staged),
          ..
        } | DisplayLine::Modified {
          hunk: HunkState::Staged,
          ..
        } | DisplayLine::Removed {
          hunk: HunkState::Staged,
          ..
        }
      )
    });
    let has_unstaged = projection.lines.iter().any(|line| {
      matches!(
        line,
        DisplayLine::Doc {
          hunk: Some(HunkState::Unstaged),
          ..
        } | DisplayLine::Modified {
          hunk: HunkState::Unstaged,
          ..
        } | DisplayLine::Removed {
          hunk: HunkState::Unstaged,
          ..
        }
      )
    });

    assert!(has_staged, "projection should keep staged lines visible");
    assert!(
      has_unstaged,
      "projection should keep unstaged lines visible"
    );
  }

  #[test]
  fn split_trailing_newline_added_is_added_empty_line() {
    let base = "/// ref\n\n\nwsdasdasd";
    let buffer = "/// ref\n\n\nwsdasdasd\n";
    let projection = projection_from(base, buffer, true);

    let last = projection.lines.last().expect("last line");
    assert!(matches!(
      last,
      DisplayLine::Doc {
        change: Some(ChangeKind::Added),
        old_line: None,
        ..
      }
    ));
  }

  #[test]
  fn review_comment_collapsed_reserves_fixed_collapsed_lines() {
    let projection = projection_from("line 1\nline 2", "line 1\nline 2", false);
    let comment = review_comment(42, "short");
    let comments = vec![comment.clone()];
    let collapsed = HashSet::from([comment.id]);
    let body_heights = HashMap::from([(comment.id, 20.0f32)]);

    let projection =
      projection.with_review_comments(&comments, &collapsed, 80, 20.0, 20.0, &body_heights);
    let reserved = count_review_comment_lines(&projection, comment.id);

    assert_eq!(reserved, REVIEW_COMMENT_COLLAPSED_LINES);
  }

  #[test]
  fn review_comment_expanded_reservation_grows_with_body_height() {
    let base_projection = projection_from("line 1\nline 2", "line 1\nline 2", false);

    let short_comment = review_comment(43, "short");
    let short_body_heights = HashMap::from([(short_comment.id, 20.0f32)]);
    let short_projection = base_projection.clone().with_review_comments(
      std::slice::from_ref(&short_comment),
      &HashSet::new(),
      80,
      20.0,
      20.0,
      &short_body_heights,
    );

    let long_body = "long paragraph ".repeat(80);
    let long_comment = review_comment(44, &long_body);
    let long_body_heights = HashMap::from([(long_comment.id, 120.0f32)]);
    let long_projection = base_projection.with_review_comments(
      std::slice::from_ref(&long_comment),
      &HashSet::new(),
      80,
      20.0,
      20.0,
      &long_body_heights,
    );

    let short_reserved = count_review_comment_lines(&short_projection, short_comment.id);
    let long_reserved = count_review_comment_lines(&long_projection, long_comment.id);

    assert!(long_reserved > short_reserved);
  }
}
