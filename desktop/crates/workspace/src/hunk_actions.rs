//! Acting on one hunk, or one conflict block, from the diff itself.

use editor::{ConflictResolution, Editor, HunkAction, HunkState};
use git::RepoStatusKind;
use gpui::{
  AnyElement, App, CursorStyle, Entity, InteractiveElement, ParentElement, Pixels, Styled, div,
  prelude::*, px, relative,
};
use gpui_component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, Theme};
use ui::Button;

pub(crate) const HUNK_ACTIONS_DEBUG_SELECTOR: &str = "hunk-actions";
pub(crate) const STAGE_HUNK_DEBUG_SELECTOR: &str = "stage-hunk";
pub(crate) const UNSTAGE_HUNK_DEBUG_SELECTOR: &str = "unstage-hunk";
pub(crate) const RESTORE_HUNK_DEBUG_SELECTOR: &str = "restore-hunk";
pub(crate) const CONFLICT_ACTIONS_DEBUG_SELECTOR: &str = "conflict-actions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConflictActionLabels {
  pub(crate) current: &'static str,
  pub(crate) incoming: &'static str,
  pub(crate) all_current: &'static str,
  pub(crate) all_incoming: &'static str,
  pub(crate) palette_current: &'static str,
  pub(crate) palette_incoming: &'static str,
  pub(crate) palette_current_description: &'static str,
  pub(crate) palette_incoming_description: &'static str,
}

impl ConflictActionLabels {
  pub(crate) fn for_rebase(rebase_in_progress: bool) -> Self {
    if rebase_in_progress {
      Self {
        current: "Accept Target",
        incoming: "Accept Replayed Commit",
        all_current: "Accept All Target",
        all_incoming: "Accept All Replayed Commit",
        palette_current: "Accept all target conflicts",
        palette_incoming: "Accept all replayed commit conflicts",
        palette_current_description: "Resolve all conflict regions by keeping the rebase target",
        palette_incoming_description: "Resolve all conflict regions by keeping the commit being replayed",
      }
    } else {
      Self {
        current: "Accept Current",
        incoming: "Accept Incoming",
        all_current: "Accept All Current",
        all_incoming: "Accept All Incoming",
        palette_current: "Accept all current conflicts",
        palette_incoming: "Accept all incoming conflicts",
        palette_current_description: "Resolve all conflict regions by keeping current changes",
        palette_incoming_description: "Resolve all conflict regions by keeping incoming changes",
      }
    }
  }
}

/// Where the floating actions sit: on the first line of the block, scrolled.
pub(crate) fn hunk_action_top(
  line_height: Pixels,
  display_line: usize,
  scroll_offset: f32,
) -> Pixels {
  line_height * (display_line as f32 - scroll_offset)
}

/// The hovered hunk or conflict block gets its actions; a read-only or
/// untracked file gets none.
pub(crate) fn render_hunk_actions(
  editor: &Entity<Editor>,
  file_status: Option<RepoStatusKind>,
  conflict_labels: ConflictActionLabels,
  cx: &mut App,
) -> Option<AnyElement> {
  let theme = cx.theme().clone();
  let editor_state = editor.read(cx);
  if editor_state.is_read_only {
    return None;
  }

  if matches!(file_status, Some(RepoStatusKind::Conflicted)) {
    return render_conflict_actions(editor, editor_state, &theme, conflict_labels, cx);
  }

  let hovered = editor_state.hovered_group_id.as_ref().and_then(|id| {
    editor_state
      .visible_groups
      .iter()
      .find(|overlay| overlay.id.as_ref() == id.as_ref())
      .map(|overlay| (id.clone(), overlay))
  });
  let selected = (!editor_state.suppress_selected_hunk_actions())
    .then(|| editor_state.highlighted_hunk_group_id(cx))
    .flatten()
    .and_then(|id| {
      editor_state
        .visible_groups
        .iter()
        .find(|overlay| overlay.id.as_ref() == id.as_ref())
        .map(|overlay| (id, overlay))
    });
  let (group_id, overlay) = hovered.or(selected)?;

  let anchor_display_line = editor_state
    .first_display_line_for_group(&group_id)
    .unwrap_or(overlay.display_line);
  let top = visible_action_top(editor_state, anchor_display_line, cx)?;

  // A file git does not track yet has nothing to stage hunk by hunk.
  if matches!(
    file_status,
    Some(RepoStatusKind::Untracked | RepoStatusKind::Added)
  ) {
    return None;
  }

  let file_dirty = editor_state.is_dirty;
  let group_id = overlay.id.clone();
  let state = overlay.state;
  let mut actions = div()
    .flex()
    .items_center()
    .cursor(CursorStyle::Arrow)
    .bg(theme.background)
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .shadow_md();

  match state {
    HunkState::Unstaged => {
      let editor = editor.clone();
      let group_id = group_id.clone();
      actions = actions.child(
        Button::new("stage-hunk")
          .debug_selector(|| STAGE_HUNK_DEBUG_SELECTOR.to_string())
          .icon(IconName::Plus)
          .label("Stage")
          .small()
          .tooltip(if file_dirty {
            "File not saved"
          } else {
            "Stage Hunk (shift-enter)"
          })
          .rounded_t_none()
          .rounded_br_none()
          .bg(theme.background)
          .disabled(file_dirty)
          .on_click(move |_, _, cx| {
            let group_id = group_id.clone();
            editor.update(cx, |editor, cx| {
              editor.enqueue_group_action(group_id, HunkAction::Stage, cx);
            });
          }),
      );
    }
    HunkState::Staged => {
      let editor = editor.clone();
      let group_id = group_id.clone();
      actions = actions.child(
        Button::new("unstage-hunk")
          .debug_selector(|| UNSTAGE_HUNK_DEBUG_SELECTOR.to_string())
          .icon(IconName::Minus)
          .label("Unstage")
          .small()
          .tooltip(if file_dirty {
            "File not saved"
          } else {
            "Unstage Hunk (shift-enter)"
          })
          .rounded_t_none()
          .bg(theme.background)
          .disabled(file_dirty)
          .on_click(move |_, _, cx| {
            let group_id = group_id.clone();
            editor.update(cx, |editor, cx| {
              editor.enqueue_group_action(group_id, HunkAction::Unstage, cx);
            });
          }),
      );
    }
  }

  if matches!(state, HunkState::Unstaged) {
    let editor = editor.clone();
    actions = actions.child(
      Button::new("restore-hunk")
        .debug_selector(|| RESTORE_HUNK_DEBUG_SELECTOR.to_string())
        .icon(IconName::Undo)
        .label("Restore")
        .small()
        .tooltip(if file_dirty {
          "File not saved"
        } else {
          "Restore Hunk (shift-backspace)"
        })
        .rounded_t_none()
        .rounded_bl_none()
        .bg(theme.background)
        .disabled(file_dirty)
        .on_click(move |_, _, cx| {
          let group_id = group_id.clone();
          editor.update(cx, |editor, cx| {
            editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
          });
        }),
    );
  }

  Some(floating(
    top,
    actions
      .debug_selector(|| HUNK_ACTIONS_DEBUG_SELECTOR.to_string())
      .into_any_element(),
    editor_state.hunk_actions_align_left(),
  ))
}

fn render_conflict_actions(
  editor: &Entity<Editor>,
  editor_state: &Editor,
  theme: &Theme,
  labels: ConflictActionLabels,
  cx: &App,
) -> Option<AnyElement> {
  let conflict_start_line = editor_state.hovered_conflict_start_line?;
  let anchor_display_line = editor_state
    .first_display_line_for_conflict(conflict_start_line)
    .unwrap_or(conflict_start_line);
  let top = visible_action_top(editor_state, anchor_display_line, cx)?;

  let side = |id: &'static str, label: &'static str, resolution: ConflictResolution| {
    let editor = editor.clone();
    Button::new(id)
      .label(label)
      .small()
      .bg(theme.background)
      .on_click(move |_, _, cx| {
        editor.update(cx, |editor, cx| {
          editor.resolve_conflict_region(conflict_start_line, resolution, cx);
        });
      })
  };

  let actions = div()
    .flex()
    .items_center()
    .bg(theme.background)
    .border_1()
    .border_color(theme.border)
    .rounded_md()
    .shadow_md()
    .child(
      side(
        "accept-current-conflict",
        labels.current,
        ConflictResolution::Current,
      )
      .rounded_t_none()
      .rounded_br_none(),
    )
    .child(
      side(
        "accept-incoming-conflict",
        labels.incoming,
        ConflictResolution::Incoming,
      )
      .rounded_none(),
    )
    .child(
      side(
        "accept-both-conflict",
        "Accept Both",
        ConflictResolution::Both,
      )
      .rounded_t_none()
      .rounded_bl_none(),
    );

  Some(floating(
    top,
    actions
      .debug_selector(|| CONFLICT_ACTIONS_DEBUG_SELECTOR.to_string())
      .into_any_element(),
    false,
  ))
}

/// `None` when the block scrolled out of the viewport or sits under the find panel.
fn visible_action_top(
  editor_state: &Editor,
  anchor_display_line: usize,
  _cx: &App,
) -> Option<Pixels> {
  if editor_state.find_panel_occludes_display_line(anchor_display_line) {
    return None;
  }
  let top = hunk_action_top(
    editor_state.measured_editor_line_height(),
    anchor_display_line,
    editor_state.scroll_offset_y,
  );
  if top >= editor_state.viewport_height {
    return None;
  }
  Some(top.max(px(0.0)))
}

fn floating(top: Pixels, actions: AnyElement, align_left_split: bool) -> AnyElement {
  let container = div().absolute().top(top).cursor(CursorStyle::Arrow);
  if align_left_split {
    container
      .left(px(0.0))
      .w(relative(0.5))
      .pr(px(30.0))
      .flex()
      .justify_end()
      .child(actions)
      .into_any_element()
  } else {
    container.right(px(30.0)).child(actions).into_any_element()
  }
}

/// `shift-enter`: stage or unstage the hunk under the cursor, accept the
/// current side of the conflict under it.
pub(crate) fn toggle_hunk_stage(
  editor: &Entity<Editor>,
  file_status: Option<RepoStatusKind>,
  cx: &mut App,
) {
  if resolve_active_conflict(editor, file_status, ConflictResolution::Current, cx) {
    return;
  }
  editor.update(cx, |editor, cx| {
    let Some(group_id) = editor.active_hunk_group_id(cx) else {
      return;
    };
    let Some(state) = editor
      .projection()
      .and_then(|projection| projection.groups.get(&group_id))
      .map(|group| group.state)
    else {
      return;
    };
    let action = match state {
      HunkState::Unstaged => HunkAction::Stage,
      HunkState::Staged => HunkAction::Unstage,
    };
    editor.enqueue_group_action(group_id, action, cx);
  });
}

/// `shift-backspace`: restore the hunk under the cursor, accept the incoming
/// side of the conflict under it.
pub(crate) fn restore_hunk(
  editor: &Entity<Editor>,
  file_status: Option<RepoStatusKind>,
  cx: &mut App,
) {
  if resolve_active_conflict(editor, file_status, ConflictResolution::Incoming, cx) {
    return;
  }
  editor.update(cx, |editor, cx| {
    let Some(group_id) = editor.active_hunk_group_id(cx) else {
      return;
    };
    editor.enqueue_group_action(group_id, HunkAction::Restore, cx);
  });
}

/// Resolves the conflict the cursor sits in and reveals the next one, or says
/// there was none to resolve.
pub(crate) fn resolve_active_conflict(
  editor: &Entity<Editor>,
  file_status: Option<RepoStatusKind>,
  resolution: ConflictResolution,
  cx: &mut App,
) -> bool {
  if !matches!(file_status, Some(RepoStatusKind::Conflicted)) {
    return false;
  }
  editor.update(cx, |editor, cx| {
    let Some(state) = editor.conflict_navigation_state(cx) else {
      return false;
    };
    editor.resolve_conflict_region(state.active_start_line, resolution, cx);
    editor.save(cx);
    if let Some(next_state) = editor.conflict_navigation_state(cx) {
      editor.reveal_conflict_start_line(next_state.active_start_line, cx);
    }
    true
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn conflict_action_labels_name_rebase_sides_by_role() {
    let standard = ConflictActionLabels::for_rebase(false);
    assert_eq!(standard.current, "Accept Current");
    assert_eq!(standard.incoming, "Accept Incoming");

    let rebase = ConflictActionLabels::for_rebase(true);
    assert_eq!(rebase.current, "Accept Target");
    assert_eq!(rebase.incoming, "Accept Replayed Commit");
    assert_eq!(rebase.palette_current, "Accept all target conflicts");
    assert_eq!(
      rebase.palette_incoming,
      "Accept all replayed commit conflicts"
    );
  }

  #[test]
  fn the_actions_sit_on_the_line_of_the_block() {
    let line_height = px(20.0);
    assert_eq!(hunk_action_top(line_height, 0, 0.0), px(0.0));
    assert_eq!(hunk_action_top(line_height, 3, 0.0), px(60.0));
    // Scrolled down by two lines: the block moved up by two.
    assert_eq!(hunk_action_top(line_height, 3, 2.0), px(20.0));
    // Fractional scrolling keeps the actions glued to the block.
    assert_eq!(hunk_action_top(line_height, 3, 2.5), px(10.0));
    // A block above the viewport reports a negative position; the caller clamps.
    assert_eq!(hunk_action_top(line_height, 1, 3.0), px(-40.0));
  }
}
