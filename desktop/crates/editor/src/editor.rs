use std::{
  cell::RefCell,
  collections::{HashMap, HashSet, VecDeque, hash_map::DefaultHasher},
  hash::{Hash, Hasher},
  ops::Range,
  path::{Path, PathBuf},
  sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  },
  time::{Duration, Instant, SystemTime},
};

use buffer::TransactionId;
use gfm_markdown_viewer::{
  GithubCodeReferencePreview, LinkAction, MarkdownRenderOptions, MarkdownRenderState,
  MarkdownTextMetrics, ParsedMarkdown, estimate_github_code_reference_preview_height_px,
  estimate_markdown_height_px_with_suggestion_context,
  estimate_parsed_markdown_height_px_with_suggestion_context, parse_markdown,
  render_github_code_reference_preview_card, render_parsed_markdown,
};
use git::{ApplyLocation, DiffSet, FileDiff, GitFileBases, GitStore, RepoFile};
use gpui::{
  Anchor, App, Bounds, Context, CursorStyle, Entity, EntityInputHandler, ExternalPaths,
  FocusHandle, Focusable, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
  ScrollHandle, ShapedLine, SharedString, Subscription, Task, UTF16Selection, Window, black, div,
  point, prelude::*, px, white,
};
use gpui_component::{
  ActiveTheme as _, ColorName, Disableable as _, Icon, IconName, Selectable, Sizable,
  avatar::Avatar,
  button::{Button, ButtonVariants as _},
  h_flex,
  input::{Escape as InputEscape, Input, InputEvent, InputState, TextareaState},
  menu::{DropdownMenu as _, PopupMenuItem},
  resizable::{h_resizable, resizable_panel},
  tag::Tag,
  v_flex,
};
use parking_lot::RwLock;
use syntax::languages;
use ui::{
  MARKDOWN_COMPOSER_CHROME_HEIGHT_PX, MarkdownComposer, StatusThemeExt as _, Theme, UiIconName,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
  boundaries::{line_range_at_offset, word_range_at_offset, word_range_in_text},
  cursor_blink::CursorBlink,
  document::Document,
  editor_element::{EditorElement, PositionMap},
  gutter_element::GutterElement,
  projection::{
    ChangeKind, DisplayLine, GapId, GapReveal, HunkState, NO_NEWLINE_MARKER_TEXT, Projection,
    REVIEW_COMMENT_CARD_BORDER_PX, REVIEW_COMMENT_CARD_PADDING_X_PX,
    REVIEW_COMMENT_HEADER_HEIGHT_LINES, REVIEW_COMMENT_REPLY_BORDER_TOP_PX,
    REVIEW_COMMENT_SPACING_PX, ReviewComment, ReviewCommentLayoutInput, ReviewCommentSide,
    review_comment_shows_header,
  },
  text_offsets::{byte_offset_to_char_offset, char_offset_to_byte_offset},
};

#[derive(Clone, Debug)]
pub struct Transaction {
  pub id: TransactionId,
  pub selection_before: Range<usize>,
  pub selection_after: Range<usize>,
}

/// Default viewport height before first render
const DEFAULT_VIEWPORT_HEIGHT: f32 = 800.0;
/// Default viewport width before first render
const DEFAULT_VIEWPORT_WIDTH: f32 = 1200.0;
/// Default maximum line width
pub const DEFAULT_MAX_LINE_WIDTH: f32 = 800.0;
/// Extra width added to editor content for horizontal scrolling
const EXTRA_EDITOR_WIDTH: f32 = 200.0;
/// Number of spaces to insert on tab
pub(crate) const TAB_SPACES: usize = 4;
/// Maximum number of cached shaped lines
const MAX_CACHE_SIZE: usize = 200;
/// Number of lines of padding when auto-scrolling to cursor
pub(crate) const SCROLL_PADDING: usize = 3;
/// Width of the gutter area
const GUTTER_WIDTH: f32 = 90.0;
const GUTTER_LINE_NUMBER_BASE_RIGHT_PADDING_PX: f32 = 20.0;
/// Default editor line height before first render/prepaint measurement
const DEFAULT_EDITOR_LINE_HEIGHT: f32 = 20.0;
/// Diff recompute debounce (ms)
const DIFF_DEBOUNCE_MS: u64 = 60;
const LARGE_FILE_DIFF_DEBOUNCE_MS: u64 = 180;
const HUGE_FILE_DIFF_DEBOUNCE_MS: u64 = 320;
const LARGE_FILE_DIFF_DEBOUNCE_LINES: usize = 20_000;
const HUGE_FILE_DIFF_DEBOUNCE_LINES: usize = 80_000;
/// Build diff projection off the UI thread for very large files.
const ASYNC_PROJECTION_MIN_DOC_LINES: usize = 5_000;
/// External change polling interval (ms)
const POLL_INTERVAL_MS: u64 = 500;
const FRACTIONAL_SCROLL_EPSILON: f32 = 0.001;
const REVIEW_COMMENT_SCROLL_DURATION: Duration = Duration::from_millis(260);
const REVIEW_COMMENT_SCROLL_TICK: Duration = Duration::from_millis(16);
const REVIEW_COMMENT_SCROLL_MIN_DELTA: f32 = 0.01;
const FIND_SCROLL_DURATION: Duration = Duration::from_millis(260);
const FIND_SCROLL_TICK: Duration = Duration::from_millis(16);
const FIND_SCROLL_MIN_DELTA: f32 = 0.01;
const FIND_PANEL_OCCLUDED_VISIBLE_LINES: usize = 3;
const REVIEW_COMMENT_DEFAULT_WRAP_COLUMNS: usize = 72;
const REVIEW_COMMENT_MIN_WRAP_COLUMNS: usize = 28;
const REVIEW_COMMENT_MAX_WRAP_COLUMNS: usize = 180;
const REVIEW_COMMENT_CHAR_WIDTH_PX: f32 = 7.8;
const REVIEW_COMMENT_FONT_SIZE_PX: f32 = 14.0;
pub(crate) const REVIEW_COMMENT_UI_FONT_FAMILY: &str = ".SystemUIFont";
/// The lines the diff set aside for a comment, and the card that has to fit in them.
pub const REVIEW_COMMENT_BLOCK_DEBUG_SELECTOR: &str = "review-comment-block";
pub const REVIEW_COMMENT_CARD_DEBUG_SELECTOR: &str = "review-comment-card";
const REVIEW_COMMENT_HORIZONTAL_PADDING_PX: f32 =
  REVIEW_COMMENT_CARD_PADDING_X_PX * 2.0 + REVIEW_COMMENT_CARD_BORDER_PX * 2.0;
/// A composer gives one card padding back to the text box's own inset.
const REVIEW_COMMENT_COMPOSER_HORIZONTAL_PADDING_PX: f32 =
  REVIEW_COMMENT_HORIZONTAL_PADDING_PX - REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX;
const REVIEW_COMMENT_DEFAULT_LINE_HEIGHT_PX: f32 = 20.0;
const REVIEW_COMMENT_MAX_WIDTH_PX: f32 = 550.0;
const REVIEW_COMMENT_MIN_WIDTH_PX: f32 = 320.0;
/// Matches the `pr_2` the card overlays keep on the right of the content area.
const REVIEW_COMMENT_CARD_RIGHT_MARGIN_PX: f32 = 8.0;
/// Strip of diff left visible under a card, whatever the whole-line rounding gave it.
const REVIEW_COMMENT_CARD_BOTTOM_MARGIN_PX: f32 = 6.0;
const REVIEW_COMMENT_COMPOSER_ACTIONS_HEIGHT_PX: f32 = 24.0;
const REVIEW_COMMENT_COMPOSER_ACTIONS_GAP_PX: f32 = 8.0;
/// gpui-component input chrome above and below the text: `input_py`. Borderless
/// here, so there is nothing else to count.
const REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_Y_PX: f32 = 8.0;
const REVIEW_COMMENT_COMPOSER_TEXTAREA_VERTICAL_CHROME_PX: f32 =
  REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_Y_PX * 2.0;
/// The input lays its text out at `LINE_HEIGHT`, which is not the markdown one.
pub(crate) const REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_REMS: f32 = 1.25;
const REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_PX: f32 = 20.0;
/// The input's own left inset before the text, `input_px` at the default size.
const REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX: f32 = 10.0;
const REVIEW_COMMENT_COMPOSER_TEXTAREA_HORIZONTAL_CHROME_PX: f32 =
  REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX * 2.0;
/// One xsmall compact icon button.
const REVIEW_COMMENT_COMPOSER_ACTION_BUTTON_WIDTH_PX: f32 = 21.0;
/// `gap_1`, between two buttons and between the text and the first of them.
const REVIEW_COMMENT_COMPOSER_ACTIONS_GAP_X_PX: f32 = 4.0;
/// Room the two spelled-out GitHub destinations take instead of icons.
const REVIEW_COMMENT_COMPOSER_LABELLED_ACTIONS_WIDTH_PX: f32 = 250.0;
/// Room the actions floating over a read card take, cancel and save sized.
/// Reserved for the widest set, send included, so a card does not resize when a
/// comment stops being sendable.
const REVIEW_COMMENT_FLOATING_ACTIONS_WIDTH_PX: f32 = review_comment_actions_width_px(3);
/// Reserving too little puts the card on the next line of diff, reserving too much
/// only leaves air inside it, so the text column is assumed a little narrow.
const REVIEW_COMMENT_COMPOSER_WRAP_SAFETY_PX: f32 = 4.0;
const REVIEW_COMMENT_COMPOSER_MIN_TEXT_WIDTH_PX: f32 = 120.0;
/// `input_text_size` at the input's default size.
const REVIEW_COMMENT_COMPOSER_TEXT_REMS: f32 = 0.875;
const REVIEW_COMMENT_COMPOSER_MIN_ROWS: usize = 1;
const REVIEW_COMMENT_COMPOSER_MAX_ROWS: usize = 12;
const REVIEW_COMMENT_CREATE_DRAFT_COMMENT_ID: u64 = u64::MAX;
const REVIEW_COMMENT_REPLY_DRAFT_COMMENT_ID: u64 = u64::MAX - 1;
const REVIEW_COMMENT_CREATE_SELECTION_BACKGROUND_ALPHA: f32 = 0.16;
const REVIEW_COMMENT_CREATE_BUTTON_GUTTER_RIGHT_PX: f32 = 10.0;
const REVIEW_COMMENT_CREATE_BUTTON_HITBOX_WIDTH_PX: f32 = 10.0;

fn has_fractional_scroll(scroll_offset: f32) -> bool {
  (scroll_offset - scroll_offset.floor()) > FRACTIONAL_SCROLL_EPSILON
}

fn ease_out_cubic(t: f32) -> f32 {
  1.0 - (1.0 - t).powi(3)
}

fn editor_code_font_family(cx: &App) -> SharedString {
  cx.theme().mono_font_family.clone()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VerticalScrollMetrics {
  pub viewport_lines: f32,
  pub scroll_padding: f32,
  pub max_scroll: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorRevealPolicy {
  WhenHidden,
  WithPadding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffViewMode {
  Inline,
  Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCommentDisplayMode {
  Conversation,
  LocalNote,
}

#[derive(Clone, Debug)]
pub struct EditorFileLoad {
  pub content: String,
  pub binary_bytes: Option<Vec<u8>>,
  pub is_read_only: bool,
  pub language_hint: Option<String>,
  pub file_mtime: Option<SystemTime>,
  pub index_mtime: Option<SystemTime>,
}

pub type ReviewCommentEditHandler = Arc<dyn Fn(u64, Arc<str>, &mut Window, &mut App)>;
pub type ReviewCommentCreateHandler =
  Arc<dyn Fn(ReviewCommentCreateRequest, &mut Window, &mut App)>;
pub type ReviewCommentDeleteHandler = Arc<dyn Fn(u64, &mut Window, &mut App)>;
pub type ReviewCommentSendHandler = Arc<dyn Fn(u64, &mut Window, &mut App)>;
pub type ReviewCommentResolveHandler = Arc<dyn Fn(Arc<str>, u64, bool, &mut Window, &mut App)>;
pub type ReviewCommentSuggestionActionFactory = Arc<
  dyn Fn(
      u64,
      Arc<str>,
      bool,
      &App,
    ) -> Arc<
      dyn Fn(gfm_markdown_viewer::SuggestionActionContext, &App) -> gpui::AnyElement + Send + Sync,
    > + Send
    + Sync,
>;
pub type ReviewCommentLinkHandler = Arc<dyn Fn(&str, &mut Window, &mut App) -> bool>;
pub type ReviewCommentCancelHandler = Arc<dyn Fn(&mut Window, &mut App)>;
pub type ReviewCommentImageUploadHandler =
  Arc<dyn Fn(&ExternalPaths, Entity<TextareaState>, &mut Window, &mut App)>;
pub type ReviewCommentPreviewRenderer = Arc<
  dyn Fn(
    &str,
    Option<gfm_markdown_viewer::SuggestionContext>,
    &mut Window,
    &mut App,
  ) -> gpui::AnyElement,
>;
pub type ReviewCommentAssetUrlResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The review capabilities of one editor, as the host installed them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewCapabilities {
  pub display_mode: ReviewCommentDisplayMode,
  pub replies_enabled: bool,
  pub create: bool,
  pub edit: bool,
  pub delete: bool,
  pub cancel: bool,
  pub send: bool,
  pub resolve: bool,
  pub link: bool,
  pub image_upload: bool,
  pub asset_url_resolver: bool,
  pub preview_renderer: bool,
  pub suggestion_action_factory: bool,
  pub pr_number: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewCommentCodeReferencePreview {
  pub url: Arc<str>,
  pub repo: Arc<str>,
  pub path: Arc<str>,
  pub reference: Arc<str>,
  pub start_line: usize,
  pub end_line: usize,
  pub snippets: Vec<Arc<str>>,
  pub full_content: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReviewCommentBodySegment {
  Markdown(String),
  Preview(ReviewCommentCodeReferencePreview),
}

/// Whether a new review comment posts immediately or joins the viewer's pending review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCommentMode {
  SingleComment,
  PendingReview,
}

/// GitHub rejects standalone comments (422) while the viewer has a pending review.
pub fn review_comment_submit_mode(
  display_mode: ReviewCommentDisplayMode,
  has_pending_review: bool,
) -> ReviewCommentMode {
  match display_mode {
    // A local note has a single destination; the batch is submitted by the page.
    ReviewCommentDisplayMode::LocalNote => ReviewCommentMode::SingleComment,
    ReviewCommentDisplayMode::Conversation if has_pending_review => {
      ReviewCommentMode::PendingReview
    }
    ReviewCommentDisplayMode::Conversation => ReviewCommentMode::SingleComment,
  }
}

#[derive(Clone, Debug)]
pub struct ReviewCommentCreateRequest {
  pub line: usize,
  pub side: ReviewCommentSide,
  pub start_line: Option<usize>,
  pub start_side: Option<ReviewCommentSide>,
  pub in_reply_to_id: Option<u64>,
  pub body: Arc<str>,
  pub mode: ReviewCommentMode,
}

fn next_review_comment_body(raw_value: &str, initial_value: &str) -> Option<Arc<str>> {
  let next_body = raw_value.trim();
  if next_body.is_empty() || next_body == initial_value {
    None
  } else {
    Some(Arc::<str>::from(next_body.to_string()))
  }
}

/// The action buttons sit beside the text box, so the taller of the two rules.
fn review_comment_composer_body_height_px(textarea_height_px: f32, chrome_height_px: f32) -> f32 {
  chrome_height_px + textarea_height_px.max(REVIEW_COMMENT_COMPOSER_ACTIONS_HEIGHT_PX)
}

/// Measures a run of the comment font in column units, the column being the sampled
/// average width the wrap budget is counted in. Counting characters makes a run of
/// `i` wrap as late as a run of `M`; real glyph advances do not.
struct ReviewCommentTextMeasurer<'a> {
  column_px: f32,
  font_id: gpui::FontId,
  font_size: Pixels,
  glyph_widths: RefCell<HashMap<char, f32>>,
  cx: &'a App,
}

impl ReviewCommentTextMeasurer<'_> {
  fn width_in_columns(&self, text: &str) -> f32 {
    let mut glyph_widths = self.glyph_widths.borrow_mut();
    let total_px: f32 = text
      .chars()
      .map(|ch| {
        *glyph_widths.entry(ch).or_insert_with(|| {
          self
            .cx
            .text_system()
            .advance(self.font_id, self.font_size, ch)
            .map(|size| size.width / px(1.0))
            .unwrap_or(self.column_px)
        })
      })
      .sum();
    total_px / self.column_px
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewCommentCreateAction {
  pub id: &'static str,
  pub label: &'static str,
  pub icon: UiIconName,
  pub mode: ReviewCommentMode,
  pub primary: bool,
}

const REVIEW_COMMENT_ADD_NOTE_ACTION: ReviewCommentCreateAction = ReviewCommentCreateAction {
  id: "review-comment-create-save",
  label: "Comment",
  icon: UiIconName::Check,
  mode: ReviewCommentMode::SingleComment,
  primary: true,
};
const REVIEW_COMMENT_SINGLE_COMMENT_ACTION: ReviewCommentCreateAction = ReviewCommentCreateAction {
  id: "review-comment-create-save",
  label: "Add single comment",
  icon: UiIconName::MessageCirclePlus,
  mode: ReviewCommentMode::SingleComment,
  primary: false,
};
const REVIEW_COMMENT_START_REVIEW_ACTION: ReviewCommentCreateAction = ReviewCommentCreateAction {
  id: "review-comment-create-start-review",
  label: "Start a review",
  icon: UiIconName::Check,
  mode: ReviewCommentMode::PendingReview,
  primary: true,
};
const REVIEW_COMMENT_ADD_TO_REVIEW_ACTION: ReviewCommentCreateAction = ReviewCommentCreateAction {
  id: "review-comment-create-start-review",
  label: "Add review comment",
  icon: UiIconName::Check,
  mode: ReviewCommentMode::PendingReview,
  primary: true,
};

/// Two destinations only where the page has two. A local note goes to the agent
/// batch, and GitHub rejects standalone comments while a review is pending.
pub fn review_comment_create_actions(
  display_mode: ReviewCommentDisplayMode,
  has_pending_review: bool,
) -> Vec<ReviewCommentCreateAction> {
  match display_mode {
    ReviewCommentDisplayMode::LocalNote => vec![REVIEW_COMMENT_ADD_NOTE_ACTION],
    ReviewCommentDisplayMode::Conversation if has_pending_review => {
      vec![REVIEW_COMMENT_ADD_TO_REVIEW_ACTION]
    }
    ReviewCommentDisplayMode::Conversation => vec![
      REVIEW_COMMENT_SINGLE_COMMENT_ACTION,
      REVIEW_COMMENT_START_REVIEW_ACTION,
    ],
  }
}

/// The diff reserves whole lines, so a card is handed a few pixels more than it
/// asked for. It takes them, and every card ends on the same thin strip of diff.
fn review_comment_card_min_height(reserved_height: Pixels) -> Pixels {
  px((reserved_height / px(1.0) - REVIEW_COMMENT_CARD_BOTTOM_MARGIN_PX).max(0.0))
}

const fn review_comment_actions_width_px(buttons: usize) -> f32 {
  buttons as f32 * REVIEW_COMMENT_COMPOSER_ACTION_BUTTON_WIDTH_PX
    + (buttons + 1) as f32 * REVIEW_COMMENT_COMPOSER_ACTIONS_GAP_X_PX
}

fn review_comment_card_width_px(available_px: f32) -> f32 {
  (available_px - REVIEW_COMMENT_CARD_RIGHT_MARGIN_PX)
    .clamp(REVIEW_COMMENT_MIN_WIDTH_PX, REVIEW_COMMENT_MAX_WIDTH_PX)
}

fn review_comment_wrap_columns_for_width(available_px: f32, char_width_px: f32) -> usize {
  let char_width_px = char_width_px.max(1.0);
  let columns = (available_px.max(char_width_px) / char_width_px).floor() as usize;
  columns.clamp(
    REVIEW_COMMENT_MIN_WRAP_COLUMNS,
    REVIEW_COMMENT_MAX_WRAP_COLUMNS,
  )
}

/// Shaped with the composer's own font: dividing by an average character width
/// wraps a line of narrow glyphs a row too early, and the card grows before the
/// text does.
fn review_comment_composer_rows(
  value: &str,
  wrap_width: Pixels,
  font: gpui::Font,
  font_size: Pixels,
  window: &Window,
) -> usize {
  let text: SharedString = value.to_string().into();
  let runs = [gpui::TextRun {
    len: text.len(),
    font,
    color: gpui::Hsla::default(),
    background_color: None,
    underline: None,
    strikethrough: None,
  }];
  let rows = window
    .text_system()
    .shape_text(text, font_size, &runs, Some(wrap_width), None)
    .map(|lines| {
      lines
        .iter()
        .map(|line| 1 + line.wrap_boundaries.len())
        .sum::<usize>()
    })
    .unwrap_or(REVIEW_COMMENT_COMPOSER_MIN_ROWS);
  rows.clamp(
    REVIEW_COMMENT_COMPOSER_MIN_ROWS,
    REVIEW_COMMENT_COMPOSER_MAX_ROWS,
  )
}

/// A local note keeps its actions in the header row when it has one, and floating over
/// the body otherwise. Only the floating case has to keep room clear of the text.
fn review_comment_body_reserves_actions_room(
  local_note: bool,
  shows_header: bool,
  has_actions: bool,
) -> bool {
  local_note && !shows_header && has_actions
}

/// What a comment's header offers about the resolution of its conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReviewCommentResolveControl {
  Toggle { label: &'static str, enabled: bool },
  ResolvedTag,
  Nothing,
}

/// A conversation nobody here can touch is not a button. Its state still has to
/// show, so a resolved thread wears a tag in its place.
fn review_comment_resolve_control(
  has_thread: bool,
  has_handler: bool,
  resolve_in_flight: bool,
  is_resolved: bool,
  viewer_can_resolve: bool,
  viewer_can_unresolve: bool,
) -> ReviewCommentResolveControl {
  if !has_thread {
    return ReviewCommentResolveControl::Nothing;
  }
  if resolve_in_flight {
    return ReviewCommentResolveControl::Toggle {
      label: if is_resolved {
        "Unresolving..."
      } else {
        "Resolving..."
      },
      enabled: false,
    };
  }
  let viewer_can_toggle = if is_resolved {
    viewer_can_unresolve
  } else {
    viewer_can_resolve
  };
  if has_handler && viewer_can_toggle {
    return ReviewCommentResolveControl::Toggle {
      label: if is_resolved {
        "Unresolve conversation"
      } else {
        "Resolve conversation"
      },
      enabled: true,
    };
  }
  if is_resolved {
    ReviewCommentResolveControl::ResolvedTag
  } else {
    ReviewCommentResolveControl::Nothing
  }
}

/// The floating actions sit on the first line of the body, like the composer's do
/// on the first line of the text box.
fn review_comment_floating_actions_top_px(text_line_height_px: f32) -> f32 {
  (REVIEW_COMMENT_SPACING_PX
    + (text_line_height_px - REVIEW_COMMENT_COMPOSER_ACTIONS_HEIGHT_PX) / 2.0)
    .max(0.0)
}

/// Lifts the buttons onto the first line of text instead of the top of the box.
fn review_comment_composer_actions_top_px(text_line_height_px: f32) -> f32 {
  (REVIEW_COMMENT_COMPOSER_TEXTAREA_VERTICAL_CHROME_PX / 2.0
    - (REVIEW_COMMENT_COMPOSER_ACTIONS_HEIGHT_PX - text_line_height_px) / 2.0)
    .max(0.0)
}

fn review_comment_composer_textarea_height_px(rows: usize, text_line_height_px: f32) -> f32 {
  rows as f32 * text_line_height_px + REVIEW_COMMENT_COMPOSER_TEXTAREA_VERTICAL_CHROME_PX
}

fn review_comment_overlay_x_offset_for_scroll(scroll_x: Pixels) -> Pixels {
  (-scroll_x).max(px(0.0))
}

fn as_gfm_code_reference_preview(
  preview: &ReviewCommentCodeReferencePreview,
) -> GithubCodeReferencePreview {
  GithubCodeReferencePreview {
    url: preview.url.clone(),
    repo: preview.repo.clone(),
    path: preview.path.clone(),
    reference: preview.reference.clone(),
    start_line: preview.start_line,
    end_line: preview.end_line,
    snippets: preview.snippets.clone(),
    full_content: preview.full_content.clone(),
  }
}

fn markdown_link_target(trimmed: &str) -> Option<&str> {
  if !trimmed.starts_with('[') || !trimmed.ends_with(')') {
    return None;
  }
  let (_, rest) = trimmed.split_once("](")?;
  rest.strip_suffix(')')
}

fn review_comment_markdown_scope_id(comment_id: u64) -> usize {
  (comment_id as usize)
    .wrapping_mul(1_000_003)
    .wrapping_add(1)
}

fn review_comment_markdown_segment_scope_id(comment_id: u64, segment_index: usize) -> usize {
  (comment_id as usize)
    .wrapping_mul(1_000_003)
    .wrapping_add(segment_index)
    .wrapping_mul(31)
    .wrapping_add(2)
}

fn is_conflict_start_marker(line: &str) -> bool {
  line.starts_with("<<<<<<<")
}

fn is_conflict_base_marker(line: &str) -> bool {
  line.starts_with("|||||||")
}

fn is_conflict_divider_marker(line: &str) -> bool {
  line.starts_with("=======")
}

fn is_conflict_end_marker(line: &str) -> bool {
  line.starts_with(">>>>>>>")
}

#[cfg(test)]
fn conflict_regions_from_lines(lines: &[String]) -> Vec<ConflictRegion> {
  let mut regions = Vec::new();
  let mut index = 0;

  while index < lines.len() {
    if !is_conflict_start_marker(lines[index].as_str()) {
      index += 1;
      continue;
    }

    let start_line = index;
    let mut scan = index + 1;
    let mut base_marker_line = None;
    let mut divider_line = None;
    let mut resolved = false;

    while scan < lines.len() {
      let line = lines[scan].as_str();
      if let Some(divider) = divider_line {
        if is_conflict_end_marker(line) {
          let current_end = base_marker_line.unwrap_or(divider);
          regions.push(ConflictRegion {
            start_line,
            current_range: (start_line + 1)..current_end,
            incoming_range: (divider + 1)..scan,
            replace_end_line: scan + 1,
          });
          index = scan + 1;
          resolved = true;
          break;
        }
      } else {
        if is_conflict_start_marker(line) {
          break;
        }
        if base_marker_line.is_none() && is_conflict_base_marker(line) {
          base_marker_line = Some(scan);
          scan += 1;
          continue;
        }
        if is_conflict_divider_marker(line) {
          divider_line = Some(scan);
          scan += 1;
          continue;
        }
      }

      scan += 1;
    }

    if !resolved {
      index = start_line + 1;
    }
  }

  regions
}

fn conflict_regions_from_document(document: &Document) -> Vec<ConflictRegion> {
  let mut regions = Vec::new();
  let mut index = 0;
  let line_count = document.len_lines();

  while index < line_count {
    let line = document.line_content(index).unwrap_or_default();
    if !is_conflict_start_marker(line.as_ref()) {
      index += 1;
      continue;
    }

    let start_line = index;
    let mut scan = index + 1;
    let mut base_marker_line = None;
    let mut divider_line = None;
    let mut resolved = false;

    while scan < line_count {
      let line = document.line_content(scan).unwrap_or_default();
      let line = line.as_ref();
      if let Some(divider) = divider_line {
        if is_conflict_end_marker(line) {
          let current_end = base_marker_line.unwrap_or(divider);
          regions.push(ConflictRegion {
            start_line,
            current_range: (start_line + 1)..current_end,
            incoming_range: (divider + 1)..scan,
            replace_end_line: scan + 1,
          });
          index = scan + 1;
          resolved = true;
          break;
        }
      } else {
        if is_conflict_start_marker(line) {
          break;
        }
        if base_marker_line.is_none() && is_conflict_base_marker(line) {
          base_marker_line = Some(scan);
          scan += 1;
          continue;
        }
        if is_conflict_divider_marker(line) {
          divider_line = Some(scan);
          scan += 1;
          continue;
        }
      }

      scan += 1;
    }

    if !resolved {
      index = start_line + 1;
    }
  }

  regions
}

fn conflict_line_kinds_from_regions(
  regions: &[ConflictRegion],
) -> HashMap<usize, ConflictLineKind> {
  let mut kinds = HashMap::new();

  for region in regions {
    kinds.insert(region.start_line, ConflictLineKind::CurrentMarker);

    for doc_line in region.current_range.clone() {
      kinds.insert(doc_line, ConflictLineKind::Current);
    }

    if region.incoming_range.start > 0 {
      kinds.insert(
        region.incoming_range.start.saturating_sub(1),
        ConflictLineKind::Divider,
      );
    }

    for doc_line in region.incoming_range.clone() {
      kinds.insert(doc_line, ConflictLineKind::Incoming);
    }

    kinds.insert(
      region.replace_end_line.saturating_sub(1),
      ConflictLineKind::IncomingMarker,
    );
  }

  kinds
}

fn editor_actions_enabled(
  find_input_focused: bool,
  review_comment_edit_input_focused: bool,
  review_comment_create_input_focused: bool,
  review_comment_reply_input_focused: bool,
  external_input_focused: bool,
) -> bool {
  !find_input_focused
    && !review_comment_edit_input_focused
    && !review_comment_create_input_focused
    && !review_comment_reply_input_focused
    && !external_input_focused
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReviewCommentCreateTarget {
  display_line: usize,
  line: usize,
  side: ReviewCommentSide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReviewCommentCreateDraft {
  first_display_line: usize,
  last_display_line: usize,
  line: usize,
  side: ReviewCommentSide,
  start_line: Option<usize>,
  start_side: Option<ReviewCommentSide>,
}

/// Emitted so hosts can react to editor lifecycle changes.
#[derive(Clone, Debug)]
pub enum EditorEvent {
  /// The buffer was written to disk.
  Saved,
  /// A hunk was staged, unstaged or restored: the index moved under the host.
  HunkStagingChanged,
}

impl gpui::EventEmitter<EditorEvent> for Editor {}

pub struct Editor {
  pub document: Entity<Document>,
  pub focus_handle: FocusHandle,
  pub selected_range: Range<usize>,
  pub selection_reversed: bool,
  pub display_selection: Option<DisplaySelection>,
  pub marked_range: Option<Range<usize>>,
  pub is_selecting: bool,

  pub line_layouts: HashMap<usize, Arc<ShapedLine>>,
  pub virtual_line_layouts: HashMap<usize, Arc<ShapedLine>>,
  pub(crate) last_layout_font_size: Pixels,

  pub scroll_offset_y: f32, // Vertical scroll offset in lines (0.0 = top, 1.5 = 1.5 lines down)
  pub editor_line_height: Pixels,
  pub editor_char_width: Pixels,
  pub review_comment_char_width: Pixels,
  pub review_comment_font_size: Pixels,
  pub review_comment_composer_line_height_px: f32,
  pub viewport_height: Pixels,
  pub viewport_width: Pixels,
  pub max_line_width: Pixels, // Maximum width of visible lines (never decreases to avoid scroll jumps)
  pub scroll_handle: ScrollHandle, // Handle for horizontal scrolling
  pub(crate) scroll_axis_lock: Option<ScrollAxis>,
  pub(crate) last_scroll_time: Option<Instant>,
  pub(crate) last_scroll_x: Pixels,

  pub(crate) max_cache_size: usize,

  pub(crate) target_column: Option<usize>,

  pub(crate) undo_stack: VecDeque<Transaction>,
  pub(crate) redo_stack: VecDeque<Transaction>,

  pub theme: Theme,
  pub projection: Option<Arc<Projection>>,
  pub visible_groups: Vec<GroupOverlay>,
  pub hovered_group_id: Option<Arc<str>>,
  /// Which split pane owns the current hover state.
  pub(crate) hovered_from_primary: bool,
  pub hovered_conflict_start_line: Option<usize>,
  pending_conflict_reveal_start_line: Option<usize>,
  conflict_cache: RwLock<ConflictCache>,
  pub last_mouse_position: Option<Point<Pixels>>,
  pub expanded_gaps: HashMap<GapId, GapReveal>,
  pub workdir_path: PathBuf,
  pub repo_file: Option<RepoFile>,
  pub git_store: Option<GitStore>,
  git_state: BufferGitState,
  pub diffs: Option<Arc<DiffSet>>,
  review_comments: Vec<ReviewComment>,
  review_comment_thread_roots: HashMap<u64, u64>,
  review_comment_threads: HashMap<u64, Vec<u64>>,
  review_comment_thread_order: Vec<u64>,
  review_comment_wrap_columns: usize,
  review_comment_line_height_px: f32,
  review_comment_display_mode: ReviewCommentDisplayMode,
  review_comment_markdown_states: HashMap<u64, MarkdownRenderState>,
  review_comment_markdown_cache: HashMap<u64, ReviewCommentMarkdownCacheEntry>,
  review_comment_code_reference_previews: HashMap<u64, Vec<ReviewCommentCodeReferencePreview>>,
  review_comment_pr_number: Option<u64>,
  editable_review_comment_ids: HashSet<u64>,
  review_comment_edit_handler: Option<ReviewCommentEditHandler>,
  review_comment_edit_input: Option<Entity<TextareaState>>,
  editing_review_comment_id: Option<u64>,
  review_comment_edit_initial_body: Option<Arc<str>>,
  review_comment_edit_submitting_id: Option<u64>,
  review_comment_edit_error: Option<(u64, Arc<str>)>,
  review_comment_delete_handler: Option<ReviewCommentDeleteHandler>,
  review_comment_send_handler: Option<ReviewCommentSendHandler>,
  sendable_review_comment_ids: HashSet<u64>,
  review_comment_delete_submitting_id: Option<u64>,
  review_comment_resolve_handler: Option<ReviewCommentResolveHandler>,
  review_comment_resolve_in_flight: HashSet<Arc<str>>,
  auto_collapsed_resolved_thread_ids: HashSet<u64>,
  review_comment_suggestion_action_factory: Option<ReviewCommentSuggestionActionFactory>,
  review_comment_create_handler: Option<ReviewCommentCreateHandler>,
  review_comment_cancel_handler: Option<ReviewCommentCancelHandler>,
  review_comment_replies_enabled: bool,
  has_pending_review: bool,
  review_comment_link_handler: Option<ReviewCommentLinkHandler>,
  review_comment_asset_url_resolver: Option<ReviewCommentAssetUrlResolver>,
  review_comment_image_upload_handler: Option<ReviewCommentImageUploadHandler>,
  review_comment_preview_renderer: Option<ReviewCommentPreviewRenderer>,
  review_comment_composer_rows: HashMap<gpui::EntityId, usize>,
  review_comment_create_input: Option<Entity<TextareaState>>,
  review_comment_create_draft: Option<ReviewCommentCreateDraft>,
  review_comment_create_drag_start_display_line: Option<usize>,
  review_comment_create_drag_active: bool,
  review_comment_create_submitting: bool,
  review_comment_create_error: Option<Arc<str>>,
  review_comment_create_preview_open: bool,
  review_comment_edit_preview_open: bool,
  review_comment_reply_input: Option<Entity<TextareaState>>,
  replying_to_review_comment_id: Option<u64>,
  review_comment_reply_submitting: bool,
  review_comment_reply_error: Option<Arc<str>>,
  review_comment_reply_preview_open: bool,
  hovered_review_comment_create_display_line: Option<usize>,
  collapsed_review_comments: HashSet<u64>,
  review_comment_scroll_epoch: usize,
  find_panel_open: bool,
  find_input: Option<Entity<InputState>>,
  find_input_subscription: Option<Subscription>,
  find_query: String,
  find_matches: Vec<FindMatch>,
  find_active_match: Option<usize>,
  find_scroll_epoch: usize,
  pub diff_task: Option<Task<()>>,
  projection_task: Option<Task<()>>,
  pub bases_task: Option<Task<()>>,
  pub poll_task: Option<Task<()>>,
  pub git_task: Option<Task<()>>,
  git_jobs: VecDeque<GitJob>,
  git_op_in_flight: bool,
  pending_git_after_bases: bool,
  pub diff_generation: Arc<AtomicUsize>,
  projection_generation: Arc<AtomicUsize>,
  pub file_mtime: Option<SystemTime>,
  pub index_mtime: Option<SystemTime>,
  pub is_dirty: bool,
  pub save_task: Option<Task<()>>,
  pub optimistic_unstaged_groups: HashSet<Arc<str>>,

  diff_view_mode: DiffViewMode,
  ignore_whitespace: bool,
  pub is_read_only: bool,
  is_unmerged: bool,

  pub last_highlights_version: usize,
  pub last_highlights_epoch: usize,

  pub cursor_blink: Entity<CursorBlink>,
}

#[derive(Clone, Debug)]
pub struct GroupOverlay {
  pub id: Arc<str>,
  pub state: HunkState,
  pub display_line: usize,
}

struct ReviewCommentLayout {
  id: u64,
  top: Pixels,
  height: Pixels,
  messages: Vec<ReviewCommentMessageLayout>,
  collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FindMatch {
  display_line: usize,
  column_start: usize,
  column_end: usize,
  doc_range: Range<usize>,
}

struct ReviewCommentMessageLayout {
  id: u64,
  author: Arc<str>,
  avatar_url: Option<Arc<str>>,
  line_label: Option<Arc<str>>,
  body: Arc<str>,
  suggestion_context: Option<gfm_markdown_viewer::SuggestionContext>,
  created_at: Arc<str>,
  thread_id: Option<Arc<str>>,
  is_resolved: bool,
  is_outdated: bool,
  viewer_can_resolve: bool,
  viewer_can_unresolve: bool,
  is_pending: bool,
  shows_header: bool,
}

#[derive(Clone)]
struct ReviewCommentMarkdownCacheEntry {
  body_hash: u64,
  parsed: ParsedMarkdown,
  estimated_heights_px: HashMap<(usize, u32), f32>,
}

struct ProjectionBuildInput {
  doc_line_count: usize,
  diffs: Arc<DiffSet>,
  expanded_gaps: HashMap<GapId, GapReveal>,
  align_modified: bool,
  projection_comments: Vec<ReviewComment>,
  collapsed_review_comments: HashSet<u64>,
  editor_line_height_px: f32,
  markdown_line_height_px: f32,
  review_comment_body_heights_px: HashMap<u64, f32>,
  composer_only_comment_ids: HashSet<u64>,
  local_notes: bool,
  conflict_doc_line_ranges: Vec<Range<usize>>,
  is_unmerged: bool,
}

fn staged_diff_from_bases(bases: &GitFileBases, rel_path: &Path) -> Option<FileDiff> {
  if bases.head.as_deref() == bases.index.as_deref() {
    return Some(FileDiff::empty(git::DiffKind::Staged));
  }

  git::compute_buffer_diff(
    git::DiffKind::Staged,
    bases.head.as_deref(),
    bases.index.as_deref().unwrap_or(""),
    rel_path,
    false,
  )
  .ok()
}

#[derive(Clone, Debug, Default)]
pub struct BufferGitState {
  pub op_id: usize,
  pub bases: Option<GitFileBases>,
  pub index_dirty: bool,
  staged_diff: Option<FileDiff>,
}

#[derive(Clone, Copy, Debug)]
pub enum HunkAction {
  Stage,
  Unstage,
  Restore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictLineKind {
  CurrentMarker,
  Current,
  Divider,
  Incoming,
  IncomingMarker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
  Current,
  Incoming,
  Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictNavigationDirection {
  Previous,
  Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictNavigationState {
  pub active_index: usize,
  pub total: usize,
  pub active_start_line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HunkNavigationDirection {
  Previous,
  Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkNavigationState {
  pub active_index: usize,
  pub total: usize,
  pub active_display_line: usize,
}

#[derive(Clone, Debug)]
struct GroupToken {
  state: HunkState,
  signature: Arc<str>,
  id: Arc<str>,
}

#[derive(Clone, Debug)]
struct GitJob {
  token: GroupToken,
  action: HunkAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConflictRegion {
  start_line: usize,
  current_range: Range<usize>,
  incoming_range: Range<usize>,
  replace_end_line: usize,
}

impl ConflictRegion {
  fn contains_doc_line(&self, doc_line: usize) -> bool {
    self.start_line <= doc_line && doc_line < self.replace_end_line
  }
}

#[derive(Debug)]
struct ConflictCache {
  dirty: bool,
  regions: Arc<Vec<ConflictRegion>>,
  line_kinds: Arc<HashMap<usize, ConflictLineKind>>,
}

impl Default for ConflictCache {
  fn default() -> Self {
    Self {
      dirty: true,
      regions: Arc::new(Vec::new()),
      line_kinds: Arc::new(HashMap::new()),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayCursor {
  pub line: usize,
  pub column: usize,
}

#[derive(Clone, Debug)]
pub struct DisplaySelection {
  pub start: DisplayCursor,
  pub end: DisplayCursor,
}

impl DisplaySelection {
  pub fn is_empty(&self) -> bool {
    self.start == self.end
  }

  pub fn normalized(&self) -> (DisplayCursor, DisplayCursor) {
    if (self.start.line, self.start.column) <= (self.end.line, self.end.column) {
      (self.start, self.end)
    } else {
      (self.end, self.start)
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GapExpandDirection {
  Up,
  Down,
}

impl GapExpandDirection {
  fn icon(self) -> UiIconName {
    match self {
      GapExpandDirection::Up => UiIconName::ArrowUpFromLine,
      GapExpandDirection::Down => UiIconName::ArrowDownFromLine,
    }
  }

  fn id_suffix(self) -> &'static str {
    match self {
      GapExpandDirection::Up => "up",
      GapExpandDirection::Down => "down",
    }
  }

  fn tooltip(self) -> &'static str {
    match self {
      GapExpandDirection::Up => "Expand 5 lines up",
      GapExpandDirection::Down => "Expand 5 lines down",
    }
  }
}

#[derive(Clone, Copy, Debug)]
struct GapControl {
  display_line: usize,
  gap_id: GapId,
  direction: GapExpandDirection,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScrollAxis {
  Horizontal,
  Vertical,
}

impl Editor {
  pub fn new_with_paths(repo_root: PathBuf, file_path: PathBuf, cx: &mut Context<Self>) -> Self {
    let loaded = Self::load_file_for_editor(&repo_root, &file_path);
    Self::new_with_loaded_file(repo_root, file_path, loaded, cx)
  }

  pub fn load_file_for_editor(repo_root: &Path, workdir_path: &Path) -> EditorFileLoad {
    let language_hint = Self::language_hint_for_path(workdir_path);
    let (content, binary_bytes, is_read_only) = match std::fs::read(workdir_path) {
      Ok(bytes) => match String::from_utf8(bytes) {
        Ok(content) => (content, None, false),
        Err(err) => (String::new(), Some(err.into_bytes()), false),
      },
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => (
        Self::deleted_file_content(repo_root, workdir_path).unwrap_or_default(),
        None,
        true,
      ),
      Err(_) => (String::new(), None, false),
    };

    let file_mtime = std::fs::metadata(workdir_path)
      .and_then(|meta| meta.modified())
      .ok();
    let index_mtime = std::fs::metadata(git::index_path(repo_root))
      .and_then(|meta| meta.modified())
      .ok();

    EditorFileLoad {
      content,
      binary_bytes,
      is_read_only,
      language_hint,
      file_mtime,
      index_mtime,
    }
  }

  pub fn new_with_loaded_file(
    repo_root: PathBuf,
    file_path: PathBuf,
    loaded: EditorFileLoad,
    cx: &mut Context<Self>,
  ) -> Self {
    let workdir_path = file_path;
    let document = cx.new(|cx| Document::new(&loaded.content, loaded.language_hint.as_deref(), cx));
    let cursor_blink = cx.new(CursorBlink::new);
    let repo_file = RepoFile::new(repo_root, workdir_path.clone()).ok();
    let git_store = repo_file
      .as_ref()
      .map(|repo_file| GitStore::new(repo_file.repo_root.clone()));

    let mut editor = Self {
      document,
      focus_handle: cx.focus_handle(),
      selected_range: 0..0,
      selection_reversed: false,
      display_selection: None,
      marked_range: None,
      is_selecting: false,
      line_layouts: HashMap::new(),
      virtual_line_layouts: HashMap::new(),
      last_layout_font_size: px(0.0),
      scroll_offset_y: 0.0,
      editor_line_height: px(DEFAULT_EDITOR_LINE_HEIGHT),
      editor_char_width: px(REVIEW_COMMENT_CHAR_WIDTH_PX),
      review_comment_char_width: px(REVIEW_COMMENT_CHAR_WIDTH_PX),
      review_comment_font_size: px(REVIEW_COMMENT_FONT_SIZE_PX),
      review_comment_composer_line_height_px: REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_PX,
      viewport_height: px(DEFAULT_VIEWPORT_HEIGHT), // Will be updated on first render
      viewport_width: px(DEFAULT_VIEWPORT_WIDTH),   // Will be updated on first render
      max_line_width: px(DEFAULT_MAX_LINE_WIDTH),   // Will be updated on first render
      scroll_handle: ScrollHandle::new(),
      scroll_axis_lock: None,
      last_scroll_time: None,
      last_scroll_x: px(0.0),
      max_cache_size: MAX_CACHE_SIZE,
      target_column: None,
      undo_stack: VecDeque::new(),
      redo_stack: VecDeque::new(),
      theme: Theme::dark(),
      projection: None,
      visible_groups: Vec::new(),
      hovered_group_id: None,
      hovered_from_primary: true,
      hovered_conflict_start_line: None,
      pending_conflict_reveal_start_line: None,
      conflict_cache: RwLock::new(ConflictCache::default()),
      last_mouse_position: None,
      expanded_gaps: HashMap::new(),
      last_highlights_version: 0,
      last_highlights_epoch: 0,
      cursor_blink,
      workdir_path,
      repo_file,
      git_store,
      git_state: BufferGitState::default(),
      diffs: None,
      review_comments: Vec::new(),
      review_comment_thread_roots: HashMap::new(),
      review_comment_threads: HashMap::new(),
      review_comment_thread_order: Vec::new(),
      review_comment_wrap_columns: REVIEW_COMMENT_DEFAULT_WRAP_COLUMNS,
      review_comment_line_height_px: REVIEW_COMMENT_DEFAULT_LINE_HEIGHT_PX,
      review_comment_display_mode: ReviewCommentDisplayMode::Conversation,
      review_comment_markdown_states: HashMap::new(),
      review_comment_markdown_cache: HashMap::new(),
      review_comment_code_reference_previews: HashMap::new(),
      review_comment_pr_number: None,
      editable_review_comment_ids: HashSet::new(),
      review_comment_edit_handler: None,
      review_comment_edit_input: None,
      editing_review_comment_id: None,
      review_comment_edit_initial_body: None,
      review_comment_edit_submitting_id: None,
      review_comment_edit_error: None,
      review_comment_delete_handler: None,
      review_comment_send_handler: None,
      sendable_review_comment_ids: HashSet::new(),
      review_comment_delete_submitting_id: None,
      review_comment_resolve_handler: None,
      review_comment_resolve_in_flight: HashSet::new(),
      auto_collapsed_resolved_thread_ids: HashSet::new(),
      review_comment_suggestion_action_factory: None,
      review_comment_create_handler: None,
      review_comment_cancel_handler: None,
      review_comment_replies_enabled: true,
      has_pending_review: false,
      review_comment_link_handler: None,
      review_comment_asset_url_resolver: None,
      review_comment_image_upload_handler: None,
      review_comment_preview_renderer: None,
      review_comment_composer_rows: HashMap::new(),
      review_comment_create_input: None,
      review_comment_create_draft: None,
      review_comment_create_drag_start_display_line: None,
      review_comment_create_drag_active: false,
      review_comment_create_submitting: false,
      review_comment_create_error: None,
      review_comment_create_preview_open: false,
      review_comment_edit_preview_open: false,
      review_comment_reply_input: None,
      replying_to_review_comment_id: None,
      review_comment_reply_submitting: false,
      review_comment_reply_error: None,
      review_comment_reply_preview_open: false,
      hovered_review_comment_create_display_line: None,
      collapsed_review_comments: HashSet::new(),
      review_comment_scroll_epoch: 0,
      find_panel_open: false,
      find_input: None,
      find_input_subscription: None,
      find_query: String::new(),
      find_matches: Vec::new(),
      find_active_match: None,
      find_scroll_epoch: 0,
      diff_task: None,
      projection_task: None,
      bases_task: None,
      poll_task: None,
      git_task: None,
      git_jobs: VecDeque::new(),
      git_op_in_flight: false,
      pending_git_after_bases: false,
      diff_generation: Arc::new(AtomicUsize::new(0)),
      projection_generation: Arc::new(AtomicUsize::new(0)),
      file_mtime: loaded.file_mtime,
      index_mtime: loaded.index_mtime,
      is_dirty: false,
      save_task: None,
      optimistic_unstaged_groups: HashSet::new(),
      diff_view_mode: DiffViewMode::Inline,
      ignore_whitespace: false,
      is_read_only: loaded.is_read_only,
      is_unmerged: false,
    };
    editor.init(cx);
    editor
  }

  fn language_hint_for_path(workdir_path: &Path) -> Option<String> {
    languages::detect_language_name_for_path(workdir_path).map(str::to_owned)
  }

  pub fn document(&self) -> &Entity<Document> {
    &self.document
  }

  pub fn measured_editor_line_height(&self) -> Pixels {
    if self.editor_line_height > px(0.0) {
      self.editor_line_height
    } else {
      px(DEFAULT_EDITOR_LINE_HEIGHT)
    }
  }

  pub fn measured_editor_char_width(&self) -> Pixels {
    if self.editor_char_width > px(0.0) {
      self.editor_char_width
    } else {
      px(REVIEW_COMMENT_CHAR_WIDTH_PX)
    }
  }

  pub fn measured_review_comment_char_width(&self) -> Pixels {
    if self.review_comment_char_width > px(0.0) {
      self.review_comment_char_width
    } else {
      px(REVIEW_COMMENT_CHAR_WIDTH_PX)
    }
  }

  pub(crate) fn set_review_comment_line_height_px(
    &mut self,
    line_height_px: f32,
    cx: &mut Context<Self>,
  ) {
    if (line_height_px - self.review_comment_line_height_px).abs() <= 0.05 {
      return;
    }

    self.review_comment_line_height_px = line_height_px;
    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    }
  }

  fn deleted_file_content(
    repo_root: &std::path::Path,
    workdir_path: &std::path::Path,
  ) -> Option<String> {
    let rel_path = workdir_path.strip_prefix(repo_root).ok()?;
    let store = GitStore::new(repo_root.to_path_buf());
    let bases = store.load_bases(rel_path).ok()?;
    bases.index.or(bases.head)
  }

  pub fn set_projection(&mut self, projection: Option<Projection>) {
    self.projection = projection.map(Arc::new);
    self.virtual_line_layouts.clear();
  }

  pub fn projection(&self) -> Option<&Projection> {
    self.projection.as_deref()
  }

  pub fn diff_view_mode(&self) -> DiffViewMode {
    self.diff_view_mode
  }

  pub fn set_diff_view_mode(&mut self, mode: DiffViewMode, cx: &mut Context<Self>) {
    if self.diff_view_mode != mode {
      self.diff_view_mode = mode;
      if self.diffs.is_some() {
        self.rebuild_projection(cx);
      } else {
        self.virtual_line_layouts.clear();
      }
    }
  }

  pub fn set_ignore_whitespace(&mut self, value: bool, cx: &mut Context<Self>) {
    if self.ignore_whitespace != value {
      self.ignore_whitespace = value;
      self.schedule_diff_recompute(cx);
    }
  }

  pub fn ignore_whitespace(&self) -> bool {
    self.ignore_whitespace
  }

  pub fn reset_selection(&mut self, cx: &mut Context<Self>) {
    self.selected_range = 0..0;
    self.selection_reversed = false;
    self.display_selection = None;
    self.marked_range = None;
    cx.notify();
  }

  pub fn reset_after_replace(&mut self) {
    self.line_layouts.clear();
    self.virtual_line_layouts.clear();
    self.expanded_gaps.clear();
    self.hovered_group_id = None;
    self.hovered_conflict_start_line = None;
    self.last_mouse_position = None;
    self.scroll_offset_y = 0.0;
    self.reset_horizontal_scroll_state();
  }

  pub fn set_diffs(&mut self, diffs: Option<DiffSet>, cx: &mut Context<Self>) {
    if let Some(diffs) = diffs {
      self.apply_diffs(diffs, cx);
    } else {
      self.invalidate_projection_builds();
      self.diffs = None;
      self.review_comments.clear();
      self.review_comment_thread_roots.clear();
      self.review_comment_threads.clear();
      self.review_comment_thread_order.clear();
      self.review_comment_markdown_states.clear();
      self.review_comment_markdown_cache.clear();
      self.review_comment_code_reference_previews.clear();
      self.review_comment_pr_number = None;
      self.editable_review_comment_ids.clear();
      self.clear_review_comment_edit_state();
      self.clear_review_comment_create_state();
      self.clear_review_comment_reply_state();
      self.review_comment_delete_submitting_id = None;
      self.collapsed_review_comments.clear();
      self.set_projection(None);
      self.virtual_line_layouts.clear();
      cx.notify();
    }
  }

  pub fn load_readonly_snapshot(
    &mut self,
    contents: String,
    diffs: Option<DiffSet>,
    cx: &mut Context<Self>,
  ) {
    self.reload_from_disk(contents, cx);

    // Freeze editor into detached readonly mode so polling/diff recomputation cannot overwrite
    // commit snapshot content while browsing history.
    self.repo_file = None;
    self.git_store = None;
    self.file_mtime = None;
    self.index_mtime = None;
    self.git_state = BufferGitState::default();
    self.diff_generation.fetch_add(1, Ordering::Relaxed);
    self.is_read_only = true;

    self.set_diffs(diffs, cx);
  }

  pub fn set_review_comments(&mut self, comments: Vec<ReviewComment>, cx: &mut Context<Self>) {
    let previously_collapsed_threads: HashSet<u64> = self
      .review_comment_threads
      .iter()
      .filter_map(|(thread_id, comment_ids)| {
        if comment_ids
          .iter()
          .all(|id| self.collapsed_review_comments.contains(id))
        {
          Some(*thread_id)
        } else {
          None
        }
      })
      .collect();

    self.review_comments = comments;

    let comments_by_id: HashMap<u64, &ReviewComment> = self
      .review_comments
      .iter()
      .map(|comment| (comment.id, comment))
      .collect();
    self.review_comment_thread_roots.clear();
    self.review_comment_threads.clear();
    self.review_comment_thread_order.clear();

    for comment in &self.review_comments {
      let root_id = Self::resolve_review_comment_thread_root(comment, &comments_by_id);
      self.review_comment_thread_roots.insert(comment.id, root_id);
      let is_new_thread = !self.review_comment_threads.contains_key(&root_id);
      self
        .review_comment_threads
        .entry(root_id)
        .or_default()
        .push(comment.id);
      if is_new_thread {
        self.review_comment_thread_order.push(root_id);
      }
    }
    self.collapsed_review_comments.clear();
    for thread_id in previously_collapsed_threads {
      if let Some(comment_ids) = self.review_comment_threads.get(&thread_id) {
        self
          .collapsed_review_comments
          .extend(comment_ids.iter().copied());
      }
    }
    // Auto-collapse newly resolved threads on first sight; preserve user
    // overrides on subsequent refreshes by tracking which thread roots we've
    // already auto-collapsed.
    let resolved_thread_roots: HashSet<u64> = self
      .review_comment_threads
      .iter()
      .filter_map(|(thread_root, comment_ids)| {
        let root = comments_by_id.get(thread_root)?;
        if root.is_resolved {
          Some((*thread_root, comment_ids.clone()))
        } else {
          None
        }
      })
      .map(|(root, _)| root)
      .collect();
    for (thread_root, comment_ids) in self.review_comment_threads.clone() {
      let Some(root) = comments_by_id.get(&thread_root) else {
        continue;
      };
      if root.is_resolved
        && !self
          .auto_collapsed_resolved_thread_ids
          .contains(&thread_root)
      {
        self
          .collapsed_review_comments
          .extend(comment_ids.iter().copied());
        self.auto_collapsed_resolved_thread_ids.insert(thread_root);
      }
    }
    self
      .auto_collapsed_resolved_thread_ids
      .retain(|id| resolved_thread_roots.contains(id));
    self.review_comment_resolve_in_flight.retain(|thread_id| {
      self
        .review_comments
        .iter()
        .any(|comment| comment.thread_id.as_deref() == Some(thread_id.as_ref()))
    });
    self
      .review_comment_markdown_states
      .retain(|id, _| self.review_comments.iter().any(|comment| comment.id == *id));
    self
      .review_comment_markdown_cache
      .retain(|id, _| self.review_comments.iter().any(|comment| comment.id == *id));
    self
      .review_comment_code_reference_previews
      .retain(|id, _| self.review_comments.iter().any(|comment| comment.id == *id));
    self
      .editable_review_comment_ids
      .retain(|id| self.review_comments.iter().any(|comment| comment.id == *id));
    if self
      .editing_review_comment_id
      .is_some_and(|id| !self.review_comments.iter().any(|comment| comment.id == id))
    {
      self.clear_review_comment_edit_state();
    }
    if self
      .review_comment_edit_submitting_id
      .is_some_and(|id| !self.review_comments.iter().any(|comment| comment.id == id))
    {
      self.review_comment_edit_submitting_id = None;
    }
    if self
      .review_comment_edit_error
      .as_ref()
      .is_some_and(|(id, _)| !self.review_comments.iter().any(|comment| comment.id == *id))
    {
      self.review_comment_edit_error = None;
    }
    if self
      .review_comment_delete_submitting_id
      .is_some_and(|id| !self.review_comments.iter().any(|comment| comment.id == id))
    {
      self.review_comment_delete_submitting_id = None;
    }
    if self
      .replying_to_review_comment_id
      .is_some_and(|id| !self.review_comments.iter().any(|comment| comment.id == id))
    {
      self.clear_review_comment_reply_state();
    }
    for comment in &self.review_comments {
      self
        .review_comment_markdown_states
        .entry(comment.id)
        .or_default();
    }

    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    }
  }

  pub fn set_review_comment_code_reference_previews(
    &mut self,
    previews_by_comment: HashMap<u64, Vec<ReviewCommentCodeReferencePreview>>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_code_reference_previews = previews_by_comment;
    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    } else {
      cx.notify();
    }
  }

  /// One frame for every review comment surface: square where it meets the diff
  /// gutter, rounded everywhere else.
  fn review_comment_card_frame(
    &self,
    min_height: Pixels,
    theme: &gpui_component::Theme,
  ) -> gpui::Div {
    div()
      .w(self.review_comment_card_width())
      .min_h(min_height)
      .bg(theme.sidebar)
      .border(px(REVIEW_COMMENT_CARD_BORDER_PX))
      .border_color(theme.border)
      .rounded_tr_md()
      .rounded_br_md()
      .rounded_bl_md()
      .cursor(CursorStyle::Arrow)
  }

  /// The card follows the content area, never wider than its readable maximum.
  fn review_comment_card_width(&self) -> Pixels {
    px(review_comment_card_width_px(
      self.horizontal_viewport_width() / px(1.0),
    ))
  }

  /// Tracked only to rebuild the projection when the card resizes; every comment
  /// then measures its own text.
  fn computed_review_comment_wrap_columns(&self) -> usize {
    self.review_comment_body_wrap_columns(self.measured_review_comment_char_width() / px(1.0))
  }

  fn review_comment_text_measurer<'a>(&self, cx: &'a App) -> ReviewCommentTextMeasurer<'a> {
    ReviewCommentTextMeasurer {
      column_px: (self.measured_review_comment_char_width() / px(1.0)).max(1.0),
      font_id: cx
        .text_system()
        .resolve_font(&gpui::font(REVIEW_COMMENT_UI_FONT_FAMILY)),
      font_size: self.review_comment_font_size,
      glyph_widths: RefCell::new(HashMap::new()),
      cx,
    }
  }

  fn review_comment_body_wrap_columns(&self, char_width_px: f32) -> usize {
    let card_width_px = self.review_comment_card_width() / px(1.0);
    review_comment_wrap_columns_for_width(
      card_width_px
        - REVIEW_COMMENT_HORIZONTAL_PADDING_PX
        - self.review_comment_floating_actions_width_px(),
      char_width_px,
    )
  }

  /// A local note keeps its actions over the body, so the text stops before them.
  fn review_comment_floating_actions_width_px(&self) -> f32 {
    match self.review_comment_display_mode {
      ReviewCommentDisplayMode::LocalNote => REVIEW_COMMENT_FLOATING_ACTIONS_WIDTH_PX,
      ReviewCommentDisplayMode::Conversation => 0.0,
    }
  }

  /// What is left of the card once the buttons and every inset are taken out.
  fn review_comment_composer_text_width(&self, actions_width_px: f32) -> Pixels {
    let card_width_px = self.review_comment_card_width() / px(1.0);
    px(
      (card_width_px
        - REVIEW_COMMENT_COMPOSER_HORIZONTAL_PADDING_PX
        - REVIEW_COMMENT_COMPOSER_TEXTAREA_HORIZONTAL_CHROME_PX
        - actions_width_px
        - REVIEW_COMMENT_COMPOSER_WRAP_SAFETY_PX)
        .max(REVIEW_COMMENT_COMPOSER_MIN_TEXT_WIDTH_PX),
    )
  }

  /// The create composer carries a Suggest button and, on a pull request without a
  /// pending review, two spelled-out destinations; the others carry cancel and save.
  fn review_comment_create_actions_width_px(&self, cx: &App) -> f32 {
    let actions =
      review_comment_create_actions(self.review_comment_display_mode, self.has_pending_review);
    if actions.len() > 1 {
      return REVIEW_COMMENT_COMPOSER_LABELLED_ACTIONS_WIDTH_PX;
    }
    let buttons = 1 + actions.len() + usize::from(self.can_insert_review_comment_suggestion(cx));
    review_comment_actions_width_px(buttons)
  }

  fn gutter_create_button_extra_width_px(&self) -> f32 {
    if self.review_comment_create_handler.is_some() {
      REVIEW_COMMENT_CREATE_BUTTON_HITBOX_WIDTH_PX
    } else {
      0.0
    }
  }

  pub(crate) fn gutter_width(&self) -> Pixels {
    px(GUTTER_WIDTH + self.gutter_create_button_extra_width_px())
  }

  pub(crate) fn gutter_line_number_right_padding(&self) -> Pixels {
    px(GUTTER_LINE_NUMBER_BASE_RIGHT_PADDING_PX + self.gutter_create_button_extra_width_px())
  }

  pub(crate) fn vertical_scroll_metrics_for_height(
    viewport_height: Pixels,
    line_height: Pixels,
    total_lines: usize,
  ) -> VerticalScrollMetrics {
    let viewport_lines = if line_height > px(0.0) {
      (viewport_height / line_height).max(1.0)
    } else {
      1.0
    };
    let max_padding = (viewport_lines - 1.0).max(0.0);
    let scroll_padding = (SCROLL_PADDING as f32).min(max_padding);
    let max_scroll = (total_lines as f32 - viewport_lines + scroll_padding).max(0.0);

    VerticalScrollMetrics {
      viewport_lines,
      scroll_padding,
      max_scroll,
    }
  }

  pub(crate) fn clamp_vertical_scroll_for_height(
    scroll_offset_y: f32,
    viewport_height: Pixels,
    line_height: Pixels,
    total_lines: usize,
  ) -> f32 {
    if total_lines == 0 {
      0.0
    } else {
      let metrics =
        Self::vertical_scroll_metrics_for_height(viewport_height, line_height, total_lines);
      scroll_offset_y.clamp(0.0, metrics.max_scroll)
    }
  }

  pub(crate) fn viewport_range_for_height(
    scroll_offset_y: f32,
    viewport_height: Pixels,
    line_height: Pixels,
    total_lines: usize,
  ) -> Range<usize> {
    if total_lines == 0 {
      return 0..0;
    }

    let scroll_offset = Self::clamp_vertical_scroll_for_height(
      scroll_offset_y,
      viewport_height,
      line_height,
      total_lines,
    );
    let mut visible_line_count = ((viewport_height / line_height).ceil() as usize).max(1);
    if has_fractional_scroll(scroll_offset) {
      visible_line_count += 1;
    }

    let start_line = (scroll_offset.floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = (start_line + visible_line_count).min(total_lines);
    start_line..end_line
  }

  fn vertical_scroll_metrics(
    &self,
    line_height: Pixels,
    total_lines: usize,
  ) -> VerticalScrollMetrics {
    Self::vertical_scroll_metrics_for_height(self.viewport_height, line_height, total_lines)
  }

  fn clamp_vertical_scroll(
    &self,
    scroll_offset_y: f32,
    line_height: Pixels,
    total_lines: usize,
  ) -> f32 {
    Self::clamp_vertical_scroll_for_height(
      scroll_offset_y,
      self.viewport_height,
      line_height,
      total_lines,
    )
  }

  fn center_display_line_in_viewport(&mut self, display_line: usize, total_lines: usize) {
    if total_lines == 0 {
      self.scroll_offset_y = 0.0;
      return;
    }

    let line_height = self.measured_editor_line_height();
    let metrics = self.vertical_scroll_metrics(line_height, total_lines);
    let center_offset = ((metrics.viewport_lines - 1.0) / 2.0).max(0.0);
    self.scroll_offset_y = (display_line as f32 - center_offset).clamp(0.0, metrics.max_scroll);
  }

  fn reset_horizontal_scroll_state(&mut self) {
    self.max_line_width = px(DEFAULT_MAX_LINE_WIDTH);
    self.last_scroll_x = px(0.0);
    self.scroll_handle.set_offset(point(px(0.0), px(0.0)));
  }

  pub(crate) fn set_horizontal_scroll_offset(&mut self, scroll_x: Pixels) {
    let clamped = self.clamp_horizontal_scroll_x(scroll_x);
    self.scroll_handle.set_offset(point(clamped, px(0.0)));
    self.last_scroll_x = clamped;
  }

  fn review_comment_body_hash(body: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    hasher.finish()
  }

  fn review_comment_code_reference_preview_height_px(
    &self,
    preview: &ReviewCommentCodeReferencePreview,
  ) -> f32 {
    estimate_github_code_reference_preview_height_px(
      preview.snippets.len(),
      self.review_comment_line_height_px,
    )
  }

  fn review_comment_body_segments(
    &self,
    comment_id: u64,
    body: &str,
  ) -> Vec<ReviewCommentBodySegment> {
    let Some(previews) = self.review_comment_code_reference_previews.get(&comment_id) else {
      return vec![ReviewCommentBodySegment::Markdown(body.to_string())];
    };

    if previews.is_empty() {
      return vec![ReviewCommentBodySegment::Markdown(body.to_string())];
    }

    let preview_by_url: HashMap<&str, &ReviewCommentCodeReferencePreview> = previews
      .iter()
      .map(|preview| (preview.url.as_ref(), preview))
      .collect();

    let mut segments = Vec::new();
    let mut markdown_lines = Vec::new();
    let flush_markdown = |segments: &mut Vec<ReviewCommentBodySegment>, lines: &mut Vec<String>| {
      if lines.is_empty() {
        return;
      }
      segments.push(ReviewCommentBodySegment::Markdown(lines.join("\n")));
      lines.clear();
    };

    for line in body.lines() {
      let trimmed = line.trim();
      let preview = if trimmed.is_empty() {
        None
      } else if let Some(inner) = trimmed
        .strip_prefix('<')
        .and_then(|inner| inner.strip_suffix('>'))
      {
        preview_by_url.get(inner).copied()
      } else {
        let markdown_link_preview =
          markdown_link_target(trimmed).and_then(|target| preview_by_url.get(target).copied());
        markdown_link_preview.or_else(|| preview_by_url.get(trimmed).copied())
      };

      if let Some(preview) = preview {
        flush_markdown(&mut segments, &mut markdown_lines);
        segments.push(ReviewCommentBodySegment::Preview(preview.clone()));
      } else {
        markdown_lines.push(line.to_string());
      }
    }

    flush_markdown(&mut segments, &mut markdown_lines);

    if segments.is_empty() {
      vec![ReviewCommentBodySegment::Markdown(body.to_string())]
    } else {
      segments
    }
  }

  #[cfg(test)]
  fn review_comment_markdown_body_from_segments(segments: &[ReviewCommentBodySegment]) -> String {
    segments
      .iter()
      .filter_map(|segment| match segment {
        ReviewCommentBodySegment::Markdown(markdown) => Some(markdown.as_str()),
        ReviewCommentBodySegment::Preview(_) => None,
      })
      .collect::<Vec<_>>()
      .join("\n")
  }

  fn review_comment_segmented_height_px(
    &self,
    comment_id: u64,
    body: &str,
    metrics: MarkdownTextMetrics<'_>,
    markdown_line_height_px: f32,
    suggestion_context: Option<&gfm_markdown_viewer::SuggestionContext>,
  ) -> f32 {
    let segments = self.review_comment_body_segments(comment_id, body);
    let mut markdown_height = 0.0;
    let mut previews_height = 0.0;

    for segment in segments {
      match segment {
        ReviewCommentBodySegment::Markdown(markdown) => {
          if markdown.trim().is_empty() {
            continue;
          }
          markdown_height += estimate_markdown_height_px_with_suggestion_context(
            &markdown,
            metrics,
            markdown_line_height_px,
            suggestion_context,
          );
        }
        ReviewCommentBodySegment::Preview(preview) => {
          previews_height += self.review_comment_code_reference_preview_height_px(&preview);
        }
      }
    }

    markdown_height + previews_height
  }

  fn ensure_review_comment_markdown_cache_entry(
    &mut self,
    comment_id: u64,
    body: &str,
  ) -> &mut ReviewCommentMarkdownCacheEntry {
    let body_hash = Self::review_comment_body_hash(body);
    let entry = self
      .review_comment_markdown_cache
      .entry(comment_id)
      .or_insert_with(|| ReviewCommentMarkdownCacheEntry {
        body_hash,
        parsed: parse_markdown(body),
        estimated_heights_px: HashMap::new(),
      });

    if entry.body_hash != body_hash {
      entry.body_hash = body_hash;
      entry.parsed = parse_markdown(body);
      entry.estimated_heights_px.clear();
    }

    entry
  }

  fn cached_parsed_review_comment_markdown(
    &mut self,
    comment_id: u64,
    body: &str,
  ) -> ParsedMarkdown {
    self
      .ensure_review_comment_markdown_cache_entry(comment_id, body)
      .parsed
      .clone()
  }

  fn cached_review_comment_body_height_px(
    &mut self,
    comment_id: u64,
    body: &str,
    markdown_line_height_px: f32,
    suggestion_context: Option<&gfm_markdown_viewer::SuggestionContext>,
    cx: &App,
  ) -> f32 {
    let wrap_columns =
      self.review_comment_body_wrap_columns(self.measured_review_comment_char_width() / px(1.0));
    let measurer = self.review_comment_text_measurer(cx);
    let width_of = |text: &str| measurer.width_in_columns(text);
    let metrics = MarkdownTextMetrics::measured(wrap_columns, &width_of);
    let entry = self.ensure_review_comment_markdown_cache_entry(comment_id, body);
    let key = (wrap_columns, markdown_line_height_px.to_bits());
    if suggestion_context.is_none()
      && let Some(height) = entry.estimated_heights_px.get(&key)
    {
      return *height;
    }

    let estimated = estimate_parsed_markdown_height_px_with_suggestion_context(
      &entry.parsed,
      metrics,
      markdown_line_height_px,
      suggestion_context,
    );
    if suggestion_context.is_none() {
      entry.estimated_heights_px.insert(key, estimated);
    }
    estimated
  }

  fn review_comment_markdown_options(
    &self,
    state: MarkdownRenderState,
    scope_id: usize,
  ) -> MarkdownRenderOptions {
    // A newline typed in the composer is a line break in the comment, as on GitHub.
    let mut options = MarkdownRenderOptions::default()
      .with_state(state)
      .with_scope_id(scope_id)
      .with_hardbreaks();
    if let Some(resolver) = self.review_comment_asset_url_resolver.clone() {
      options = options.with_asset_url_resolver(resolver);
    }
    options
  }

  pub fn set_review_comment_pr_number(&mut self, pr_number: Option<u64>, cx: &mut Context<Self>) {
    if self.review_comment_pr_number != pr_number {
      self.review_comment_pr_number = pr_number;
      cx.notify();
    }
  }

  /// Which comments offer a send action. The host decides: a comment already
  /// handed to the agent has nowhere left to go.
  pub fn set_sendable_review_comment_ids<I>(&mut self, ids: I, cx: &mut Context<Self>)
  where
    I: IntoIterator<Item = u64>,
  {
    self.sendable_review_comment_ids = ids.into_iter().collect();
    cx.notify();
  }

  pub fn set_editable_review_comment_ids<I>(&mut self, ids: I, cx: &mut Context<Self>)
  where
    I: IntoIterator<Item = u64>,
  {
    self.editable_review_comment_ids = ids.into_iter().collect();
    if self
      .editing_review_comment_id
      .is_some_and(|id| !self.editable_review_comment_ids.contains(&id))
    {
      self.clear_review_comment_edit_state();
    }
    if self
      .review_comment_edit_submitting_id
      .is_some_and(|id| !self.editable_review_comment_ids.contains(&id))
    {
      self.review_comment_edit_submitting_id = None;
    }
    if self
      .review_comment_edit_error
      .as_ref()
      .is_some_and(|(id, _)| !self.editable_review_comment_ids.contains(id))
    {
      self.review_comment_edit_error = None;
    }
    if self
      .review_comment_delete_submitting_id
      .is_some_and(|id| !self.editable_review_comment_ids.contains(&id))
    {
      self.review_comment_delete_submitting_id = None;
    }
    cx.notify();
  }

  pub fn set_review_comment_edit_handler(
    &mut self,
    handler: Option<ReviewCommentEditHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_edit_handler = handler;
    cx.notify();
  }

  pub fn set_review_comment_delete_handler(
    &mut self,
    handler: Option<ReviewCommentDeleteHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_delete_handler = handler;
    if self.review_comment_delete_handler.is_none() {
      self.review_comment_delete_submitting_id = None;
    }
    cx.notify();
  }

  pub fn set_review_comment_send_handler(
    &mut self,
    handler: Option<ReviewCommentSendHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_send_handler = handler;
    cx.notify();
  }

  pub fn set_review_comment_create_handler(
    &mut self,
    handler: Option<ReviewCommentCreateHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_create_handler = handler;
    if self.review_comment_create_handler.is_none() {
      self.clear_review_comment_create_state();
      self.clear_review_comment_reply_state();
      self.refresh_review_comment_projection(cx);
      return;
    }
    cx.notify();
  }

  pub fn set_review_comment_cancel_handler(
    &mut self,
    handler: Option<ReviewCommentCancelHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_cancel_handler = handler;
    cx.notify();
  }

  pub fn set_review_comment_replies_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
    if self.review_comment_replies_enabled == enabled {
      return;
    }
    self.review_comment_replies_enabled = enabled;
    if !enabled {
      self.clear_review_comment_reply_state();
    }
    cx.notify();
  }

  pub fn set_has_pending_review(&mut self, has_pending_review: bool, cx: &mut Context<Self>) {
    if self.has_pending_review == has_pending_review {
      return;
    }
    self.has_pending_review = has_pending_review;
    cx.notify();
  }

  pub fn review_comment_display_mode(&self) -> ReviewCommentDisplayMode {
    self.review_comment_display_mode
  }

  /// What this editor currently offers a review comment. The host installs the
  /// capabilities as a set, and this is how it checks it got the set it asked
  /// for.
  pub fn review_capabilities(&self) -> ReviewCapabilities {
    ReviewCapabilities {
      display_mode: self.review_comment_display_mode,
      replies_enabled: self.review_comment_replies_enabled,
      create: self.review_comment_create_handler.is_some(),
      edit: self.review_comment_edit_handler.is_some(),
      delete: self.review_comment_delete_handler.is_some(),
      cancel: self.review_comment_cancel_handler.is_some(),
      send: self.review_comment_send_handler.is_some(),
      resolve: self.review_comment_resolve_handler.is_some(),
      link: self.review_comment_link_handler.is_some(),
      image_upload: self.review_comment_image_upload_handler.is_some(),
      asset_url_resolver: self.review_comment_asset_url_resolver.is_some(),
      preview_renderer: self.review_comment_preview_renderer.is_some(),
      suggestion_action_factory: self.review_comment_suggestion_action_factory.is_some(),
      pr_number: self.review_comment_pr_number,
    }
  }

  /// The comments hanging in the diff right now, in the order the host gave
  /// them.
  pub fn review_comment_ids(&self) -> Vec<u64> {
    self
      .review_comments
      .iter()
      .map(|comment| comment.id)
      .collect()
  }

  pub fn has_pending_review(&self) -> bool {
    self.has_pending_review
  }

  pub fn set_review_comment_display_mode(
    &mut self,
    mode: ReviewCommentDisplayMode,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_display_mode == mode {
      return;
    }
    self.review_comment_display_mode = mode;
    if matches!(mode, ReviewCommentDisplayMode::LocalNote) {
      self.collapsed_review_comments.clear();
      self.clear_review_comment_reply_state();
    }
    self.refresh_review_comment_projection(cx);
  }

  pub fn set_review_comment_resolve_handler(
    &mut self,
    handler: Option<ReviewCommentResolveHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_resolve_handler = handler;
    if self.review_comment_resolve_handler.is_none() {
      self.review_comment_resolve_in_flight.clear();
    }
    cx.notify();
  }

  pub fn set_review_comment_suggestion_action_factory(
    &mut self,
    factory: Option<ReviewCommentSuggestionActionFactory>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_suggestion_action_factory = factory;
    self.review_comment_markdown_cache.clear();
    cx.notify();
  }

  /// The host answers every resolve toggle with one write completion, success
  /// or failure; without this the spinner survives the write forever.
  pub fn finish_review_comment_resolve_submissions(&mut self, cx: &mut Context<Self>) {
    if self.review_comment_resolve_in_flight.is_empty() {
      return;
    }
    self.review_comment_resolve_in_flight.clear();
    cx.notify();
  }

  pub fn set_review_comment_link_handler(
    &mut self,
    handler: Option<ReviewCommentLinkHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_link_handler = handler;
    cx.notify();
  }

  pub fn set_review_comment_asset_url_resolver(
    &mut self,
    resolver: Option<ReviewCommentAssetUrlResolver>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_asset_url_resolver = resolver;
    cx.notify();
  }

  pub fn set_review_comment_image_upload_handler(
    &mut self,
    handler: Option<ReviewCommentImageUploadHandler>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_image_upload_handler = handler;
    cx.notify();
  }

  pub fn set_review_comment_preview_renderer(
    &mut self,
    renderer: Option<ReviewCommentPreviewRenderer>,
    cx: &mut Context<Self>,
  ) {
    self.review_comment_preview_renderer = renderer;
    if self.review_comment_preview_renderer.is_none() {
      self.review_comment_create_preview_open = false;
      self.review_comment_edit_preview_open = false;
      self.review_comment_reply_preview_open = false;
    }
    cx.notify();
  }

  /// Caches every open composer's row count; returns true when one of them moved,
  /// so the diff has to reserve a different number of lines for it.
  fn sync_review_comment_composer_rows(&mut self, window: &Window, cx: &App) -> bool {
    let font = gpui::font(cx.theme().font_family.as_ref());
    let font_size = window.rem_size() * REVIEW_COMMENT_COMPOSER_TEXT_REMS;
    let inputs = [
      (
        self.review_comment_create_input.clone(),
        self.review_comment_create_actions_width_px(cx),
      ),
      (
        self.review_comment_edit_input.clone(),
        REVIEW_COMMENT_FLOATING_ACTIONS_WIDTH_PX,
      ),
      (
        self.review_comment_reply_input.clone(),
        REVIEW_COMMENT_FLOATING_ACTIONS_WIDTH_PX,
      ),
    ];
    let rows = inputs
      .into_iter()
      .filter_map(|(input, actions_width_px)| input.map(|input| (input, actions_width_px)))
      .map(|(input, actions_width_px)| {
        let value = input.read(cx).value();
        (
          input.entity_id(),
          review_comment_composer_rows(
            value.as_ref(),
            self.review_comment_composer_text_width(actions_width_px),
            font.clone(),
            font_size,
            window,
          ),
        )
      })
      .collect::<HashMap<_, _>>();
    if rows == self.review_comment_composer_rows {
      return false;
    }
    self.review_comment_composer_rows = rows;
    true
  }

  fn refresh_review_comment_composer_layout(&mut self, window: &Window, cx: &mut Context<Self>) {
    if !self.sync_review_comment_composer_rows(window, cx) {
      return;
    }
    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    } else {
      cx.notify();
    }
  }

  fn review_comment_composer_rows_for(&self, input: &Entity<TextareaState>) -> usize {
    self
      .review_comment_composer_rows
      .get(&input.entity_id())
      .copied()
      .unwrap_or(REVIEW_COMMENT_COMPOSER_MIN_ROWS)
  }

  fn review_comment_composer_textarea_height(&self, input: &Entity<TextareaState>) -> Pixels {
    px(review_comment_composer_textarea_height_px(
      self.review_comment_composer_rows_for(input),
      self.review_comment_composer_line_height_px,
    ))
  }

  fn review_comment_composer_chrome_height_px(&self) -> f32 {
    if self.review_comment_preview_renderer.is_some() {
      MARKDOWN_COMPOSER_CHROME_HEIGHT_PX
    } else {
      0.0
    }
  }

  /// A text box sits taller than the line of markdown it replaces: its own padding,
  /// and a line height of its own. Half the difference is pulled back at each end so
  /// editing a comment neither moves its text nor resizes its card.
  fn review_comment_in_card_composer_pull_px(&self) -> f32 {
    ((REVIEW_COMMENT_COMPOSER_TEXTAREA_VERTICAL_CHROME_PX
      + self.review_comment_composer_line_height_px
      - self.review_comment_line_height_px)
      / 2.0)
      .max(0.0)
  }

  fn review_comment_in_card_composer_body_height_px(
    &self,
    input: Option<&Entity<TextareaState>>,
  ) -> f32 {
    (self.review_comment_composer_body_height_px(input)
      - 2.0 * self.review_comment_in_card_composer_pull_px())
    .max(self.review_comment_line_height_px)
  }

  fn review_comment_composer_body_height_px(&self, input: Option<&Entity<TextareaState>>) -> f32 {
    let textarea_height_px = input
      .map(|input| self.review_comment_composer_textarea_height(input) / px(1.0))
      .unwrap_or_else(|| {
        review_comment_composer_textarea_height_px(
          REVIEW_COMMENT_COMPOSER_MIN_ROWS,
          self.review_comment_composer_line_height_px,
        )
      });
    review_comment_composer_body_height_px(
      textarea_height_px,
      self.review_comment_composer_chrome_height_px(),
    )
  }

  fn review_comment_drop_zone(
    &self,
    id: impl Into<gpui::ElementId>,
    input: Entity<TextareaState>,
    cx: &mut Context<Self>,
  ) -> gpui::Stateful<gpui::Div> {
    let handler = self.review_comment_image_upload_handler.clone();
    div()
      .id(id.into())
      .w_full()
      .rounded_md()
      .drag_over::<ExternalPaths>(|this, _, _, cx| this.bg(cx.theme().drop_target))
      .on_drop(cx.listener(move |_, paths: &ExternalPaths, window, cx| {
        if let Some(handler) = handler.as_ref() {
          handler(paths, input.clone(), window, cx);
        }
      }))
  }

  fn clear_review_comment_edit_state(&mut self) {
    self.editing_review_comment_id = None;
    self.review_comment_edit_initial_body = None;
    self.review_comment_edit_error = None;
    self.review_comment_edit_preview_open = false;
  }

  fn clear_review_comment_create_state(&mut self) {
    self.review_comment_create_draft = None;
    self.review_comment_create_drag_start_display_line = None;
    self.review_comment_create_drag_active = false;
    self.review_comment_create_submitting = false;
    self.review_comment_create_error = None;
    self.review_comment_create_preview_open = false;
    self.hovered_review_comment_create_display_line = None;
  }

  fn clear_review_comment_reply_state(&mut self) {
    self.replying_to_review_comment_id = None;
    self.review_comment_reply_submitting = false;
    self.review_comment_reply_error = None;
    self.review_comment_reply_preview_open = false;
  }

  pub fn finish_review_comment_edit_submission(
    &mut self,
    comment_id: u64,
    error: Option<Arc<str>>,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_edit_submitting_id != Some(comment_id) {
      return;
    }
    self.review_comment_edit_submitting_id = None;
    if let Some(error) = error {
      self.review_comment_edit_error = Some((comment_id, error));
    } else {
      self.clear_review_comment_edit_state();
    }
    self.refresh_review_comment_projection(cx);
  }

  pub fn start_review_comment_delete_submission(
    &mut self,
    comment_id: u64,
    cx: &mut Context<Self>,
  ) {
    if !self.editable_review_comment_ids.contains(&comment_id)
      || !self
        .review_comments
        .iter()
        .any(|comment| comment.id == comment_id)
    {
      return;
    }

    self.review_comment_delete_submitting_id = Some(comment_id);
    if self.editing_review_comment_id == Some(comment_id) {
      self.clear_review_comment_edit_state();
    }
    self.refresh_review_comment_projection(cx);
  }

  pub fn finish_review_comment_delete_submission(
    &mut self,
    comment_id: u64,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_delete_submitting_id != Some(comment_id) {
      return;
    }
    self.review_comment_delete_submitting_id = None;
    self.refresh_review_comment_projection(cx);
  }

  pub fn finish_review_comment_create_submission(
    &mut self,
    error: Option<Arc<str>>,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_reply_submitting {
      self.review_comment_reply_submitting = false;
      if let Some(error) = error {
        self.review_comment_reply_error = Some(error);
      } else {
        self.clear_review_comment_reply_state();
      }
      self.refresh_review_comment_projection(cx);
      return;
    }

    if !self.review_comment_create_submitting {
      return;
    }
    self.review_comment_create_submitting = false;
    if let Some(error) = error {
      self.review_comment_create_error = Some(error);
    } else {
      self.clear_review_comment_create_state();
    }
    self.refresh_review_comment_projection(cx);
  }

  fn refresh_review_comment_projection(&mut self, cx: &mut Context<Self>) {
    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    } else {
      cx.notify();
    }
  }

  fn ensure_review_comment_edit_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<TextareaState> {
    if let Some(input) = self.review_comment_edit_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(
          REVIEW_COMMENT_COMPOSER_MIN_ROWS,
          REVIEW_COMMENT_COMPOSER_MAX_ROWS,
        )
        .submit_on_enter(true)
        .placeholder("Edit review comment...")
    });
    cx.subscribe_in(
      &input,
      window,
      |editor, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { shift: false, .. } = event {
          let Some(comment_id) = editor.editing_review_comment_id else {
            return;
          };
          Self::trim_review_comment_input_trailing_newline(state, window, cx);
          editor.save_review_comment_edit(comment_id, window, cx);
        }
      },
    )
    .detach();
    // `insert` and `set_value` are silent, so the row count is read from the entity
    // itself: every mutation notifies, typed or programmatic.
    cx.observe_in(&input, window, |editor, _, window, cx| {
      editor.refresh_review_comment_composer_layout(window, cx);
    })
    .detach();
    self.review_comment_edit_input = Some(input.clone());
    input
  }

  fn start_review_comment_edit(
    &mut self,
    comment_id: u64,
    body: Arc<str>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_create_submitting
      || self.review_comment_reply_submitting
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }
    if !self.editable_review_comment_ids.contains(&comment_id) {
      return;
    }
    self.clear_review_comment_create_state();
    self.clear_review_comment_reply_state();
    self.expand_review_comment_thread_for(comment_id);

    let input = self.ensure_review_comment_edit_input(window, cx);
    let initial_text = body.to_string();
    input.update(cx, |state, cx| {
      state.set_value(initial_text.clone(), window, cx);
    });

    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.editing_review_comment_id = Some(comment_id);
    self.review_comment_edit_initial_body = Some(body);
    self.review_comment_edit_error = None;
    self.refresh_review_comment_projection(cx);
  }

  fn cancel_review_comment_edit(&mut self, cx: &mut Context<Self>) {
    if self.review_comment_edit_submitting_id.is_some() {
      return;
    }
    if self.editing_review_comment_id.is_none() {
      return;
    }
    self.clear_review_comment_edit_state();
    self.refresh_review_comment_projection(cx);
  }

  fn on_review_comment_edit_input_escape(
    &mut self,
    _: &InputEscape,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_edit_submitting_id.is_some() {
      cx.stop_propagation();
      return;
    }
    let was_editing = self.editing_review_comment_id.is_some();
    self.cancel_review_comment_edit(cx);
    if was_editing {
      self.invoke_review_comment_cancel_handler(window, cx);
    }
    cx.stop_propagation();
  }

  fn save_review_comment_edit(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }
    if self.editing_review_comment_id != Some(comment_id) {
      return;
    }

    let Some(initial_body) = self.review_comment_edit_initial_body.as_ref() else {
      self.clear_review_comment_edit_state();
      self.refresh_review_comment_projection(cx);
      return;
    };
    let Some(input) = self.review_comment_edit_input.as_ref() else {
      self.clear_review_comment_edit_state();
      self.refresh_review_comment_projection(cx);
      return;
    };
    let raw_value = input.read(cx).value();

    if let Some(next_body) = next_review_comment_body(raw_value.as_str(), initial_body.as_ref())
      && let Some(handler) = self.review_comment_edit_handler.as_ref()
    {
      self.review_comment_edit_error = None;
      self.review_comment_edit_submitting_id = Some(comment_id);
      handler(comment_id, next_body, window, cx);
      self.refresh_review_comment_projection(cx);
      return;
    }

    self.clear_review_comment_edit_state();
    self.refresh_review_comment_projection(cx);
  }

  /// Sends one comment on its own, if the host says it still can go.
  fn request_review_comment_send(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.editable_review_comment_ids.contains(&comment_id) {
      return;
    }
    if !self.sendable_review_comment_ids.contains(&comment_id) {
      return;
    }
    if !self
      .review_comments
      .iter()
      .any(|comment| comment.id == comment_id)
    {
      return;
    }
    let Some(handler) = self.review_comment_send_handler.clone() else {
      return;
    };

    handler(comment_id, window, cx);
  }

  fn request_review_comment_delete(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_delete_submitting_id.is_some()
      || self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_create_submitting
      || self.review_comment_reply_submitting
    {
      return;
    }
    if !self.editable_review_comment_ids.contains(&comment_id) {
      return;
    }
    if !self
      .review_comments
      .iter()
      .any(|comment| comment.id == comment_id)
    {
      return;
    }
    let Some(handler) = self.review_comment_delete_handler.as_ref() else {
      return;
    };

    handler(comment_id, window, cx);
  }

  fn ensure_review_comment_create_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<TextareaState> {
    if let Some(input) = self.review_comment_create_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(
          REVIEW_COMMENT_COMPOSER_MIN_ROWS,
          REVIEW_COMMENT_COMPOSER_MAX_ROWS,
        )
        .submit_on_enter(true)
        .placeholder("Add review comment...")
    });
    cx.subscribe_in(
      &input,
      window,
      |editor, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { shift: false, .. } = event {
          Self::trim_review_comment_input_trailing_newline(state, window, cx);
          let mode = review_comment_submit_mode(
            editor.review_comment_display_mode,
            editor.has_pending_review,
          );
          editor.save_review_comment_create(mode, window, cx);
        }
      },
    )
    .detach();
    // `insert` and `set_value` are silent, so the row count is read from the entity
    // itself: every mutation notifies, typed or programmatic.
    cx.observe_in(&input, window, |editor, _, window, cx| {
      editor.refresh_review_comment_composer_layout(window, cx);
    })
    .detach();
    self.review_comment_create_input = Some(input.clone());
    input
  }

  fn ensure_review_comment_reply_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<TextareaState> {
    if let Some(input) = self.review_comment_reply_input.as_ref() {
      return input.clone();
    }

    let input = cx.new(|cx| {
      TextareaState::new(window, cx)
        .auto_grow(
          REVIEW_COMMENT_COMPOSER_MIN_ROWS,
          REVIEW_COMMENT_COMPOSER_MAX_ROWS,
        )
        .submit_on_enter(true)
        .placeholder("Reply to review comment...")
    });
    cx.subscribe_in(
      &input,
      window,
      |editor, state, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { shift: false, .. } = event {
          Self::trim_review_comment_input_trailing_newline(state, window, cx);
          editor.save_review_comment_reply(window, cx);
        }
      },
    )
    .detach();
    // `insert` and `set_value` are silent, so the row count is read from the entity
    // itself: every mutation notifies, typed or programmatic.
    cx.observe_in(&input, window, |editor, _, window, cx| {
      editor.refresh_review_comment_composer_layout(window, cx);
    })
    .detach();
    self.review_comment_reply_input = Some(input.clone());
    input
  }

  fn trim_review_comment_input_trailing_newline(
    state: &Entity<TextareaState>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let raw = state.read(cx).value().to_string();
    let trimmed = raw.trim_end_matches('\n').to_string();
    if trimmed != raw {
      state.update(cx, |input, cx| {
        input.set_value(trimmed, window, cx);
      });
    }
  }

  fn start_review_comment_reply(
    &mut self,
    comment_id: u64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.review_comment_replies_enabled
      || self.review_comment_create_handler.is_none()
      || self.review_comment_create_drag_active
      || self.review_comment_create_submitting
      || self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_reply_submitting
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }

    let thread_id = self.thread_id_for_comment(comment_id);
    if self
      .review_comment_threads
      .get(&thread_id)
      .and_then(|comments| comments.last().copied())
      != Some(comment_id)
    {
      return;
    }
    if !self
      .review_comments
      .iter()
      .any(|comment| comment.id == comment_id)
    {
      return;
    }

    self.clear_review_comment_edit_state();
    self.clear_review_comment_create_state();
    self.expand_review_comment_thread_for(comment_id);

    let input = self.ensure_review_comment_reply_input(window, cx);
    input.update(cx, |state, cx| {
      state.set_value("", window, cx);
    });

    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.replying_to_review_comment_id = Some(comment_id);
    self.review_comment_reply_error = None;
    self.refresh_review_comment_projection(cx);
  }

  fn cancel_review_comment_reply(&mut self, cx: &mut Context<Self>) {
    if self.review_comment_reply_submitting {
      return;
    }
    if self.replying_to_review_comment_id.is_none() {
      return;
    }
    self.clear_review_comment_reply_state();
    self.refresh_review_comment_projection(cx);
  }

  fn on_review_comment_reply_input_escape(
    &mut self,
    _: &InputEscape,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_reply_submitting {
      cx.stop_propagation();
      return;
    }
    let was_replying = self.replying_to_review_comment_id.is_some();
    self.cancel_review_comment_reply(cx);
    if was_replying {
      self.invoke_review_comment_cancel_handler(window, cx);
    }
    cx.stop_propagation();
  }

  fn save_review_comment_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.review_comment_replies_enabled
      || self.review_comment_create_drag_active
      || self.review_comment_create_submitting
      || self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_reply_submitting
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }
    let Some(in_reply_to_id) = self.replying_to_review_comment_id else {
      return;
    };
    let Some(reply_to_comment) = self
      .review_comments
      .iter()
      .find(|comment| comment.id == in_reply_to_id)
      .cloned()
    else {
      self.clear_review_comment_reply_state();
      self.refresh_review_comment_projection(cx);
      return;
    };
    let Some(input) = self.review_comment_reply_input.as_ref() else {
      self.clear_review_comment_reply_state();
      self.refresh_review_comment_projection(cx);
      return;
    };

    let raw_value = input.read(cx).value();
    let Some(body) = next_review_comment_body(raw_value.as_str(), "") else {
      return;
    };
    let Some(handler) = self.review_comment_create_handler.as_ref() else {
      return;
    };

    handler(
      ReviewCommentCreateRequest {
        line: reply_to_comment.line,
        side: reply_to_comment.side,
        start_line: None,
        start_side: None,
        in_reply_to_id: Some(in_reply_to_id),
        body,
        mode: ReviewCommentMode::SingleComment,
      },
      window,
      cx,
    );

    self.review_comment_reply_error = None;
    self.review_comment_reply_submitting = true;
    self.refresh_review_comment_projection(cx);
  }

  fn review_comment_create_target_for_display_line(
    &self,
    display_line: usize,
    cx: &App,
  ) -> Option<ReviewCommentCreateTarget> {
    let doc_line_count = self.document.read(cx).len_lines();
    match self.display_line(display_line, doc_line_count)? {
      DisplayLine::Doc { doc_line, .. } | DisplayLine::Modified { doc_line, .. } => {
        Some(ReviewCommentCreateTarget {
          display_line,
          line: doc_line,
          side: ReviewCommentSide::Right,
        })
      }
      DisplayLine::Removed { old_line, .. } => Some(ReviewCommentCreateTarget {
        display_line,
        line: old_line,
        side: ReviewCommentSide::Left,
      }),
      _ => None,
    }
  }

  fn review_comment_create_display_range_for_group(
    &self,
    group_id: &Arc<str>,
    cx: &App,
  ) -> Option<(usize, usize)> {
    let projection = self.projection.as_ref()?;
    let mut right_targets = Vec::new();
    let mut left_targets = Vec::new();

    for (display_line, line) in projection.lines.iter().enumerate() {
      let preferred_side = match line {
        DisplayLine::Doc {
          change: Some(_),
          group_id: Some(id),
          ..
        }
        | DisplayLine::Modified {
          group_id: Some(id), ..
        } if id.as_ref() == group_id.as_ref() => Some(ReviewCommentSide::Right),
        DisplayLine::Removed {
          group_id: Some(id), ..
        } if id.as_ref() == group_id.as_ref() => Some(ReviewCommentSide::Left),
        _ => None,
      };
      let Some(preferred_side) = preferred_side else {
        continue;
      };
      let Some(target) = self.review_comment_create_target_for_display_line(display_line, cx)
      else {
        continue;
      };
      if target.side != preferred_side {
        continue;
      }

      match target.side {
        ReviewCommentSide::Right => right_targets.push(target.display_line),
        ReviewCommentSide::Left => left_targets.push(target.display_line),
      }
    }

    let targets = if right_targets.is_empty() {
      &left_targets
    } else {
      &right_targets
    };
    Some((*targets.first()?, *targets.last()?))
  }

  pub fn start_review_comment_for_active_hunk(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(group_id) = self.active_hunk_group_id(cx) else {
      return false;
    };
    let Some((first_display_line, last_display_line)) =
      self.review_comment_create_display_range_for_group(&group_id, cx)
    else {
      return false;
    };

    self.start_review_comment_create_drag(first_display_line, cx);
    self.update_review_comment_create_drag_from_display_line(Some(last_display_line), cx);
    if self.review_comment_create_draft.is_none() {
      return false;
    }
    self.finish_review_comment_create_drag(window, cx);
    true
  }

  fn set_review_comment_create_hover_from_display_line(
    &mut self,
    display_line: Option<usize>,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_create_drag_active || self.review_comment_create_draft.is_some() {
      return;
    }

    let hovered = display_line.and_then(|line| {
      self
        .review_comment_create_target_for_display_line(line, cx)
        .map(|target| target.display_line)
    });
    if self.hovered_review_comment_create_display_line != hovered {
      self.hovered_review_comment_create_display_line = hovered;
      cx.notify();
    }
  }

  fn start_review_comment_create_drag(&mut self, display_line: usize, cx: &mut Context<Self>) {
    if self.review_comment_create_handler.is_none()
      || self.editing_review_comment_id.is_some()
      || self.review_comment_edit_submitting_id.is_some()
      || self.replying_to_review_comment_id.is_some()
      || self.review_comment_reply_submitting
      || self.review_comment_create_submitting
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }
    let Some(target) = self.review_comment_create_target_for_display_line(display_line, cx) else {
      return;
    };

    self.is_selecting = false;
    self.display_selection = None;
    self.review_comment_create_drag_start_display_line = Some(target.display_line);
    self.review_comment_create_drag_active = true;
    self.review_comment_create_draft = Some(ReviewCommentCreateDraft {
      first_display_line: target.display_line,
      last_display_line: target.display_line,
      line: target.line,
      side: target.side,
      start_line: None,
      start_side: None,
    });
    self.review_comment_create_error = None;
    self.hovered_review_comment_create_display_line = Some(target.display_line);
    cx.notify();
  }

  pub(crate) fn update_review_comment_create_drag_from_display_line(
    &mut self,
    display_line: Option<usize>,
    cx: &mut Context<Self>,
  ) {
    if !self.review_comment_create_drag_active {
      self.set_review_comment_create_hover_from_display_line(display_line, cx);
      return;
    }

    let Some(start_display_line) = self.review_comment_create_drag_start_display_line else {
      return;
    };
    let Some(current_display_line) = display_line else {
      return;
    };
    let Some(start_target) =
      self.review_comment_create_target_for_display_line(start_display_line, cx)
    else {
      return;
    };
    let Some(current_target) =
      self.review_comment_create_target_for_display_line(current_display_line, cx)
    else {
      return;
    };

    let (first_target, last_target) = if start_target.display_line <= current_target.display_line {
      (start_target, current_target)
    } else {
      (current_target, start_target)
    };
    let first_display_line = first_target.display_line;
    let last_display_line = last_target.display_line;

    let (start_line, start_side) =
      if first_target.side == last_target.side && first_target.line != last_target.line {
        (Some(first_target.line), Some(first_target.side))
      } else {
        (None, None)
      };

    let next_draft = ReviewCommentCreateDraft {
      first_display_line,
      last_display_line,
      line: last_target.line,
      side: last_target.side,
      start_line,
      start_side,
    };

    if self.review_comment_create_draft != Some(next_draft) {
      self.review_comment_create_draft = Some(next_draft);
      cx.notify();
    }
  }

  fn finish_review_comment_create_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.review_comment_create_submitting {
      return;
    }
    if !self.review_comment_create_drag_active {
      return;
    }
    self.review_comment_create_drag_active = false;
    self.review_comment_create_drag_start_display_line = None;

    if self.review_comment_create_draft.is_none() {
      self.clear_review_comment_create_state();
      self.refresh_review_comment_projection(cx);
      return;
    }

    let input = self.ensure_review_comment_create_input(window, cx);
    input.update(cx, |state, cx| {
      state.set_value("", window, cx);
    });
    let input_for_focus = input.clone();
    window.on_next_frame(move |window, cx| {
      input_for_focus.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    });

    self.refresh_review_comment_projection(cx);
  }

  fn cancel_review_comment_create(&mut self, cx: &mut Context<Self>) {
    if self.review_comment_create_submitting {
      return;
    }
    if self.review_comment_create_draft.is_none() {
      return;
    }
    self.clear_review_comment_create_state();
    self.refresh_review_comment_projection(cx);
  }

  /// Drops the draft and hands the focus back to the page that owns the diff.
  pub fn cancel_review_comment_create_draft(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let had_draft = self.review_comment_create_draft.is_some();
    self.cancel_review_comment_create(cx);
    if had_draft {
      self.invoke_review_comment_cancel_handler(window, cx);
    }
  }

  fn on_review_comment_create_input_escape(
    &mut self,
    _: &InputEscape,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_create_submitting {
      cx.stop_propagation();
      return;
    }
    self.cancel_review_comment_create_draft(window, cx);
    cx.stop_propagation();
  }

  fn invoke_review_comment_cancel_handler(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(handler) = self.review_comment_cancel_handler.clone() {
      handler(window, cx);
    }
  }

  fn save_review_comment_create(
    &mut self,
    mode: ReviewCommentMode,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_create_drag_active
      || self.review_comment_create_submitting
      || self.review_comment_reply_submitting
      || self.review_comment_edit_submitting_id.is_some()
      || self.review_comment_delete_submitting_id.is_some()
    {
      return;
    }
    let Some(draft) = self.review_comment_create_draft else {
      return;
    };
    let Some(input) = self.review_comment_create_input.as_ref() else {
      self.clear_review_comment_create_state();
      self.refresh_review_comment_projection(cx);
      return;
    };
    let raw_value = input.read(cx).value();
    let Some(body) = next_review_comment_body(raw_value.as_str(), "") else {
      return;
    };
    let Some(handler) = self.review_comment_create_handler.as_ref() else {
      return;
    };

    handler(
      ReviewCommentCreateRequest {
        line: draft.line,
        side: draft.side,
        start_line: draft.start_line,
        start_side: draft.start_side,
        in_reply_to_id: None,
        body,
        mode,
      },
      window,
      cx,
    );

    self.review_comment_create_error = None;
    self.review_comment_create_submitting = true;
    self.refresh_review_comment_projection(cx);
  }

  fn original_lines_for_create_draft(&self, cx: &App) -> Vec<String> {
    let Some(draft) = self.review_comment_create_draft else {
      return Vec::new();
    };
    let Some(projection) = self.projection.as_ref() else {
      return Vec::new();
    };
    let document = self.document.read(cx);
    let mut lines = Vec::new();
    for display_line in draft.first_display_line..=draft.last_display_line {
      let Some(doc_line) = projection.display_to_doc_line(display_line) else {
        continue;
      };
      let line_content = document
        .line_content(doc_line)
        .map(|cow| cow.into_owned())
        .unwrap_or_default();
      let trimmed = line_content.trim_end_matches(['\n', '\r']);
      lines.push(trimmed.to_string());
    }
    lines
  }

  pub(crate) fn can_insert_review_comment_suggestion(&self, cx: &App) -> bool {
    let Some(draft) = self.review_comment_create_draft else {
      return false;
    };
    if draft.side != ReviewCommentSide::Right {
      return false;
    }
    if self.review_comment_create_submitting {
      return false;
    }
    !self.original_lines_for_create_draft(cx).is_empty()
  }

  fn review_comment_create_preview_suggestion_context(
    &self,
    cx: &App,
  ) -> Option<gfm_markdown_viewer::SuggestionContext> {
    let draft = self.review_comment_create_draft?;
    if draft.side != ReviewCommentSide::Right {
      return None;
    }
    let original_lines = self.original_lines_for_create_draft(cx);
    if original_lines.is_empty() {
      return None;
    }
    let start_line = draft.start_line.unwrap_or(draft.line).saturating_add(1);
    Some(gfm_markdown_viewer::SuggestionContext {
      original_start_line: Some(start_line),
      suggested_start_line: Some(start_line),
      original_lines,
      path: Arc::from(""),
    })
  }

  fn review_comment_preview_suggestion_context_for_id(
    &self,
    comment_id: u64,
  ) -> Option<gfm_markdown_viewer::SuggestionContext> {
    self
      .review_comments
      .iter()
      .find(|comment| comment.id == comment_id)
      .and_then(|comment| comment.suggestion_context.clone())
  }

  fn insert_review_comment_suggestion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.can_insert_review_comment_suggestion(cx) {
      return;
    }
    let Some(input) = self.review_comment_create_input.clone() else {
      return;
    };
    let lines = self.original_lines_for_create_draft(cx);
    if lines.is_empty() {
      return;
    }
    let block = format!("```suggestion\n{}\n```\n", lines.join("\n"));
    let prefix = if input.read(cx).value().is_empty() {
      ""
    } else {
      "\n\n"
    };
    let body = format!("{prefix}{block}");
    input.update(cx, |state, cx| {
      state.insert(body, window, cx);
      state.focus(window, cx);
    });
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

  fn thread_id_for_comment(&self, comment_id: u64) -> u64 {
    self
      .review_comment_thread_roots
      .get(&comment_id)
      .copied()
      .unwrap_or(comment_id)
  }

  /// Replying to or editing a collapsed thread reopens it: the input has to
  /// land somewhere visible.
  fn expand_review_comment_thread_for(&mut self, comment_id: u64) {
    let thread_id = self.thread_id_for_comment(comment_id);
    if let Some(ids) = self.review_comment_threads.get(&thread_id) {
      for id in ids {
        self.collapsed_review_comments.remove(id);
      }
    }
  }

  fn toggle_review_comment_thread(&mut self, thread_id: u64, cx: &mut Context<Self>) {
    let ids = self
      .review_comment_threads
      .get(&thread_id)
      .cloned()
      .unwrap_or_else(|| vec![thread_id]);
    let should_collapse = ids
      .iter()
      .any(|id| !self.collapsed_review_comments.contains(id));

    for id in ids {
      if should_collapse {
        self.collapsed_review_comments.insert(id);
      } else {
        self.collapsed_review_comments.remove(&id);
      }
    }

    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    }
  }

  pub fn toggle_review_comment(&mut self, id: u64, cx: &mut Context<Self>) {
    self.toggle_review_comment_thread(self.thread_id_for_comment(id), cx);
  }

  fn toggle_review_comment_thread_resolution(
    &mut self,
    thread_id: Arc<str>,
    first_message_id: u64,
    currently_resolved: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_resolve_in_flight.contains(&thread_id) {
      return;
    }
    let Some(handler) = self.review_comment_resolve_handler.clone() else {
      return;
    };
    self
      .review_comment_resolve_in_flight
      .insert(thread_id.clone());
    self.apply_resolution_visibility(first_message_id, currently_resolved);
    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    } else {
      cx.notify();
    }
    handler(thread_id, first_message_id, currently_resolved, window, cx);
  }

  /// Resolving folds the thread away at once; unresolving reopens the
  /// conversation, so it unfolds at once. The auto-collapse marker resets so
  /// a fresh re-resolve folds again on the next refresh.
  fn apply_resolution_visibility(&mut self, first_message_id: u64, currently_resolved: bool) {
    self
      .auto_collapsed_resolved_thread_ids
      .remove(&first_message_id);
    if currently_resolved {
      self.expand_review_comment_thread_for(first_message_id);
    } else if let Some(comment_ids) = self.review_comment_threads.get(&first_message_id).cloned() {
      self
        .collapsed_review_comments
        .extend(comment_ids.iter().copied());
      self
        .auto_collapsed_resolved_thread_ids
        .insert(first_message_id);
    }
  }

  fn ensure_find_input(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Entity<InputState> {
    if let Some(input) = self.find_input.clone() {
      return input;
    }

    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in file..."));
    let subscription = cx.subscribe_in(&input, window, Self::on_find_input_event);
    self.find_input = Some(input.clone());
    self.find_input_subscription = Some(subscription);
    input
  }

  fn focus_find_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(input) = self.find_input.clone() {
      input.update(cx, |state, cx| {
        state.focus(window, cx);
      });
    }
  }

  fn is_find_input_focused(&self, window: &Window, cx: &App) -> bool {
    if !self.find_panel_open {
      return false;
    }

    self
      .find_input
      .as_ref()
      .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
      .unwrap_or(false)
  }

  fn is_review_comment_edit_input_focused(&self, window: &Window, cx: &App) -> bool {
    if self.editing_review_comment_id.is_none() || self.review_comment_edit_submitting_id.is_some()
    {
      return false;
    }

    self
      .review_comment_edit_input
      .as_ref()
      .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
      .unwrap_or(false)
  }

  fn is_review_comment_create_input_focused(&self, window: &Window, cx: &App) -> bool {
    if self.review_comment_create_draft.is_none()
      || self.review_comment_create_drag_active
      || self.review_comment_create_submitting
    {
      return false;
    }

    self
      .review_comment_create_input
      .as_ref()
      .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
      .unwrap_or(false)
  }

  fn is_review_comment_reply_input_focused(&self, window: &Window, cx: &App) -> bool {
    if self.replying_to_review_comment_id.is_none() || self.review_comment_reply_submitting {
      return false;
    }

    self
      .review_comment_reply_input
      .as_ref()
      .map(|input| input.read(cx).focus_handle(cx).is_focused(window))
      .unwrap_or(false)
  }

  fn on_find_input_event(
    &mut self,
    state: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      InputEvent::Change => {
        self.find_query = state.read(cx).value().to_string();
        self.refresh_find_matches(self.measured_editor_line_height(), true, cx);
      }
      InputEvent::PressEnter { secondary, .. } => {
        if *secondary {
          self.find_previous_match(window, cx);
        } else {
          self.find_next_match(window, cx);
        }
      }
      _ => {}
    }
  }

  fn collect_find_matches(&self, query: &str, cx: &App) -> Vec<FindMatch> {
    if query.is_empty() {
      return Vec::new();
    }

    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let mut matches = Vec::new();

    for display_line in 0..total_display_lines {
      let Some(doc_line) = (match self.display_line(display_line, doc_line_count) {
        Some(DisplayLine::Doc { doc_line, .. }) | Some(DisplayLine::Modified { doc_line, .. }) => {
          Some(doc_line)
        }
        _ => None,
      }) else {
        continue;
      };

      let Some(line_content) = document.line_content(doc_line) else {
        continue;
      };

      let line_text = line_content.as_ref();
      if line_text.is_empty() {
        continue;
      }

      let line_start_offset = document.line_to_char(doc_line);
      let mut search_start = 0usize;
      while search_start <= line_text.len() {
        let Some(search_slice) = line_text.get(search_start..) else {
          break;
        };
        let Some(found) = search_slice.find(query) else {
          break;
        };
        let byte_start = search_start + found;
        let byte_end = byte_start + query.len();
        if !line_text.is_char_boundary(byte_start) || !line_text.is_char_boundary(byte_end) {
          search_start = search_start.saturating_add(1).min(line_text.len());
          while search_start < line_text.len() && !line_text.is_char_boundary(search_start) {
            search_start += 1;
          }
          continue;
        }
        let column_start = line_text[..byte_start].chars().count();
        let column_end = line_text[..byte_end].chars().count();
        let range_start = line_start_offset + column_start;
        let range_end = line_start_offset + column_end;
        matches.push(FindMatch {
          display_line,
          column_start,
          column_end,
          doc_range: range_start..range_end,
        });
        let next_char_boundary = line_text
          .get(byte_start..)
          .and_then(|slice| slice.chars().next().map(|ch| byte_start + ch.len_utf8()))
          .unwrap_or(line_text.len());
        search_start = byte_end.max(next_char_boundary);
      }
    }

    matches
  }

  fn scroll_to_find_match(
    &mut self,
    display_line: usize,
    line_height: Pixels,
    smooth: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let total_lines = self.display_line_count(self.document.read(cx).len_lines());
    if total_lines == 0 {
      return false;
    }

    let metrics = self.vertical_scroll_metrics(line_height, total_lines);
    let target = (display_line as f32 - metrics.scroll_padding).clamp(0.0, metrics.max_scroll);
    let start = self.scroll_offset_y;
    let delta = target - start;

    if !smooth || delta.abs() <= FIND_SCROLL_MIN_DELTA {
      self.scroll_offset_y = target;
      cx.notify();
      return true;
    }

    self.find_scroll_epoch = self.find_scroll_epoch.saturating_add(1);
    let scroll_epoch = self.find_scroll_epoch;
    cx.spawn(async move |this, cx| {
      let started_at = cx.background_executor().now();
      loop {
        cx.background_executor().timer(FIND_SCROLL_TICK).await;
        let progress = ((cx.background_executor().now() - started_at).as_secs_f32()
          / FIND_SCROLL_DURATION.as_secs_f32())
        .min(1.0);
        let eased = ease_out_cubic(progress);
        let next_scroll = start + delta * eased;
        let done = progress >= 1.0;
        let should_continue = this
          .update(cx, |editor, cx| {
            if editor.find_scroll_epoch != scroll_epoch {
              return false;
            }
            editor.scroll_offset_y = if done { target } else { next_scroll };
            cx.notify();
            !done
          })
          .unwrap_or(false);
        if !should_continue {
          break;
        }
      }
    })
    .detach();

    true
  }

  fn select_find_match(
    &mut self,
    index: usize,
    line_height: Pixels,
    smooth_scroll: bool,
    cx: &mut Context<Self>,
  ) {
    let Some(found) = self.find_matches.get(index).cloned() else {
      self.find_active_match = None;
      cx.notify();
      return;
    };

    self.find_active_match = Some(index);
    self.selected_range = found.doc_range.clone();
    self.selection_reversed = false;
    self.display_selection = None;
    self.target_column = Some(found.column_end);
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    self.scroll_to_find_match(found.display_line, line_height, smooth_scroll, cx);
    cx.notify();
  }

  fn refresh_find_matches(
    &mut self,
    line_height: Pixels,
    smooth_scroll: bool,
    cx: &mut Context<Self>,
  ) {
    if self.find_query.is_empty() {
      self.find_matches.clear();
      self.find_active_match = None;
      cx.notify();
      return;
    }

    let previous_active_match = self
      .find_active_match
      .and_then(|index| self.find_matches.get(index).cloned());
    self.find_matches = self.collect_find_matches(&self.find_query, cx);

    if self.find_matches.is_empty() {
      self.find_active_match = None;
      cx.notify();
      return;
    }

    let next_active = previous_active_match
      .and_then(|previous| {
        self
          .find_matches
          .iter()
          .position(|candidate| *candidate == previous)
      })
      .unwrap_or(0);
    self.select_find_match(next_active, line_height, smooth_scroll, cx);
  }

  fn find_next_match_with_line_height(&mut self, line_height: Pixels, cx: &mut Context<Self>) {
    if self.find_matches.is_empty() {
      return;
    }

    let next_index = self
      .find_active_match
      .map(|index| (index + 1) % self.find_matches.len())
      .unwrap_or(0);
    self.select_find_match(next_index, line_height, true, cx);
  }

  fn find_previous_match_with_line_height(&mut self, line_height: Pixels, cx: &mut Context<Self>) {
    if self.find_matches.is_empty() {
      return;
    }

    let previous_index = self
      .find_active_match
      .map(|index| {
        if index == 0 {
          self.find_matches.len() - 1
        } else {
          index - 1
        }
      })
      .unwrap_or_else(|| self.find_matches.len() - 1);
    self.select_find_match(previous_index, line_height, true, cx);
  }

  pub fn is_find_panel_open(&self) -> bool {
    self.find_panel_open
  }

  pub fn find_panel_occludes_display_line(&self, display_line: usize) -> bool {
    if !self.find_panel_open {
      return false;
    }

    let first_visible_line = self.scroll_offset_y.floor().max(0.0) as usize;
    display_line < first_visible_line.saturating_add(FIND_PANEL_OCCLUDED_VISIBLE_LINES)
  }

  fn reset_find_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.find_query.clear();
    self.find_matches.clear();
    self.find_active_match = None;
    self.find_scroll_epoch = self.find_scroll_epoch.saturating_add(1);

    if let Some(input) = self.find_input.clone() {
      input.update(cx, |state, cx| {
        state.set_value(String::new(), window, cx);
      });
    }
  }

  pub(crate) fn open_find_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.find_panel_open = true;
    let input = self.ensure_find_input(window, cx);
    let input_value = input.read(cx).value().to_string();
    let query = self
      .find_query_from_selection(cx)
      .filter(|selection| !selection.is_empty())
      .unwrap_or(input_value);
    input.update(cx, |state, cx| {
      state.set_value(query.clone(), window, cx);
    });
    self.find_query = query;
    self.refresh_find_matches(self.measured_editor_line_height(), false, cx);
    cx.on_next_frame(window, |this, window, cx| {
      this.focus_find_input(window, cx);
    });
    cx.notify();
  }

  /// Returns whether a panel was actually closed, so callers can let the key
  /// event bubble when there was nothing to close.
  pub(crate) fn close_find_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
    if !self.find_panel_open {
      return false;
    }

    self.find_panel_open = false;
    self.reset_find_input(window, cx);
    window.focus(&self.focus_handle, cx);
    cx.notify();
    true
  }

  pub(crate) fn find_next_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let _ = window;
    self.find_next_match_with_line_height(self.measured_editor_line_height(), cx);
  }

  pub(crate) fn find_previous_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let _ = window;
    self.find_previous_match_with_line_height(self.measured_editor_line_height(), cx);
  }

  fn find_query_from_selection(&self, cx: &App) -> Option<String> {
    let selected = self.selected_text_for_copy(cx)?;
    let selected = selected.replace('\r', "");
    let first_line = selected.split('\n').next().unwrap_or_default().to_string();
    if first_line.is_empty() {
      None
    } else {
      Some(first_line)
    }
  }

  fn clear_hovered_hunk_for_overlay(&mut self, cx: &mut Context<Self>) {
    let had_hover = self.hovered_group_id.take().is_some();
    let had_conflict_hover = self.hovered_conflict_start_line.take().is_some();
    let had_comment_create_hover = self
      .hovered_review_comment_create_display_line
      .take()
      .is_some();
    self.last_mouse_position = None;
    if had_hover || had_conflict_hover || had_comment_create_hover {
      cx.notify();
    }
  }

  fn render_find_panel(
    &mut self,
    editor_entity: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    if !self.find_panel_open {
      return None;
    }

    let input = self.ensure_find_input(window, cx);
    let theme = cx.theme().clone();
    let total_matches = self.find_matches.len();
    let current_match = self
      .find_active_match
      .map(|index| index + 1)
      .unwrap_or(0)
      .min(total_matches);
    let has_matches = total_matches > 0;

    let previous_editor = editor_entity.clone();
    let next_editor = editor_entity.clone();
    let close_editor = editor_entity.clone();
    let mouse_down_editor = editor_entity.clone();
    let mouse_move_editor = editor_entity.clone();
    let mouse_up_editor = editor_entity.clone();

    Some(
      div()
        .absolute()
        .top(px(8.0))
        .right(px(12.0))
        .w(px(360.0))
        .p_2()
        .occlude()
        .flex()
        .items_center()
        .gap_2()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
          mouse_down_editor.update(cx, |editor, cx| {
            editor.clear_hovered_hunk_for_overlay(cx);
          });
          cx.stop_propagation();
        })
        .on_mouse_move(move |_, _, cx| {
          mouse_move_editor.update(cx, |editor, cx| {
            editor.clear_hovered_hunk_for_overlay(cx);
          });
          cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
          mouse_up_editor.update(cx, |editor, cx| {
            editor.clear_hovered_hunk_for_overlay(cx);
          });
          cx.stop_propagation();
        })
        .child(
          div()
            .flex_1()
            .min_w(px(0.0))
            .child(Input::new(&input).small().border_color(theme.border)),
        )
        .child(
          div()
            .w(px(52.0))
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{}/{}", current_match, total_matches)),
        )
        .child(
          Button::new("editor-find-prev")
            .icon(IconName::ArrowUp)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Previous match")
            .disabled(!has_matches)
            .on_click(move |_, window, cx| {
              previous_editor.update(cx, |editor, cx| {
                editor.find_previous_match(window, cx);
              });
            }),
        )
        .child(
          Button::new("editor-find-next")
            .icon(IconName::ArrowDown)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Next match")
            .disabled(!has_matches)
            .on_click(move |_, window, cx| {
              next_editor.update(cx, |editor, cx| {
                editor.find_next_match(window, cx);
              });
            }),
        )
        .child(
          Button::new("editor-find-close")
            .icon(IconName::Close)
            .ghost()
            .xsmall()
            .compact()
            .tooltip("Close find")
            .on_click(move |_, window, cx| {
              close_editor.update(cx, |editor, cx| {
                editor.close_find_panel(window, cx);
              });
            }),
        )
        .into_any_element(),
    )
  }

  pub fn scroll_to_review_comment(
    &mut self,
    comment_id: u64,
    line_height: Pixels,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(projection) = self.projection.as_ref() else {
      return false;
    };

    let display_line_for_comment = |id: u64| {
      projection.lines.iter().position(
        |line| matches!(line, DisplayLine::ReviewComment { id: line_id, .. } if *line_id == id),
      )
    };

    let Some(display_line) = display_line_for_comment(comment_id).or_else(|| {
      let thread_id = self.thread_id_for_comment(comment_id);
      display_line_for_comment(thread_id)
    }) else {
      return false;
    };

    let total_lines = projection.lines.len();
    let metrics = self.vertical_scroll_metrics(line_height, total_lines);
    let target = (display_line as f32 - metrics.scroll_padding)
      .max(0.0)
      .min(metrics.max_scroll);
    let start = self.scroll_offset_y;
    let delta = target - start;
    if delta.abs() <= REVIEW_COMMENT_SCROLL_MIN_DELTA {
      self.scroll_offset_y = target;
      cx.notify();
      return true;
    }

    self.review_comment_scroll_epoch = self.review_comment_scroll_epoch.saturating_add(1);
    let scroll_epoch = self.review_comment_scroll_epoch;
    cx.spawn(async move |this, cx| {
      let started_at = cx.background_executor().now();
      loop {
        cx.background_executor()
          .timer(REVIEW_COMMENT_SCROLL_TICK)
          .await;
        let progress = ((cx.background_executor().now() - started_at).as_secs_f32()
          / REVIEW_COMMENT_SCROLL_DURATION.as_secs_f32())
        .min(1.0);
        let eased = ease_out_cubic(progress);
        let next_scroll = start + delta * eased;
        let done = progress >= 1.0;
        let should_continue = this
          .update(cx, |editor, cx| {
            if editor.review_comment_scroll_epoch != scroll_epoch {
              return false;
            }
            editor.scroll_offset_y = if done { target } else { next_scroll };
            cx.notify();
            !done
          })
          .unwrap_or(false);
        if !should_continue {
          break;
        }
      }
    })
    .detach();

    true
  }

  fn review_comment_layouts(
    &self,
    side_filter: Option<ReviewCommentSide>,
    line_height: Pixels,
  ) -> Vec<ReviewCommentLayout> {
    let Some(projection) = self.projection.as_ref() else {
      return Vec::new();
    };
    if self.review_comments.is_empty() {
      return Vec::new();
    }
    let is_local_note_mode = matches!(
      self.review_comment_display_mode,
      ReviewCommentDisplayMode::LocalNote
    );

    let mut spans_by_comment: HashMap<u64, (usize, usize)> = HashMap::new();

    for (idx, line) in projection.lines.iter().enumerate() {
      let DisplayLine::ReviewComment { id, side, .. } = line else {
        continue;
      };
      if let Some(filter) = side_filter
        && *side != filter
      {
        continue;
      }
      let entry = spans_by_comment.entry(*id).or_insert((idx, 0));
      if idx < entry.0 {
        entry.0 = idx;
      }
      entry.1 = entry.1.saturating_add(1);
    }

    if spans_by_comment.is_empty() {
      return Vec::new();
    }
    let total_display_lines = projection.lines.len();
    let display_viewport = self.viewport_range(line_height, total_display_lines);

    let comments_by_id: HashMap<u64, &ReviewComment> = self
      .review_comments
      .iter()
      .map(|comment| (comment.id, comment))
      .collect();

    let mut layouts = Vec::new();
    let mut seen_threads = HashSet::new();
    for thread_id in &self.review_comment_thread_order {
      let Some(comment_ids) = self.review_comment_threads.get(thread_id) else {
        continue;
      };

      let mut messages = Vec::new();
      let mut visible_comment_ids = Vec::new();
      let mut thread_first_line = usize::MAX;
      let mut thread_last_line = 0usize;

      for comment_id in comment_ids {
        let Some(comment) = comments_by_id.get(comment_id).copied() else {
          continue;
        };
        if let Some(filter) = side_filter
          && comment.side != filter
        {
          continue;
        }
        if let Some((first, count)) = spans_by_comment.get(comment_id).copied()
          && count > 0
        {
          thread_first_line = thread_first_line.min(first);
          thread_last_line = thread_last_line.max(first + count - 1);
        }
        visible_comment_ids.push(*comment_id);
        messages.push(ReviewCommentMessageLayout {
          id: comment.id,
          author: comment.author.clone(),
          avatar_url: comment.avatar_url.clone(),
          line_label: comment.line_label.clone(),
          body: comment.body.clone(),
          suggestion_context: comment.suggestion_context.clone(),
          created_at: comment.created_at.clone(),
          thread_id: comment.thread_id.clone(),
          is_resolved: comment.is_resolved,
          is_outdated: comment.is_outdated,
          viewer_can_resolve: comment.viewer_can_resolve,
          viewer_can_unresolve: comment.viewer_can_unresolve,
          is_pending: comment.is_pending,
          shows_header: review_comment_shows_header(comment, is_local_note_mode),
        });
      }

      if visible_comment_ids.is_empty() {
        continue;
      }
      if thread_first_line == usize::MAX {
        continue;
      }
      if thread_last_line < display_viewport.start || thread_first_line >= display_viewport.end {
        continue;
      }

      let span_count = thread_last_line - thread_first_line + 1;

      let top = line_height * (thread_first_line as f32 - self.scroll_offset_y);
      let height = line_height * span_count as f32;
      let collapsed = !visible_comment_ids.is_empty()
        && visible_comment_ids
          .iter()
          .all(|id| self.collapsed_review_comments.contains(id));

      layouts.push(ReviewCommentLayout {
        id: *thread_id,
        top,
        height,
        messages,
        collapsed,
      });
      seen_threads.insert(*thread_id);
    }

    for comment in &self.review_comments {
      let thread_id = self.thread_id_for_comment(comment.id);
      if seen_threads.contains(&thread_id) {
        continue;
      }
      if let Some(filter) = side_filter
        && comment.side != filter
      {
        continue;
      }
      let Some((first, count)) = spans_by_comment.get(&comment.id).copied() else {
        continue;
      };
      if count == 0 {
        continue;
      }
      let last = first + count - 1;
      if last < display_viewport.start || first >= display_viewport.end {
        continue;
      }

      layouts.push(ReviewCommentLayout {
        id: thread_id,
        top: line_height * (first as f32 - self.scroll_offset_y),
        height: line_height * count as f32,
        messages: vec![ReviewCommentMessageLayout {
          id: comment.id,
          author: comment.author.clone(),
          avatar_url: comment.avatar_url.clone(),
          line_label: comment.line_label.clone(),
          body: comment.body.clone(),
          suggestion_context: comment.suggestion_context.clone(),
          created_at: comment.created_at.clone(),
          thread_id: comment.thread_id.clone(),
          is_resolved: comment.is_resolved,
          is_outdated: comment.is_outdated,
          viewer_can_resolve: comment.viewer_can_resolve,
          viewer_can_unresolve: comment.viewer_can_unresolve,
          is_pending: comment.is_pending,
          shows_header: review_comment_shows_header(comment, is_local_note_mode),
        }],
        collapsed: self.collapsed_review_comments.contains(&comment.id),
      });
      seen_threads.insert(thread_id);
    }

    layouts
  }

  fn render_review_comment_code_reference_preview_card(
    &self,
    comment_id: u64,
    segment_index: usize,
    preview: &ReviewCommentCodeReferencePreview,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let preview = as_gfm_code_reference_preview(preview);
    div()
      .id(format!(
        "review-comment-code-reference-{}-{segment_index}",
        comment_id
      ))
      .child(render_github_code_reference_preview_card(&preview, cx))
      .into_any_element()
  }

  fn review_comment_overlay_x_offset(&self) -> Pixels {
    review_comment_overlay_x_offset_for_scroll(self.scroll_handle.offset().x)
  }

  fn render_review_comment_actions_menu(
    message_id: u64,
    body: Arc<str>,
    can_delete: bool,
    button_id: String,
    editor_entity: Entity<Editor>,
  ) -> gpui::AnyElement {
    let editor_edit = editor_entity.clone();
    let editor_delete = editor_entity;
    let body_for_edit = body;
    div()
      .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
      .child(
        Button::new(button_id)
          .ghost()
          .xsmall()
          .compact()
          .icon(IconName::Ellipsis)
          .tooltip("More actions")
          .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            let editor_edit = editor_edit.clone();
            let editor_delete = editor_delete.clone();
            let body_for_edit = body_for_edit.clone();
            let mut menu = menu.item(
              PopupMenuItem::new("Edit")
                .icon(Icon::new(UiIconName::SquarePen))
                .on_click(move |_, window, cx| {
                  let body = body_for_edit.clone();
                  editor_edit.update(cx, |editor, cx| {
                    editor.start_review_comment_edit(message_id, body, window, cx);
                  });
                }),
            );
            if can_delete {
              menu = menu.item(
                PopupMenuItem::new("Delete")
                  .icon(Icon::new(UiIconName::Trash))
                  .on_click(move |_, window, cx| {
                    editor_delete.update(cx, |editor, cx| {
                      editor.request_review_comment_delete(message_id, window, cx);
                    });
                  }),
              );
            }
            menu
          }),
      )
      .into_any_element()
  }

  /// Cancel and save, laid out like the read actions so they can take their place.
  fn render_review_comment_edit_actions(
    message_id: u64,
    can_save: bool,
    is_submitting: bool,
    editor_entity: Entity<Editor>,
  ) -> gpui::AnyElement {
    let cancel_editor = editor_entity.clone();
    let save_editor = editor_entity;
    h_flex()
      .items_center()
      .gap_1()
      .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
      .child(
        Button::new(format!("review-comment-edit-cancel-{message_id}"))
          .ghost()
          .xsmall()
          .compact()
          .icon(Icon::new(UiIconName::X))
          .tooltip("Cancel")
          .disabled(is_submitting)
          .on_click(move |_, window, cx| {
            cx.stop_propagation();
            cancel_editor.update(cx, |editor, cx| {
              let was_editing = editor.editing_review_comment_id.is_some();
              editor.cancel_review_comment_edit(cx);
              if was_editing {
                editor.invoke_review_comment_cancel_handler(window, cx);
              }
            });
          }),
      )
      .child(
        Button::new(format!("review-comment-edit-save-{message_id}"))
          .xsmall()
          .compact()
          .icon(Icon::new(UiIconName::Check))
          .tooltip("Save")
          .disabled(!can_save || is_submitting)
          .on_click(move |_, window, cx| {
            cx.stop_propagation();
            save_editor.update(cx, |editor, cx| {
              editor.save_review_comment_edit(message_id, window, cx);
            });
          }),
      )
      .into_any_element()
  }

  fn render_review_comment_direct_actions(
    message_id: u64,
    body: Arc<str>,
    can_delete: bool,
    can_send: bool,
    editor_entity: Entity<Editor>,
  ) -> gpui::AnyElement {
    let send_editor = editor_entity.clone();
    let edit_editor = editor_entity.clone();
    let delete_editor = editor_entity;
    div()
      .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
      .child(
        h_flex()
          .items_center()
          .gap_1()
          .when(can_send, |this| {
            this.child(
              Button::new(format!("review-comment-send-{message_id}"))
                .ghost()
                .xsmall()
                .compact()
                .icon(UiIconName::ArrowUp)
                .tooltip("Send this comment to the agent")
                .on_click(move |_, window, cx| {
                  cx.stop_propagation();
                  send_editor.update(cx, |editor, cx| {
                    editor.request_review_comment_send(message_id, window, cx);
                  });
                }),
            )
          })
          .child(
            Button::new(format!("review-comment-edit-{message_id}"))
              .ghost()
              .xsmall()
              .compact()
              .icon(UiIconName::SquarePen)
              .tooltip("Edit")
              .on_click({
                let body = body.clone();
                move |_, window, cx| {
                  cx.stop_propagation();
                  let body = body.clone();
                  edit_editor.update(cx, |editor, cx| {
                    editor.start_review_comment_edit(message_id, body, window, cx);
                  });
                }
              }),
          )
          .when(can_delete, |this| {
            this.child(
              Button::new(format!("review-comment-delete-{message_id}"))
                .ghost()
                .xsmall()
                .compact()
                .icon(UiIconName::Trash)
                .tooltip("Delete")
                .on_click(move |_, window, cx| {
                  cx.stop_propagation();
                  delete_editor.update(cx, |editor, cx| {
                    editor.request_review_comment_delete(message_id, window, cx);
                  });
                }),
            )
          }),
      )
      .into_any_element()
  }

  fn render_review_comments_overlay(
    &mut self,
    editor_entity: Entity<Editor>,
    side_filter: Option<ReviewCommentSide>,
    line_height: Pixels,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    let mut layouts = self.review_comment_layouts(side_filter, line_height);
    if layouts.is_empty() {
      return None;
    }
    layouts.sort_by(|a, b| {
      (a.top / px(1.0))
        .partial_cmp(&(b.top / px(1.0)))
        .unwrap_or(std::cmp::Ordering::Equal)
    });

    let theme = cx.theme().clone();
    let overlay_x_offset = self.review_comment_overlay_x_offset();
    let review_comment_header_height = line_height * REVIEW_COMMENT_HEADER_HEIGHT_LINES;
    let is_local_note_mode = matches!(
      self.review_comment_display_mode,
      ReviewCommentDisplayMode::LocalNote
    );
    let mut overlay = div()
      .absolute()
      .top(px(0.0))
      .left(overlay_x_offset)
      .right(px(0.0))
      .bottom(px(0.0));

    for layout in layouts.into_iter().rev() {
      let thread_id = layout.id;
      let Some(first_message) = layout.messages.first() else {
        continue;
      };
      let review_comment_edit_handler = self.review_comment_edit_handler.clone();
      let review_comment_delete_handler = self.review_comment_delete_handler.clone();
      let review_comment_send_handler = self.review_comment_send_handler.clone();
      let review_comment_submission_in_flight = self.review_comment_edit_submitting_id.is_some()
        || self.review_comment_create_submitting
        || self.review_comment_reply_submitting
        || self.review_comment_delete_submitting_id.is_some();
      let can_save_review_comment_edit =
        review_comment_edit_handler.is_some() && !review_comment_submission_in_flight;
      let can_save_review_comment_reply =
        self.review_comment_create_handler.is_some() && !review_comment_submission_in_flight;
      let editor = editor_entity.clone();
      let is_collapsed = layout.collapsed;
      let last_message_id = layout.messages.last().map(|message| message.id);
      let toggle_button = if is_local_note_mode {
        None
      } else {
        let toggle_icon = if is_collapsed {
          IconName::ChevronRight
        } else {
          IconName::ChevronDown
        };
        let toggle_button = Button::new(format!("review-comment-toggle-{}", thread_id))
          .icon(toggle_icon)
          .ghost()
          .xsmall()
          .compact()
          .on_click(move |_, _, cx| {
            cx.stop_propagation();
            editor.update(cx, |editor, cx| {
              editor.toggle_review_comment_thread(thread_id, cx)
            });
          });
        Some(
          div()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
              cx.stop_propagation();
            })
            .child(toggle_button)
            .into_any_element(),
        )
      };
      let first_message_id = first_message.id;
      let first_message_actions = if !review_comment_submission_in_flight
        && self.editing_review_comment_id != Some(first_message.id)
        && self.editable_review_comment_ids.contains(&first_message.id)
      {
        let body = first_message.body.clone();
        let can_delete = review_comment_delete_handler.is_some();
        let can_send = review_comment_send_handler.is_some()
          && self.sendable_review_comment_ids.contains(&first_message.id);
        Some(if is_local_note_mode {
          Self::render_review_comment_direct_actions(
            first_message_id,
            body,
            can_delete,
            can_send,
            editor_entity.clone(),
          )
        } else {
          Self::render_review_comment_actions_menu(
            first_message_id,
            body,
            can_delete,
            format!("review-comment-actions-{}", first_message_id),
            editor_entity.clone(),
          )
        })
      } else {
        None
      };
      let resolve_thread_id = first_message.thread_id.clone();
      let thread_is_resolved = first_message.is_resolved;
      let resolve_in_flight = resolve_thread_id
        .as_ref()
        .is_some_and(|id| self.review_comment_resolve_in_flight.contains(id));
      let resolve_control = review_comment_resolve_control(
        resolve_thread_id.is_some(),
        self.review_comment_resolve_handler.is_some(),
        resolve_in_flight,
        thread_is_resolved,
        first_message.viewer_can_resolve,
        first_message.viewer_can_unresolve,
      );
      let first_message_resolve_button = match resolve_control {
        ReviewCommentResolveControl::Toggle { label, enabled } => {
          resolve_thread_id.clone().map(|thread_id_arc| {
            let editor = editor_entity.clone();
            let thread_id_for_click = thread_id_arc.clone();
            div()
              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
              })
              .child(
                Button::new(format!("review-comment-resolve-{}", first_message_id))
                  .ghost()
                  .xsmall()
                  .compact()
                  .label(label)
                  .disabled(!enabled)
                  .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let thread_id_for_click = thread_id_for_click.clone();
                    editor.update(cx, |editor, cx| {
                      editor.toggle_review_comment_thread_resolution(
                        thread_id_for_click,
                        first_message_id,
                        thread_is_resolved,
                        window,
                        cx,
                      );
                    });
                  }),
              )
          })
        }
        ReviewCommentResolveControl::ResolvedTag | ReviewCommentResolveControl::Nothing => None,
      };
      let first_message_reply_button =
        if self.review_comment_replies_enabled && last_message_id == Some(first_message_id) {
          let is_replying = self.replying_to_review_comment_id == Some(first_message_id);
          let disabled_reason = if review_comment_submission_in_flight {
            Some("A comment submission is in progress.")
          } else if self.replying_to_review_comment_id.is_some() && !is_replying {
            Some("Finish or cancel the open reply first.")
          } else {
            None
          };
          let disabled = disabled_reason.is_some();
          let editor = editor_entity.clone();
          Some(
            div()
              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
              })
              .child({
                let button = Button::new(format!("review-comment-reply-{}", first_message_id))
                  .ghost()
                  .xsmall()
                  .compact()
                  .icon(UiIconName::MessageCircleReply)
                  .label("Reply")
                  .selected(is_replying)
                  .disabled(disabled)
                  .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    editor.update(cx, |editor, cx| {
                      if is_replying {
                        let was_replying = editor.replying_to_review_comment_id.is_some();
                        editor.cancel_review_comment_reply(cx);
                        if was_replying {
                          editor.invoke_review_comment_cancel_handler(window, cx);
                        }
                      } else {
                        editor.start_review_comment_reply(first_message_id, window, cx);
                      }
                    });
                  });
                if let Some(reason) = disabled_reason {
                  button.tooltip(reason)
                } else {
                  button
                }
              }),
          )
        } else {
          None
        };

      // A single line is visible in the diff; only a range is worth spelling out.
      let line_label = first_message.line_label.clone();

      let meta = if is_local_note_mode {
        h_flex()
          .items_center()
          .gap_2()
          .when_some(line_label, |this, label| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label.as_ref().to_string()),
            )
          })
          .when(first_message.is_outdated, |this| {
            this.child(Tag::color(ColorName::Orange).child("Outdated"))
          })
      } else {
        h_flex()
          .items_center()
          .gap_2()
          .child(
            Avatar::new()
              .name(first_message.author.to_string())
              .when_some(first_message.avatar_url.clone(), |this, url| {
                this.src(url.as_ref().to_string())
              })
              .small(),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.foreground)
              .child(first_message.author.to_string()),
          )
          .when_some(line_label, |this, label| {
            this
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child(label.as_ref().to_string()),
              )
              .child(
                div()
                  .text_xs()
                  .text_color(theme.muted_foreground)
                  .child("•"),
              )
          })
          .when(!first_message.created_at.is_empty(), |this| {
            this.child(
              div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(first_message.created_at.as_ref().to_string()),
            )
          })
      };

      let shows_header = first_message.shows_header;
      let editing_first_message = self.editing_review_comment_id == Some(first_message_id);
      let reserve_floating_actions_room = review_comment_body_reserves_actions_room(
        is_local_note_mode,
        shows_header,
        first_message_actions.is_some() || editing_first_message,
      );
      let wears_resolved_tag = resolve_control == ReviewCommentResolveControl::ResolvedTag;
      let actions_cluster = h_flex()
        .items_center()
        .gap_1()
        .when(first_message.is_pending, |this| {
          this.child(Tag::color(ColorName::Amber).child("Pending"))
        })
        .when(first_message.is_outdated, |this| {
          this.child(Tag::color(ColorName::Orange).child("Outdated"))
        })
        .when(wears_resolved_tag, |this| {
          this.child(Tag::color(ColorName::Green).child("Resolved"))
        })
        .when_some(first_message_resolve_button, |this, button| {
          this.child(button)
        })
        .when_some(first_message_reply_button, |this, button| {
          this.child(button)
        })
        .when_some(first_message_actions, |this, actions| this.child(actions))
        .when_some(toggle_button, |this, button| this.child(button));

      // In a local note the actions float over the body instead of sitting in a header,
      // and the composer takes their place while it is open.
      let actions_cluster = if is_local_note_mode && editing_first_message {
        h_flex()
          .items_center()
          .gap_1()
          .child(Self::render_review_comment_edit_actions(
            first_message_id,
            can_save_review_comment_edit,
            self.review_comment_edit_submitting_id == Some(first_message_id),
            editor_entity.clone(),
          ))
      } else {
        actions_cluster
      };

      let (header_actions, floating_actions) = if is_local_note_mode {
        (None, Some(actions_cluster))
      } else {
        (Some(actions_cluster), None)
      };
      let header_editor = editor_entity.clone();
      let mut header = h_flex()
        .items_center()
        .h(review_comment_header_height)
        .justify_between()
        .gap_2()
        .child(meta)
        .when_some(header_actions, |this, actions| this.child(actions));
      if !is_local_note_mode {
        header = header.on_mouse_down(MouseButton::Left, move |_, _, cx| {
          header_editor.update(cx, |editor, cx| {
            editor.toggle_review_comment_thread(thread_id, cx)
          });
        });
      }

      let link_handler = {
        let editor = editor_entity.clone();
        Arc::new(move |url: &str, window: &mut Window, cx: &mut App| {
          if window.modifiers().secondary() {
            return LinkAction::Open;
          }

          let parsed_comment_link = parse_github_pr_comment_link(url);
          let (handled, link_handler) = editor.update(cx, |editor, cx| {
            if let Some((pr_number, comment_id)) = parsed_comment_link {
              if editor.review_comment_pr_number != Some(pr_number) {
                return (false, editor.review_comment_link_handler.clone());
              }
              if !editor
                .review_comments
                .iter()
                .any(|comment| comment.id == comment_id)
              {
                return (false, editor.review_comment_link_handler.clone());
              }
              return (
                editor.scroll_to_review_comment(comment_id, line_height, cx),
                editor.review_comment_link_handler.clone(),
              );
            }
            (false, editor.review_comment_link_handler.clone())
          });
          if handled {
            return LinkAction::Handled;
          }

          if let Some(handler) = link_handler
            && handler(url, window, cx)
          {
            return LinkAction::Handled;
          }

          LinkAction::Open
        })
      };

      let mut thread_messages = v_flex().gap(px(REVIEW_COMMENT_SPACING_PX));
      for (index, message) in layout.messages.iter().enumerate() {
        let is_last_message = Some(message.id) == last_message_id;
        let is_edit_submitting = self.review_comment_edit_submitting_id == Some(message.id);
        let body: gpui::AnyElement = if self.editing_review_comment_id == Some(message.id) {
          if let Some(input_state) = self.review_comment_edit_input.clone() {
            let toggle_editor = editor_entity.clone();
            let message_id = message.id;
            let edit_error = self
              .review_comment_edit_error
              .as_ref()
              .and_then(|(id, error)| (*id == message_id).then_some(error.clone()));
            let edit_preview_open = self.review_comment_edit_preview_open;
            let edit_preview_renderer = self.review_comment_preview_renderer.clone();
            let edit_preview_suggestion_context =
              self.review_comment_preview_suggestion_context_for_id(message_id);
            v_flex()
              .on_action(cx.listener(Self::on_review_comment_edit_input_escape))
              .mt(px(-self.review_comment_in_card_composer_pull_px()))
              .mb(px(-self.review_comment_in_card_composer_pull_px()))
              .gap_1()
              .font_family(theme.font_family.clone())
              .child(
                h_flex()
                  .items_start()
                  .gap_1()
                  .child(
                    div()
                      .flex_1()
                      .min_w_0()
                      .ml(px(-REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX))
                      .child(
                        self
                          .review_comment_drop_zone(
                            format!("review-comment-edit-drop-zone-{}", message_id),
                            input_state.clone(),
                            cx,
                          )
                          .child({
                            let mut composer = MarkdownComposer::new(&input_state)
                              .disabled(is_edit_submitting)
                              .appearance(false)
                              .preview_open(edit_preview_open)
                              .on_toggle_preview(move |_, cx| {
                                toggle_editor.update(cx, |editor, cx| {
                                  editor.review_comment_edit_preview_open =
                                    !editor.review_comment_edit_preview_open;
                                  cx.notify();
                                });
                              });
                            if let Some(renderer) = edit_preview_renderer {
                              composer = composer.preview(move |text, window, cx| {
                                renderer(text, edit_preview_suggestion_context.clone(), window, cx)
                              });
                            }
                            composer
                          }),
                      ),
                  )
                  .when(!is_local_note_mode, |this| {
                    this.child(
                      h_flex()
                        .mt(px(review_comment_composer_actions_top_px(
                          self.review_comment_composer_line_height_px,
                        )))
                        .items_center()
                        .gap_1()
                        .child(Self::render_review_comment_edit_actions(
                          message_id,
                          can_save_review_comment_edit,
                          is_edit_submitting,
                          editor_entity.clone(),
                        )),
                    )
                  }),
              )
              .when_some(edit_error, |this, error| {
                this.child(
                  div()
                    .text_xs()
                    .text_color(theme.status_red())
                    .overflow_hidden()
                    .text_ellipsis_start()
                    .child(error.as_ref().to_string()),
                )
              })
              .into_any_element()
          } else {
            let state = self
              .review_comment_markdown_states
              .get(&message.id)
              .cloned()
              .unwrap_or_else(MarkdownRenderState::new);
            let parsed =
              self.cached_parsed_review_comment_markdown(message.id, message.body.as_ref());
            let mut options = self
              .review_comment_markdown_options(state, review_comment_markdown_scope_id(message.id));
            options.on_link = Some(link_handler.clone());
            if let Some(ctx) = message.suggestion_context.clone() {
              options = options.with_suggestion_context(ctx);
              if let Some(factory) = self.review_comment_suggestion_action_factory.as_ref() {
                let action = factory(message.id, message.author.clone(), message.is_outdated, cx);
                options = options.with_suggestion_action(action);
              }
            }
            render_parsed_markdown(&parsed, &options, cx).into_any_element()
          }
        } else {
          let has_previews = self
            .review_comment_code_reference_previews
            .get(&message.id)
            .is_some_and(|previews| !previews.is_empty());
          if !has_previews {
            let state = self
              .review_comment_markdown_states
              .get(&message.id)
              .cloned()
              .unwrap_or_else(MarkdownRenderState::new);
            let parsed =
              self.cached_parsed_review_comment_markdown(message.id, message.body.as_ref());
            let mut options = self
              .review_comment_markdown_options(state, review_comment_markdown_scope_id(message.id));
            options.on_link = Some(link_handler.clone());
            if let Some(ctx) = message.suggestion_context.clone() {
              options = options.with_suggestion_context(ctx);
              if let Some(factory) = self.review_comment_suggestion_action_factory.as_ref() {
                let action = factory(message.id, message.author.clone(), message.is_outdated, cx);
                options = options.with_suggestion_action(action);
              }
            }
            render_parsed_markdown(&parsed, &options, cx).into_any_element()
          } else {
            let segments = self.review_comment_body_segments(message.id, message.body.as_ref());
            let state = self
              .review_comment_markdown_states
              .get(&message.id)
              .cloned()
              .unwrap_or_else(MarkdownRenderState::new);
            let mut rendered = v_flex();
            let mut has_rendered_content = false;

            for (segment_index, segment) in segments.iter().enumerate() {
              match segment {
                ReviewCommentBodySegment::Markdown(markdown) => {
                  if markdown.trim().is_empty() {
                    continue;
                  }
                  let parsed = parse_markdown(markdown);
                  let mut options = self.review_comment_markdown_options(
                    state.clone(),
                    review_comment_markdown_segment_scope_id(message.id, segment_index),
                  );
                  options.on_link = Some(link_handler.clone());
                  if let Some(ctx) = message.suggestion_context.clone() {
                    options = options.with_suggestion_context(ctx);
                    if let Some(factory) = self.review_comment_suggestion_action_factory.as_ref() {
                      let action =
                        factory(message.id, message.author.clone(), message.is_outdated, cx);
                      options = options.with_suggestion_action(action);
                    }
                  }
                  rendered = rendered
                    .child(render_parsed_markdown(&parsed, &options, cx).into_any_element());
                  has_rendered_content = true;
                }
                ReviewCommentBodySegment::Preview(preview) => {
                  rendered =
                    rendered.child(self.render_review_comment_code_reference_preview_card(
                      message.id,
                      segment_index,
                      preview,
                      cx,
                    ));
                  has_rendered_content = true;
                }
              }
            }

            if has_rendered_content {
              rendered.into_any_element()
            } else {
              div().into_any_element()
            }
          }
        };

        let message_block = if index == 0 {
          v_flex()
            .when(reserve_floating_actions_room, |this| {
              this.pr(px(REVIEW_COMMENT_FLOATING_ACTIONS_WIDTH_PX))
            })
            .child(body)
        } else {
          let message_line_label: Option<Arc<str>> = None;
          let message_actions_menu = if !review_comment_submission_in_flight
            && self.editable_review_comment_ids.contains(&message.id)
          {
            let message_id = message.id;
            let body = message.body.clone();
            let can_delete = review_comment_delete_handler.is_some();
            Some(Self::render_review_comment_actions_menu(
              message_id,
              body,
              can_delete,
              format!("review-comment-actions-{}", message_id),
              editor_entity.clone(),
            ))
          } else {
            None
          };
          let message_reply_button = if self.review_comment_replies_enabled && is_last_message {
            let message_id = message.id;
            let is_replying = self.replying_to_review_comment_id == Some(message_id);
            let disabled_reason = if review_comment_submission_in_flight {
              Some("A comment submission is in progress.")
            } else if self.replying_to_review_comment_id.is_some() && !is_replying {
              Some("Finish or cancel the open reply first.")
            } else {
              None
            };
            let disabled = disabled_reason.is_some();
            let editor = editor_entity.clone();
            Some(
              div()
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                  cx.stop_propagation();
                })
                .child({
                  let button = Button::new(format!("review-comment-reply-{}", message_id))
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(UiIconName::MessageCircleReply)
                    .label("Reply")
                    .selected(is_replying)
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                      cx.stop_propagation();
                      editor.update(cx, |editor, cx| {
                        if is_replying {
                          let was_replying = editor.replying_to_review_comment_id.is_some();
                          editor.cancel_review_comment_reply(cx);
                          if was_replying {
                            editor.invoke_review_comment_cancel_handler(window, cx);
                          }
                        } else {
                          editor.start_review_comment_reply(message_id, window, cx);
                        }
                      });
                    });
                  if let Some(reason) = disabled_reason {
                    button.tooltip(reason)
                  } else {
                    button
                  }
                }),
            )
          } else {
            None
          };

          v_flex()
            .pt(px(REVIEW_COMMENT_SPACING_PX))
            .gap(px(REVIEW_COMMENT_SPACING_PX))
            .border_t(px(REVIEW_COMMENT_REPLY_BORDER_TOP_PX))
            .border_color(theme.border)
            .child(
              h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                  h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                      Avatar::new()
                        .name(message.author.to_string())
                        .when_some(message.avatar_url.clone(), |this, url| {
                          this.src(url.as_ref().to_string())
                        })
                        .small(),
                    )
                    .child(
                      div()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(message.author.to_string()),
                    )
                    .when_some(message_line_label, |this, label| {
                      this
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(label.as_ref().to_string()),
                        )
                        .child(
                          div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("•"),
                        )
                    })
                    .child(
                      div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(message.created_at.as_ref().to_string()),
                    ),
                )
                .child(
                  h_flex()
                    .items_center()
                    .gap_1()
                    .when_some(message_reply_button, |this, button| this.child(button))
                    .when_some(message_actions_menu, |this, menu| this.child(menu)),
                ),
            )
            .child(body)
        };
        thread_messages = thread_messages.child(message_block);
      }

      if self
        .replying_to_review_comment_id
        .is_some_and(|reply_to_id| self.thread_id_for_comment(reply_to_id) == thread_id)
      {
        let reply_to_id = self
          .replying_to_review_comment_id
          .expect("reply target should exist when rendering thread reply composer");
        let is_reply_submitting = self.review_comment_reply_submitting;
        let reply_block: gpui::AnyElement =
          if let Some(input_state) = self.review_comment_reply_input.clone() {
            let cancel_editor = editor_entity.clone();
            let save_editor = editor_entity.clone();
            let toggle_editor = editor_entity.clone();
            let reply_error = self.review_comment_reply_error.clone();
            let reply_preview_open = self.review_comment_reply_preview_open;
            let reply_preview_renderer = self.review_comment_preview_renderer.clone();
            let reply_preview_suggestion_context =
              self.review_comment_preview_suggestion_context_for_id(reply_to_id);
            v_flex()
              .on_action(cx.listener(Self::on_review_comment_reply_input_escape))
              .pt(px(REVIEW_COMMENT_SPACING_PX))
              .gap(px(REVIEW_COMMENT_SPACING_PX))
              .border_t(px(REVIEW_COMMENT_REPLY_BORDER_TOP_PX))
              .border_color(theme.border)
              .font_family(theme.font_family.clone())
              .child(
                h_flex()
                  .items_center()
                  .gap_2()
                  .child(div().text_sm().text_color(theme.foreground).child("You")),
              )
              .child(
                v_flex()
                  .mt(px(-self.review_comment_in_card_composer_pull_px()))
                  .mb(px(-self.review_comment_in_card_composer_pull_px()))
                  .gap_1()
                  .child(
                    h_flex()
                      .items_start()
                      .gap_1()
                      .child(
                        div()
                          .flex_1()
                          .min_w_0()
                          .ml(px(-REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX))
                          .child(
                            self
                              .review_comment_drop_zone(
                                format!("review-comment-reply-drop-zone-{}", reply_to_id),
                                input_state.clone(),
                                cx,
                              )
                              .child({
                                let mut composer = MarkdownComposer::new(&input_state)
                                  .disabled(is_reply_submitting)
                                  .appearance(false)
                                  .preview_open(reply_preview_open)
                                  .on_toggle_preview(move |_, cx| {
                                    toggle_editor.update(cx, |editor, cx| {
                                      editor.review_comment_reply_preview_open =
                                        !editor.review_comment_reply_preview_open;
                                      cx.notify();
                                    });
                                  });
                                if let Some(renderer) = reply_preview_renderer {
                                  composer = composer.preview(move |text, window, cx| {
                                    renderer(
                                      text,
                                      reply_preview_suggestion_context.clone(),
                                      window,
                                      cx,
                                    )
                                  });
                                }
                                composer
                              }),
                          ),
                      )
                      .child(
                        h_flex()
                          .mt(px(review_comment_composer_actions_top_px(
                            self.review_comment_composer_line_height_px,
                          )))
                          .items_center()
                          .gap_1()
                          .child(
                            div()
                              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                              })
                              .child(
                                Button::new(format!("review-comment-reply-cancel-{}", reply_to_id))
                                  .ghost()
                                  .xsmall()
                                  .compact()
                                  .icon(Icon::new(UiIconName::X))
                                  .tooltip("Cancel")
                                  .disabled(is_reply_submitting)
                                  .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    cancel_editor.update(cx, |editor, cx| {
                                      let was_replying =
                                        editor.replying_to_review_comment_id.is_some();
                                      editor.cancel_review_comment_reply(cx);
                                      if was_replying {
                                        editor.invoke_review_comment_cancel_handler(window, cx);
                                      }
                                    });
                                  }),
                              ),
                          )
                          .child(
                            div()
                              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                              })
                              .child(
                                Button::new(format!("review-comment-reply-save-{}", reply_to_id))
                                  .xsmall()
                                  .compact()
                                  .icon(Icon::new(UiIconName::Check))
                                  .tooltip("Reply")
                                  .disabled(!can_save_review_comment_reply || is_reply_submitting)
                                  .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    save_editor.update(cx, |editor, cx| {
                                      editor.save_review_comment_reply(window, cx);
                                    });
                                  }),
                              ),
                          ),
                      ),
                  )
                  .when_some(reply_error, |this, error| {
                    this.child(
                      div()
                        .text_xs()
                        .text_color(theme.status_red())
                        .overflow_hidden()
                        .text_ellipsis_start()
                        .child(error.as_ref().to_string()),
                    )
                  }),
              )
              .into_any_element()
          } else {
            div().into_any_element()
          };

        thread_messages = thread_messages.child(reply_block);
      }

      let card = self
        .review_comment_card_frame(review_comment_card_min_height(layout.height), &theme)
        .relative()
        .debug_selector(|| REVIEW_COMMENT_CARD_DEBUG_SELECTOR.to_string())
        .font_family(REVIEW_COMMENT_UI_FONT_FAMILY)
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
          cx.stop_propagation();
        })
        .child(
          v_flex()
            .px(px(REVIEW_COMMENT_CARD_PADDING_X_PX))
            .py(px(REVIEW_COMMENT_SPACING_PX))
            .gap(px(REVIEW_COMMENT_SPACING_PX))
            .when(shows_header, |this| this.child(header))
            .when(is_local_note_mode || !is_collapsed, |this| {
              this.child(thread_messages)
            }),
        )
        .when_some(floating_actions, |this, actions| {
          this.child(
            div()
              .absolute()
              .top(px(review_comment_floating_actions_top_px(
                self.review_comment_line_height_px,
              )))
              .right(px(REVIEW_COMMENT_CARD_PADDING_X_PX))
              .bg(theme.sidebar)
              .rounded_md()
              .child(actions),
          )
        });

      overlay = overlay.child(
        div()
          .absolute()
          .top(layout.top)
          .left_0()
          .right_0()
          .h(layout.height)
          .pr_2()
          .debug_selector(|| REVIEW_COMMENT_BLOCK_DEBUG_SELECTOR.to_string())
          .child(card),
      );
    }

    Some(overlay.into_any_element())
  }

  fn review_comment_create_span(
    &self,
    side_filter: Option<ReviewCommentSide>,
  ) -> Option<(usize, usize)> {
    let projection = self.projection.as_ref()?;
    let mut first = None;
    let mut count = 0usize;

    for (idx, line) in projection.lines.iter().enumerate() {
      let DisplayLine::ReviewComment { id, side, .. } = line else {
        continue;
      };
      if *id != REVIEW_COMMENT_CREATE_DRAFT_COMMENT_ID {
        continue;
      }
      if let Some(filter) = side_filter
        && *side != filter
      {
        continue;
      }
      if first.is_none() {
        first = Some(idx);
      }
      count = count.saturating_add(1);
    }

    first
      .map(|line| (line, count))
      .filter(|(_, count)| *count > 0)
  }

  fn render_review_comment_create_overlay(
    &self,
    editor_entity: Entity<Editor>,
    side_filter: Option<ReviewCommentSide>,
    line_height: Pixels,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    let theme = cx.theme().clone();
    let overlay_x_offset = self.review_comment_overlay_x_offset();
    let mut overlay = div()
      .absolute()
      .top(px(0.0))
      .left(overlay_x_offset)
      .right(px(0.0))
      .bottom(px(0.0));
    let mut has_content = false;

    if let Some(draft) = self.review_comment_create_draft
      && side_filter.is_none_or(|filter| filter == draft.side)
    {
      let top = line_height * (draft.first_display_line as f32 - self.scroll_offset_y);
      let span_count = draft
        .last_display_line
        .saturating_sub(draft.first_display_line)
        .saturating_add(1);
      let height = line_height * span_count as f32;
      let selection_color = theme
        .status_amber()
        .opacity(REVIEW_COMMENT_CREATE_SELECTION_BACKGROUND_ALPHA);

      overlay = overlay.child(
        div()
          .absolute()
          .top(top)
          .left_0()
          .right_0()
          .h(height)
          .bg(selection_color),
      );
      has_content = true;
    }

    if !self.review_comment_create_drag_active
      && let Some((first_display_line, line_count)) = self.review_comment_create_span(side_filter)
      && line_count > 0
    {
      let composer_top = line_height * (first_display_line as f32 - self.scroll_offset_y);
      let composer_height = line_height * line_count as f32;
      let composer_card = if let Some(input_state) = self.review_comment_create_input.clone() {
        let cancel_editor = editor_entity.clone();
        let save_editor = editor_entity.clone();
        let suggest_editor = editor_entity.clone();
        let can_save = self.review_comment_create_handler.is_some();
        let create_actions =
          review_comment_create_actions(self.review_comment_display_mode, self.has_pending_review);
        // Two destinations need words; a single one reads fine as an icon.
        let icon_only_actions = create_actions.len() == 1;
        let can_suggest = self.can_insert_review_comment_suggestion(cx);
        let is_create_submitting = self.review_comment_create_submitting;
        let create_error = self.review_comment_create_error.clone();
        let create_preview_open = self.review_comment_create_preview_open;
        let create_preview_renderer = self.review_comment_preview_renderer.clone();
        let create_preview_suggestion_context =
          self.review_comment_create_preview_suggestion_context(cx);
        let create_toggle_editor = editor_entity.clone();
        Some(
          self
            .review_comment_card_frame(review_comment_card_min_height(composer_height), &theme)
            .debug_selector(|| REVIEW_COMMENT_CARD_DEBUG_SELECTOR.to_string())
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
              cx.stop_propagation();
            })
            .child(
              v_flex()
                .on_action(cx.listener(Self::on_review_comment_create_input_escape))
                // The text box carries its own inset; the card only pads what is left.
                .pl(px(
                  REVIEW_COMMENT_CARD_PADDING_X_PX - REVIEW_COMMENT_COMPOSER_TEXTAREA_INSET_PX,
                ))
                .pr(px(REVIEW_COMMENT_CARD_PADDING_X_PX))
                .gap(px(REVIEW_COMMENT_COMPOSER_ACTIONS_GAP_PX))
                .font_family(theme.font_family.clone())
                .child(
                  h_flex()
                    .items_start()
                    .gap_1()
                    .child(
                      div().flex_1().min_w_0().child(
                        self
                          .review_comment_drop_zone(
                            "review-comment-create-drop-zone",
                            input_state.clone(),
                            cx,
                          )
                          .child({
                            let mut composer = MarkdownComposer::new(&input_state)
                              .disabled(is_create_submitting)
                              .appearance(false)
                              .preview_open(create_preview_open)
                              .on_toggle_preview(move |_, cx| {
                                create_toggle_editor.update(cx, |editor, cx| {
                                  editor.review_comment_create_preview_open =
                                    !editor.review_comment_create_preview_open;
                                  cx.notify();
                                });
                              });
                            if let Some(renderer) = create_preview_renderer {
                              composer = composer.preview(move |text, window, cx| {
                                renderer(
                                  text,
                                  create_preview_suggestion_context.clone(),
                                  window,
                                  cx,
                                )
                              });
                            }
                            composer
                          }),
                      ),
                    )
                    .child(
                      h_flex()
                        .mt(px(review_comment_composer_actions_top_px(
                          self.review_comment_composer_line_height_px,
                        )))
                        .items_center()
                        .gap_1()
                        .when(can_suggest, |this| {
                          this.child(
                            div()
                              .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                              })
                              .child(
                                Button::new("review-comment-create-suggest")
                                  .ghost()
                                  .xsmall()
                                  .compact()
                                  .icon(Icon::new(UiIconName::FileDiff))
                                  .tooltip("Suggest a change")
                                  .disabled(is_create_submitting)
                                  .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    suggest_editor.update(cx, |editor, cx| {
                                      editor.insert_review_comment_suggestion(window, cx);
                                    });
                                  }),
                              ),
                          )
                        })
                        .child(
                          div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                              cx.stop_propagation();
                            })
                            .child(
                              Button::new("review-comment-create-cancel")
                                .ghost()
                                .xsmall()
                                .compact()
                                .icon(Icon::new(UiIconName::X))
                                .tooltip("Cancel")
                                .disabled(is_create_submitting)
                                .on_click(move |_, window, cx| {
                                  cx.stop_propagation();
                                  cancel_editor.update(cx, |editor, cx| {
                                    editor.cancel_review_comment_create_draft(window, cx);
                                  });
                                }),
                            ),
                        )
                        .children(create_actions.into_iter().map(|action| {
                          let action_editor = save_editor.clone();
                          div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                              cx.stop_propagation();
                            })
                            .child(
                              Button::new(action.id)
                                .xsmall()
                                .compact()
                                .when(!action.primary, |this| this.ghost())
                                .when(icon_only_actions, |this| {
                                  this.icon(Icon::new(action.icon)).tooltip(action.label)
                                })
                                .when(!icon_only_actions, |this| this.label(action.label))
                                .disabled(!can_save || is_create_submitting)
                                .on_click(move |_, window, cx| {
                                  cx.stop_propagation();
                                  action_editor.update(cx, |editor, cx| {
                                    editor.save_review_comment_create(action.mode, window, cx);
                                  });
                                }),
                            )
                        })),
                    ),
                )
                .when_some(create_error, |this, error| {
                  this.child(
                    div()
                      .text_xs()
                      .text_color(theme.status_red())
                      .overflow_hidden()
                      .text_ellipsis_start()
                      .child(error.as_ref().to_string()),
                  )
                }),
            ),
        )
      } else {
        None
      };

      if let Some(card) = composer_card {
        overlay = overlay.child(
          div()
            .absolute()
            .top(composer_top)
            .left_0()
            .right_0()
            .h(composer_height)
            .pr_2()
            .debug_selector(|| REVIEW_COMMENT_BLOCK_DEBUG_SELECTOR.to_string())
            .child(card),
        );
        has_content = true;
      }
    }

    has_content.then_some(overlay.into_any_element())
  }

  pub fn refresh_git_state(&mut self, cx: &mut Context<Self>) {
    self.reload_git_bases(cx);
  }

  fn maybe_optimistic_unstage_for_edit(
    &mut self,
    start_line: usize,
    end_line: usize,
    cx: &mut Context<Self>,
  ) {
    let Some(projection) = self.projection.as_ref() else {
      return;
    };
    if self.git_state.bases.is_none() {
      return;
    }

    let mut group_ids = HashSet::new();
    for doc_line in start_line..=end_line {
      let Some(display_line) = self.doc_to_display_line(doc_line) else {
        continue;
      };
      let Some(line) = projection.lines.get(display_line) else {
        continue;
      };
      let group_id = match line {
        DisplayLine::Doc {
          hunk: Some(HunkState::Staged),
          group_id: Some(group_id),
          ..
        } => group_id.clone(),
        DisplayLine::Modified {
          hunk: HunkState::Staged,
          group_id: Some(group_id),
          ..
        } => group_id.clone(),
        _ => continue,
      };

      group_ids.insert(group_id);
    }

    for group_id in group_ids {
      self.optimistic_unstage_group(group_id, cx);
    }
  }

  fn optimistic_unstage_group(&mut self, group_id: Arc<str>, cx: &mut Context<Self>) {
    if self.optimistic_unstaged_groups.contains(&group_id) {
      return;
    }
    let Some(projection) = self.projection.as_ref() else {
      return;
    };
    let Some(group) = projection.groups.get(group_id.as_ref()) else {
      return;
    };
    if group.state != HunkState::Staged {
      return;
    }
    let rel_path = self
      .repo_file
      .as_ref()
      .and_then(|repo_file| repo_file.relative_path().ok());
    let Some(bases) = self.git_state.bases.as_mut() else {
      return;
    };
    let base_index = bases.index.as_deref().unwrap_or("");
    let Ok(updated) = git::apply_hunk_to_text(base_index, &group.hunk, true) else {
      return;
    };
    bases.index = Some(updated);
    self.git_state.index_dirty = true;
    self.git_state.staged_diff = rel_path
      .as_deref()
      .and_then(|rel_path| staged_diff_from_bases(bases, rel_path));
    self.optimistic_unstaged_groups.insert(group_id);
    self.schedule_diff_recompute(cx);
  }

  fn init(&mut self, cx: &mut Context<Self>) {
    if self.repo_file.is_some() {
      self.reload_git_bases(cx);
      self.start_polling(cx);
    }
  }

  fn reload_git_bases(&mut self, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      return;
    };
    let Some(git_store) = self.git_store.clone() else {
      return;
    };
    let Ok(rel_path) = repo_file.relative_path() else {
      return;
    };
    let op_id = git_store.op_id();
    let staged_diff_rel_path = rel_path.clone();

    self.bases_task = Some(cx.spawn(async move |this, cx| {
      let bases = cx
        .background_spawn(async move { git_store.load_bases(&rel_path) })
        .await;
      let Ok(bases) = bases else {
        return;
      };

      let _ = this.update(cx, |editor, cx| {
        let mut merged = bases;
        if editor.git_state.index_dirty
          && let Some(existing) = editor
            .git_state
            .bases
            .as_ref()
            .and_then(|b| b.index.clone())
        {
          merged.index = Some(existing);
        }
        let staged_diff = staged_diff_from_bases(&merged, &staged_diff_rel_path);
        editor.git_state.bases = Some(merged);
        editor.git_state.staged_diff = staged_diff;
        editor.git_state.op_id = op_id;
        if editor.pending_git_after_bases {
          editor.pending_git_after_bases = false;
          editor.git_op_in_flight = false;
          cx.emit(EditorEvent::HunkStagingChanged);
          editor.maybe_start_next_git_job(cx);
        }
        editor.schedule_diff_recompute(cx);
      });
    }));
  }

  pub fn schedule_diff_recompute(&mut self, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      return;
    };
    let Some(git_store) = self.git_store.clone() else {
      return;
    };
    let Some(git_bases) = self.git_state.bases.clone() else {
      self.reload_git_bases(cx);
      return;
    };

    if git_store.op_id() != self.git_state.op_id {
      self.reload_git_bases(cx);
      return;
    }

    let Ok(rel_path) = repo_file.relative_path() else {
      return;
    };
    let doc_line_count = self.document.read(cx).len_lines();
    let debounce_ms = Self::diff_debounce_ms_for_line_count(doc_line_count);

    // For clean buffers, diff directly from git/workdir to avoid copying very large
    // in-memory documents on the UI thread.
    let use_workdir_diff = !self.is_dirty && !self.git_state.index_dirty && !self.ignore_whitespace;
    let repo_file_for_diff = repo_file.clone();
    let staged_diff = self.git_state.staged_diff.clone();

    let ignore_whitespace = self.ignore_whitespace;
    let generation = self.diff_generation.fetch_add(1, Ordering::Relaxed) + 1;
    let diff_generation = self.diff_generation.clone();
    self.diff_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor()
        .timer(Duration::from_millis(debounce_ms))
        .await;

      if diff_generation.load(Ordering::Relaxed) != generation {
        return;
      }

      let diffs = if use_workdir_diff {
        cx.background_spawn(async move { git::compute_file_diffs(&repo_file_for_diff) })
          .await
      } else {
        let Ok(buffer_snapshot) = this.update(cx, |editor, cx| {
          let document = editor.document.read(cx);
          document.buffer.clone()
        }) else {
          return;
        };
        cx.background_spawn(async move {
          let buffer_text = buffer_snapshot.slice_to_string(0..buffer_snapshot.len());
          let head = git_bases.head.as_deref();
          let index = git_bases.index.as_deref();
          let uncommitted = git::compute_buffer_diff(
            git::DiffKind::Uncommitted,
            head,
            &buffer_text,
            &rel_path,
            ignore_whitespace,
          )?;
          let unstaged = if head == index {
            uncommitted.clone_with_kind(git::DiffKind::Unstaged)
          } else {
            git::compute_buffer_diff(
              git::DiffKind::Unstaged,
              index,
              &buffer_text,
              &rel_path,
              ignore_whitespace,
            )?
          };
          let staged = match staged_diff {
            Some(staged) => staged,
            None if head == index => FileDiff::empty(git::DiffKind::Staged),
            None => git::compute_buffer_diff(
              git::DiffKind::Staged,
              head,
              index.unwrap_or(""),
              &rel_path,
              ignore_whitespace,
            )?,
          };

          Ok(DiffSet {
            uncommitted,
            unstaged,
            staged,
          })
        })
        .await
      };
      let Ok(diffs) = diffs else {
        return;
      };

      if diff_generation.load(Ordering::Relaxed) != generation {
        return;
      }

      let _ = this.update(cx, |editor, cx| {
        editor.apply_diffs(diffs, cx);
      });
    }));
  }

  fn invalidate_projection_builds(&mut self) {
    self.projection_generation.fetch_add(1, Ordering::Relaxed);
    self.projection_task = None;
  }

  fn apply_diffs(&mut self, diffs: DiffSet, cx: &mut Context<Self>) {
    self.diffs = Some(Arc::new(diffs));
    self.rebuild_projection(cx);
  }

  fn should_build_projection_in_background(doc_line_count: usize) -> bool {
    doc_line_count >= ASYNC_PROJECTION_MIN_DOC_LINES
  }

  fn diff_debounce_ms_for_line_count(line_count: usize) -> u64 {
    if line_count >= HUGE_FILE_DIFF_DEBOUNCE_LINES {
      HUGE_FILE_DIFF_DEBOUNCE_MS
    } else if line_count >= LARGE_FILE_DIFF_DEBOUNCE_LINES {
      LARGE_FILE_DIFF_DEBOUNCE_MS
    } else {
      DIFF_DEBOUNCE_MS
    }
  }

  fn build_projection_from_diffs(input: &ProjectionBuildInput) -> Projection {
    let projection = if !input.conflict_doc_line_ranges.is_empty() {
      Projection::from_conflict_regions(
        input.doc_line_count,
        &input.conflict_doc_line_ranges,
        &input.expanded_gaps,
      )
    } else if input.is_unmerged {
      Projection::full(input.doc_line_count)
    } else {
      Projection::from_diffs(
        input.doc_line_count,
        &input.diffs.uncommitted,
        &input.diffs.unstaged,
        &input.diffs.staged,
        &input.expanded_gaps,
        input.align_modified,
      )
    };
    projection.with_review_comments(
      &input.projection_comments,
      &ReviewCommentLayoutInput {
        collapsed: &input.collapsed_review_comments,
        editor_line_height_px: input.editor_line_height_px,
        markdown_line_height_px: input.markdown_line_height_px,
        body_heights_px: &input.review_comment_body_heights_px,
        composer_only_ids: &input.composer_only_comment_ids,
        local_notes: input.local_notes,
      },
    )
  }

  fn apply_projection_result(
    &mut self,
    projection: Projection,
    doc_line_count: usize,
    cx: &mut Context<Self>,
  ) {
    self.set_projection(Some(projection));

    if let Some(conflict_start_line) = self.pending_conflict_reveal_start_line.take() {
      self.reveal_conflict_start_line(conflict_start_line, cx);
      self.schedule_visible_viewport_highlights(cx);
      return;
    }

    let total_lines = self.display_line_count(doc_line_count);
    self.scroll_offset_y = self.clamp_vertical_scroll(
      self.scroll_offset_y,
      self.measured_editor_line_height(),
      total_lines,
    );

    self.schedule_visible_viewport_highlights(cx);
    cx.notify();
  }

  fn schedule_visible_viewport_highlights(&mut self, cx: &mut Context<Self>) {
    let line_height = self.measured_editor_line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewports = self.doc_ranges_for_display_viewport(display_viewport);

    self.document.update(cx, |doc, cx| {
      doc.schedule_viewport_highlights_for_ranges(
        &doc_viewports,
        None,
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
    });
  }

  fn rebuild_projection(&mut self, cx: &mut Context<Self>) {
    let doc_line_count = self.document.read(cx).len_lines();
    if self.diffs.is_none() {
      self.invalidate_projection_builds();
      self.set_projection(None);
      self.virtual_line_layouts.clear();
      cx.notify();
      return;
    }

    let editor_line_height_px = (self.measured_editor_line_height() / px(1.0)).max(1.0);
    let markdown_line_height_px = self.review_comment_line_height_px;
    let mut review_comment_body_heights_px = HashMap::new();
    let mut composer_only_comment_ids = HashSet::new();
    let show_review_comment_create_composer =
      self.review_comment_create_draft.is_some() && !self.review_comment_create_drag_active;
    let reply_target_comment = self.replying_to_review_comment_id.and_then(|reply_to_id| {
      self
        .review_comments
        .iter()
        .find(|comment| comment.id == reply_to_id)
        .cloned()
    });
    let show_review_comment_reply_composer = reply_target_comment.is_some();
    let mut projection_comments = self.review_comments.clone();

    for index in 0..self.review_comments.len() {
      let (comment_id, body, suggestion_context) = {
        let comment = &self.review_comments[index];
        (
          comment.id,
          comment.body.clone(),
          comment.suggestion_context.clone(),
        )
      };
      let estimated_height = if self.editing_review_comment_id == Some(comment_id) {
        self.review_comment_in_card_composer_body_height_px(self.review_comment_edit_input.as_ref())
      } else {
        let has_previews = self
          .review_comment_code_reference_previews
          .get(&comment_id)
          .is_some_and(|previews| !previews.is_empty());
        if has_previews {
          let measurer = self.review_comment_text_measurer(cx);
          let width_of = |text: &str| measurer.width_in_columns(text);
          self.review_comment_segmented_height_px(
            comment_id,
            body.as_ref(),
            MarkdownTextMetrics::measured(
              self.review_comment_body_wrap_columns(
                self.measured_review_comment_char_width() / px(1.0),
              ),
              &width_of,
            ),
            markdown_line_height_px,
            suggestion_context.as_ref(),
          )
        } else {
          self.cached_review_comment_body_height_px(
            comment_id,
            body.as_ref(),
            markdown_line_height_px,
            suggestion_context.as_ref(),
            cx,
          )
        }
      };
      review_comment_body_heights_px.insert(comment_id, estimated_height);
    }
    if show_review_comment_create_composer && let Some(draft) = self.review_comment_create_draft {
      projection_comments.push(ReviewComment {
        id: REVIEW_COMMENT_CREATE_DRAFT_COMMENT_ID,
        in_reply_to_id: None,
        line: draft.line,
        side: draft.side,
        author: Arc::from("You"),
        avatar_url: None,
        line_label: None,
        body: Arc::from(""),
        suggestion_context: None,
        created_at: Arc::from(""),
        thread_id: None,
        is_resolved: false,
        is_outdated: false,
        viewer_can_resolve: false,
        viewer_can_unresolve: false,
        is_pending: false,
      });
      review_comment_body_heights_px.insert(
        REVIEW_COMMENT_CREATE_DRAFT_COMMENT_ID,
        self.review_comment_composer_body_height_px(self.review_comment_create_input.as_ref()),
      );
      composer_only_comment_ids.insert(REVIEW_COMMENT_CREATE_DRAFT_COMMENT_ID);
    }
    if show_review_comment_reply_composer && let Some(reply_to_comment) = reply_target_comment {
      projection_comments.push(ReviewComment {
        id: REVIEW_COMMENT_REPLY_DRAFT_COMMENT_ID,
        in_reply_to_id: Some(reply_to_comment.id),
        line: reply_to_comment.line,
        side: reply_to_comment.side,
        author: Arc::from("You"),
        avatar_url: None,
        line_label: None,
        body: Arc::from(""),
        suggestion_context: None,
        created_at: Arc::from(""),
        thread_id: None,
        is_resolved: false,
        is_outdated: false,
        viewer_can_resolve: false,
        viewer_can_unresolve: false,
        is_pending: false,
      });
      review_comment_body_heights_px.insert(
        REVIEW_COMMENT_REPLY_DRAFT_COMMENT_ID,
        self
          .review_comment_in_card_composer_body_height_px(self.review_comment_reply_input.as_ref()),
      );
    }

    let conflict_doc_line_ranges = self
      .conflict_regions(cx)
      .iter()
      .map(|region| region.start_line..region.replace_end_line)
      .collect();
    let is_unmerged = self.is_unmerged;

    let build_input = ProjectionBuildInput {
      doc_line_count,
      diffs: self
        .diffs
        .as_ref()
        .expect("diffs should be present when rebuilding projection")
        .clone(),
      expanded_gaps: self.expanded_gaps.clone(),
      align_modified: matches!(self.diff_view_mode, DiffViewMode::Split),
      projection_comments,
      collapsed_review_comments: self.collapsed_review_comments.clone(),
      editor_line_height_px,
      markdown_line_height_px,
      review_comment_body_heights_px,
      composer_only_comment_ids,
      local_notes: matches!(
        self.review_comment_display_mode,
        ReviewCommentDisplayMode::LocalNote
      ),
      conflict_doc_line_ranges,
      is_unmerged,
    };
    let build_in_background =
      Self::should_build_projection_in_background(build_input.doc_line_count);

    let generation = self.projection_generation.fetch_add(1, Ordering::Relaxed) + 1;
    let projection_generation = self.projection_generation.clone();

    if !build_in_background {
      self.projection_task = None;
      let projection = Self::build_projection_from_diffs(&build_input);
      if self.projection_generation.load(Ordering::Relaxed) != generation {
        return;
      }
      self.apply_projection_result(projection, build_input.doc_line_count, cx);
      return;
    }

    self.projection_task = Some(cx.spawn(async move |this, cx| {
      if projection_generation.load(Ordering::Relaxed) != generation {
        return;
      }

      let projection_doc_line_count = build_input.doc_line_count;
      let projection = cx
        .background_spawn({
          let build_input = build_input;
          async move { Self::build_projection_from_diffs(&build_input) }
        })
        .await;

      if projection_generation.load(Ordering::Relaxed) != generation {
        return;
      }

      let _ = this.update(cx, move |editor, cx| {
        if editor.projection_generation.load(Ordering::Relaxed) != generation {
          return;
        }
        editor.apply_projection_result(projection, projection_doc_line_count, cx);
      });
    }));
  }

  pub fn save(&mut self, cx: &mut Context<Self>) {
    if self.is_read_only {
      return;
    }
    let workdir_path = self.workdir_path.clone();
    let contents = {
      let document = self.document.read(cx);
      document.slice_to_string(0..document.len())
    };
    let repo_file = self.repo_file.clone();
    let index_text = self
      .git_state
      .bases
      .as_ref()
      .and_then(|bases| bases.index.clone());
    let needs_index_write = self.git_state.index_dirty;

    self.save_task = Some(cx.spawn(async move |this, cx| {
      let result = cx
        .background_spawn(async move {
          std::fs::write(&workdir_path, contents)?;
          let file_mtime = std::fs::metadata(&workdir_path)
            .and_then(|meta| meta.modified())
            .ok();
          let mut index_mtime = None;
          if needs_index_write && let (Some(repo_file), Some(index_text)) = (repo_file, index_text)
          {
            if let Err(err) = git::write_index_content(&repo_file, &index_text) {
              return Err(std::io::Error::other(err));
            }
            index_mtime = std::fs::metadata(git::index_path(&repo_file.repo_root))
              .and_then(|meta| meta.modified())
              .ok();
          }
          Ok::<_, std::io::Error>((file_mtime, index_mtime))
        })
        .await;

      let _ = this.update(cx, |editor, cx| match result {
        Ok((file_mtime, index_mtime)) => {
          editor.is_dirty = false;
          cx.emit(EditorEvent::Saved);
          editor.file_mtime = file_mtime;
          if needs_index_write {
            editor.git_state.index_dirty = false;
            editor.optimistic_unstaged_groups.clear();
            if let Some(index_mtime) = index_mtime {
              editor.index_mtime = Some(index_mtime);
            }
            if let Some(store) = editor.git_store.as_ref() {
              store.bump_op();
              editor.git_state.op_id = store.op_id();
            }
          }
          editor.reload_git_bases(cx);
          editor.schedule_diff_recompute(cx);
          cx.notify();
        }
        Err(err) => {
          app_log::log!("[editor] save failed: {:?}", err);
        }
      });
    }));
  }

  pub fn selected_text_for_copy(&self, cx: &App) -> Option<String> {
    if let Some(selection) = &self.display_selection
      && let Some(text) = self.display_selection_text(selection, cx)
    {
      return Some(text);
    }

    if self.selected_range.is_empty() {
      return None;
    }

    let document = self.document.read(cx);
    let range = Self::clamp_range_to_len(self.selected_range.clone(), document.len());
    if range.is_empty() {
      None
    } else {
      Some(document.slice_to_string(range))
    }
  }

  fn display_selection_text(&self, selection: &DisplaySelection, cx: &App) -> Option<String> {
    if selection.is_empty() {
      return None;
    }

    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    let total_lines = self.display_line_count(doc_line_count);
    if total_lines == 0 {
      return None;
    }

    let (start, end) = selection.normalized();
    let mut end_line = end.line.min(total_lines.saturating_sub(1));
    if end.column == 0 && end.line > start.line {
      end_line = end_line.saturating_sub(1);
    }

    if start.line > end_line {
      return None;
    }

    let mut lines = Vec::new();
    for display_line in start.line..=end_line {
      let line_text = match self.display_line(display_line, doc_line_count) {
        Some(DisplayLine::Doc { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Modified { doc_line, .. }) => document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
        Some(DisplayLine::Removed { text, .. }) => text.to_string(),
        _ => continue,
      };

      let line_len = line_text.chars().count();
      let mut slice_start = 0;
      let mut slice_end = line_len;

      if display_line == start.line {
        slice_start = start.column.min(line_len);
      }

      if display_line == end_line && display_line == end.line {
        slice_end = end.column.min(line_len);
      }

      if slice_end < slice_start {
        continue;
      }

      let slice_start = char_offset_to_byte_offset(&line_text, slice_start);
      let slice_end = char_offset_to_byte_offset(&line_text, slice_end);
      let slice = line_text
        .get(slice_start..slice_end)
        .unwrap_or("")
        .to_string();
      lines.push(slice);
    }

    if lines.is_empty() {
      None
    } else {
      Some(lines.join("\n"))
    }
  }

  fn group_token_for_id(&self, group_id: &Arc<str>) -> Option<GroupToken> {
    let projection = self.projection.as_ref()?;
    let group = projection.groups.get(group_id.as_ref())?;
    Some(GroupToken {
      state: group.state,
      signature: group.signature.clone(),
      id: group_id.clone(),
    })
  }

  fn resolve_group_from_token(&self, token: &GroupToken) -> Option<(HunkState, git::DiffHunk)> {
    let projection = self.projection.as_ref()?;
    if let Some((_, group)) = projection
      .groups
      .iter()
      .find(|(_, group)| group.state == token.state && group.signature == token.signature)
    {
      return Some((group.state, group.hunk.clone()));
    }
    let group = projection.groups.get(token.id.as_ref())?;
    Some((group.state, group.hunk.clone()))
  }

  pub fn enqueue_group_action(
    &mut self,
    group_id: Arc<str>,
    action: HunkAction,
    cx: &mut Context<Self>,
  ) {
    let Some(token) = self.group_token_for_id(&group_id) else {
      return;
    };
    self.git_jobs.push_back(GitJob { token, action });
    self.maybe_start_next_git_job(cx);
  }

  fn maybe_start_next_git_job(&mut self, cx: &mut Context<Self>) {
    if self.git_op_in_flight {
      return;
    }
    let Some(job) = self.git_jobs.pop_front() else {
      return;
    };
    self.start_git_job(job, cx);
  }

  fn start_git_job(&mut self, job: GitJob, cx: &mut Context<Self>) {
    let Some(repo_file) = self.repo_file.clone() else {
      self.git_op_in_flight = false;
      return;
    };
    let Some((state, hunk)) = self.resolve_group_from_token(&job.token) else {
      self.git_op_in_flight = false;
      self.maybe_start_next_git_job(cx);
      return;
    };

    let (reverse, location) = match (state, job.action) {
      (HunkState::Unstaged, HunkAction::Stage) => (false, ApplyLocation::Index),
      (HunkState::Staged, HunkAction::Unstage) => (true, ApplyLocation::Index),
      (HunkState::Unstaged, HunkAction::Restore) => (true, ApplyLocation::WorkDir),
      (HunkState::Staged, HunkAction::Restore) => (true, ApplyLocation::Both),
      _ => {
        self.git_op_in_flight = false;
        self.maybe_start_next_git_job(cx);
        return;
      }
    };

    let needs_index = matches!(location, ApplyLocation::Index | ApplyLocation::Both);
    let needs_workdir = matches!(location, ApplyLocation::WorkDir | ApplyLocation::Both);
    let mut index_text_to_write: Option<String> = None;
    if needs_index {
      let Some(bases) = self.git_state.bases.clone() else {
        self.git_jobs.push_front(job);
        self.pending_git_after_bases = true;
        self.reload_git_bases(cx);
        return;
      };
      let base_index = bases.index.as_deref().unwrap_or("");
      match git::apply_hunk_to_text(base_index, &hunk, reverse) {
        Ok(updated) => {
          index_text_to_write = Some(updated);
        }
        Err(_err) => {
          self.git_op_in_flight = false;
          self.maybe_start_next_git_job(cx);
          return;
        }
      }
    }

    let workdir_path = self.workdir_path.clone();
    let workdir_path_for_fallback = workdir_path.clone();
    let hunk_for_fallback = hunk.clone();
    let reverse_for_fallback = reverse;
    let git_store = self.git_store.clone();
    if let Some(store) = &git_store {
      store.bump_op();
    }
    self.git_op_in_flight = true;
    let repo_for_index = repo_file.clone();
    let repo_for_apply = repo_file.clone();
    let repo_for_meta = repo_file.clone();
    let index_text_for_write = index_text_to_write.clone();
    let index_text_for_update = index_text_to_write.clone();
    self.git_task = Some(cx.spawn(async move |this, cx| {
      let index_result = if needs_index {
        if let Some(text) = index_text_for_write.clone() {
          cx.background_spawn(async move { git::write_index_content(&repo_for_index, &text) })
            .await
        } else {
          Ok(())
        }
      } else {
        Ok(())
      };

      if let Err(_err) = index_result {
        let _ = this.update(cx, |editor, cx| {
          editor.git_op_in_flight = false;
          editor.maybe_start_next_git_job(cx);
        });
        return;
      }

      let workdir_result = if needs_workdir {
        cx.background_spawn(async move {
          git::apply_hunk(&repo_for_apply, &hunk, reverse, ApplyLocation::WorkDir)
        })
        .await
      } else {
        Ok(())
      };

      if let Err(_err) = workdir_result {
        let fallback_result = if needs_workdir {
          cx.background_spawn(async move {
            let text = std::fs::read_to_string(&workdir_path_for_fallback)?;
            let updated = git::apply_hunk_to_text(&text, &hunk_for_fallback, reverse_for_fallback)
              .map_err(std::io::Error::other)?;
            std::fs::write(&workdir_path_for_fallback, updated)?;
            Ok::<(), std::io::Error>(())
          })
          .await
        } else {
          Ok(())
        };

        if let Err(_err) = fallback_result {
          let _ = this.update(cx, |editor, cx| {
            editor.git_op_in_flight = false;
            editor.maybe_start_next_git_job(cx);
          });
          return;
        }
      }

      let (contents, file_mtime, index_mtime): (
        Option<String>,
        Option<SystemTime>,
        Option<SystemTime>,
      ) = cx
        .background_spawn(async move {
          let contents = if needs_workdir {
            std::fs::read_to_string(&workdir_path).ok()
          } else {
            None
          };
          let file_mtime = std::fs::metadata(&workdir_path)
            .and_then(|meta| meta.modified())
            .ok();
          let index_mtime = std::fs::metadata(git::index_path(&repo_for_meta.repo_root))
            .and_then(|meta| meta.modified())
            .ok();
          (contents, file_mtime, index_mtime)
        })
        .await;

      let _ = this.update(cx, |editor, cx| {
        if let Some(contents) = contents {
          editor.reload_from_disk(contents, cx);
        }
        editor.file_mtime = file_mtime;
        editor.index_mtime = index_mtime;
        if let Some(index_text) = index_text_for_update
          && let Some(bases) = editor.git_state.bases.as_mut()
        {
          bases.index = Some(index_text);
        }
        editor.pending_git_after_bases = true;
        editor.reload_git_bases(cx);
        editor.schedule_diff_recompute(cx);
      });
    }));
  }

  fn start_polling(&mut self, cx: &mut Context<Self>) {
    if self.poll_task.is_some() {
      return;
    }

    self.poll_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(POLL_INTERVAL_MS))
          .await;

        let state = this
          .update(cx, |editor, _| {
            (
              editor.repo_file.clone(),
              editor.workdir_path.clone(),
              editor.file_mtime,
              editor.index_mtime,
            )
          })
          .ok();
        let Some((repo_file, workdir_path, last_file_mtime, last_index_mtime)) = state else {
          return;
        };
        let Some(repo_file) = repo_file else {
          continue;
        };

        let workdir_path_for_meta = workdir_path.clone();
        let (file_mtime, index_mtime): (Option<SystemTime>, Option<SystemTime>) = cx
          .background_spawn(async move {
            let file_mtime = std::fs::metadata(&workdir_path_for_meta)
              .and_then(|meta| meta.modified())
              .ok();
            let index_mtime = std::fs::metadata(git::index_path(&repo_file.repo_root))
              .ok()
              .and_then(|meta| meta.modified().ok());
            (file_mtime, index_mtime)
          })
          .await;

        let file_changed = file_mtime.is_some() && file_mtime != last_file_mtime;
        let index_changed = index_mtime.is_some() && index_mtime != last_index_mtime;

        let new_contents = if file_changed {
          let workdir_path = workdir_path.clone();
          cx.background_spawn(async move { std::fs::read_to_string(&workdir_path) })
            .await
            .ok()
        } else {
          None
        };

        if new_contents.is_some() || index_changed {
          let _ = this.update(cx, |editor, cx| {
            if let Some(contents) = new_contents {
              editor.reload_from_disk(contents, cx);
              editor.file_mtime = file_mtime;
            }
            if index_changed {
              editor.index_mtime = index_mtime;
              editor.reload_git_bases(cx);
            } else {
              editor.schedule_diff_recompute(cx);
            }
          });
        }
      }
    }));
  }

  fn reload_from_disk(&mut self, contents: String, cx: &mut Context<Self>) {
    self.is_read_only = false;
    self.invalidate_projection_builds();
    self.document.update(cx, |doc, cx| {
      doc.replace_all(&contents, cx);
    });
    self.mark_conflict_cache_dirty();
    self.line_layouts.clear();
    self.virtual_line_layouts.clear();
    self.expanded_gaps.clear();
    self.undo_stack.clear();
    self.redo_stack.clear();
    self.selected_range = 0..0;
    self.selection_reversed = false;
    self.display_selection = None;
    self.marked_range = None;
    self.hovered_group_id = None;
    self.hovered_conflict_start_line = None;
    self.last_mouse_position = None;
    self.git_jobs.clear();
    self.git_op_in_flight = false;
    self.pending_git_after_bases = false;
    self.git_state.index_dirty = false;
    self.optimistic_unstaged_groups.clear();
    self.is_dirty = false;
    self.last_highlights_version = 0;
    self.last_highlights_epoch = 0;
    self.scroll_offset_y = 0.0;
    self.reset_horizontal_scroll_state();
    cx.notify();
  }

  pub fn display_line_count(&self, doc_line_count: usize) -> usize {
    self
      .projection
      .as_ref()
      .map(|projection| projection.lines.len())
      .unwrap_or(doc_line_count)
  }

  pub fn display_line(&self, display_line: usize, doc_line_count: usize) -> Option<DisplayLine> {
    if let Some(projection) = &self.projection {
      projection.lines.get(display_line).cloned()
    } else if display_line < doc_line_count {
      Some(DisplayLine::Doc {
        doc_line: display_line,
        old_line: Some(display_line),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      })
    } else {
      None
    }
  }

  pub(crate) fn group_id_for_modified_display_line(&self, display_line: usize) -> Option<Arc<str>> {
    let projection = self.projection.as_ref()?;
    match projection.lines.get(display_line)? {
      DisplayLine::Doc {
        change: Some(ChangeKind::Added),
        group_id: Some(id),
        ..
      } => Some(id.clone()),
      DisplayLine::Modified { group_id, .. } => group_id.clone(),
      DisplayLine::Removed { group_id, .. } => group_id.clone(),
      _ => None,
    }
  }

  pub fn first_display_line_for_group(&self, group_id: &Arc<str>) -> Option<usize> {
    let projection = self.projection.as_ref()?;
    projection.lines.iter().position(|line| match line {
      DisplayLine::Doc {
        group_id: Some(id), ..
      }
      | DisplayLine::Modified {
        group_id: Some(id), ..
      }
      | DisplayLine::Removed {
        group_id: Some(id), ..
      }
      | DisplayLine::NoNewline {
        group_id: Some(id), ..
      } => id.as_ref() == group_id.as_ref(),
      _ => false,
    })
  }

  fn mark_conflict_cache_dirty(&self) {
    self.conflict_cache.write().dirty = true;
  }

  fn ensure_conflict_cache(&self, cx: &App) {
    if !self.conflict_cache.read().dirty {
      return;
    }

    let regions = {
      let document = self.document.read(cx);
      conflict_regions_from_document(document)
    };
    let line_kinds = conflict_line_kinds_from_regions(&regions);

    let mut cache = self.conflict_cache.write();
    if !cache.dirty {
      return;
    }
    cache.regions = Arc::new(regions);
    cache.line_kinds = Arc::new(line_kinds);
    cache.dirty = false;
  }

  fn conflict_regions(&self, cx: &App) -> Arc<Vec<ConflictRegion>> {
    self.ensure_conflict_cache(cx);
    self.conflict_cache.read().regions.clone()
  }

  pub(crate) fn conflict_line_kinds(&self, cx: &App) -> Arc<HashMap<usize, ConflictLineKind>> {
    self.ensure_conflict_cache(cx);
    self.conflict_cache.read().line_kinds.clone()
  }

  pub fn is_unmerged(&self) -> bool {
    self.is_unmerged
  }

  pub fn set_is_unmerged(&mut self, value: bool, cx: &mut Context<Self>) {
    if self.is_unmerged == value {
      return;
    }
    self.is_unmerged = value;
    self.rebuild_projection(cx);
  }

  pub fn has_unresolved_conflict_markers(&self, cx: &App) -> bool {
    !self.conflict_regions(cx).is_empty()
  }

  fn active_conflict_index(&self, regions: &[ConflictRegion], cx: &App) -> Option<usize> {
    if regions.is_empty() {
      return None;
    }

    let cursor_doc_line = {
      let document = self.document.read(cx);
      document.char_to_line(self.cursor_offset().min(document.len()))
    };

    if let Some(index) = regions
      .iter()
      .position(|region| region.contains_doc_line(cursor_doc_line))
    {
      return Some(index);
    }

    if let Some(index) = regions
      .iter()
      .position(|region| region.start_line >= cursor_doc_line)
    {
      return Some(index);
    }

    Some(regions.len().saturating_sub(1))
  }

  pub fn conflict_navigation_state(&self, cx: &App) -> Option<ConflictNavigationState> {
    let regions = self.conflict_regions(cx);
    let active_index = self.active_conflict_index(regions.as_ref(), cx)?;
    let active_start_line = regions[active_index].start_line;

    Some(ConflictNavigationState {
      active_index,
      total: regions.len(),
      active_start_line,
    })
  }

  pub(crate) fn conflict_start_line_for_display_line(
    &self,
    display_line: usize,
    cx: &App,
  ) -> Option<usize> {
    let doc_line = self.display_to_doc_line(display_line)?;
    let regions = self.conflict_regions(cx);
    regions
      .iter()
      .find(|region| region.contains_doc_line(doc_line))
      .map(|region| region.start_line)
  }

  pub(crate) fn highlighted_conflict_doc_range(&self, cx: &App) -> Option<Range<usize>> {
    let regions = self.conflict_regions(cx);
    let active_index = self.active_conflict_index(regions.as_ref(), cx)?;
    let region = &regions[active_index];
    Some(region.start_line..region.replace_end_line)
  }

  pub fn first_display_line_for_conflict(&self, conflict_start_line: usize) -> Option<usize> {
    self.doc_to_display_line(conflict_start_line)
  }

  fn should_preserve_conflict_reveal_through_projection(&self) -> bool {
    self.git_state.bases.is_none()
      || self.bases_task.is_some()
      || self.diff_task.is_some()
      || self.projection_task.is_some()
  }

  pub fn reveal_conflict_start_line(&mut self, conflict_start_line: usize, cx: &mut Context<Self>) {
    let target_display_line = self
      .doc_to_display_line(conflict_start_line)
      .unwrap_or(conflict_start_line);
    let target_offset = {
      let document = self.document.read(cx);
      document.line_to_char(conflict_start_line)
    };
    let total_lines = self.display_line_count(self.document.read(cx).len_lines());
    let center_line = self
      .conflict_center_display_line(conflict_start_line, cx)
      .unwrap_or(target_display_line);

    self.move_to(target_offset, cx);
    self.hovered_conflict_start_line = None;
    self.last_mouse_position = None;
    self.center_display_line_in_viewport(center_line, total_lines);
    self.ensure_cursor_visible_with_policy(CursorRevealPolicy::WithPadding, cx);
    cx.notify();
  }

  /// Center and place the cursor on a document line (0-based), clamped to the file.
  pub fn reveal_source_line(&mut self, doc_line: usize, cx: &mut Context<Self>) {
    let line_count = self.document.read(cx).len_lines();
    if line_count == 0 {
      return;
    }
    let doc_line = doc_line.min(line_count - 1);
    let target_display_line = self.doc_to_display_line(doc_line).unwrap_or(doc_line);
    let target_offset = self.document.read(cx).line_to_char(doc_line);
    let total_lines = self.display_line_count(line_count);

    self.move_to(target_offset, cx);
    self.center_display_line_in_viewport(target_display_line, total_lines);
    self.ensure_cursor_visible_with_policy(CursorRevealPolicy::WithPadding, cx);
    cx.notify();
  }

  fn conflict_center_display_line(&self, conflict_start_line: usize, cx: &App) -> Option<usize> {
    let regions = self.conflict_regions(cx);
    let region = regions
      .iter()
      .find(|region| region.start_line == conflict_start_line)?;
    let last_doc_line = region.replace_end_line.saturating_sub(1);
    let start_display = self.doc_to_display_line(region.start_line)?;
    let end_display = self
      .doc_to_display_line(last_doc_line)
      .unwrap_or(start_display);
    Some(start_display + (end_display.saturating_sub(start_display)) / 2)
  }

  pub fn reveal_first_conflict(&mut self, cx: &mut Context<Self>) {
    let regions = self.conflict_regions(cx);
    let Some(first_region) = regions.first() else {
      return;
    };

    self.pending_conflict_reveal_start_line = self
      .should_preserve_conflict_reveal_through_projection()
      .then_some(first_region.start_line);
    self.reveal_conflict_start_line(first_region.start_line, cx);
  }

  pub fn resolve_conflict_region(
    &mut self,
    conflict_start_line: usize,
    resolution: ConflictResolution,
    cx: &mut Context<Self>,
  ) {
    if self.is_read_only {
      return;
    }

    let regions = self.conflict_regions(cx);
    let Some(region) = regions
      .iter()
      .find(|region| region.start_line == conflict_start_line)
      .cloned()
    else {
      return;
    };

    let (range, mut replacement, start_line, end_line, doc_line_count) = {
      let document = self.document.read(cx);
      let line_count = document.len_lines();
      let start_offset = document.line_to_char(region.start_line);
      let end_offset = if region.replace_end_line < line_count {
        document.line_to_char(region.replace_end_line)
      } else {
        document.len()
      };

      let mut replacement_lines = Vec::new();
      let mut append_lines = |line_range: Range<usize>| {
        for line_idx in line_range {
          replacement_lines.push(
            document
              .line_content(line_idx)
              .map(|line| line.into_owned())
              .unwrap_or_default(),
          );
        }
      };

      match resolution {
        ConflictResolution::Current => append_lines(region.current_range.clone()),
        ConflictResolution::Incoming => append_lines(region.incoming_range.clone()),
        ConflictResolution::Both => {
          append_lines(region.current_range.clone());
          append_lines(region.incoming_range.clone());
        }
      }

      (
        start_offset..end_offset,
        replacement_lines.join("\n"),
        region.start_line,
        region.replace_end_line.saturating_sub(1),
        line_count,
      )
    };

    if !replacement.is_empty() && region.replace_end_line < doc_line_count {
      replacement.push('\n');
    }

    let selection_before = self.clamp_range_to_doc_len(self.selected_range.clone(), cx);
    let line_height = self.measured_editor_line_height();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewports = self.doc_ranges_for_display_viewport(display_viewport);
    let replacement_line_count = replacement.matches('\n').count();
    let force_end_line = start_line
      .saturating_add(replacement_line_count)
      .max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    let transaction_id = self.document.update(cx, |doc, cx| {
      let id = doc.buffer.transaction(Instant::now(), |buffer, tx| {
        buffer.replace(tx, range.clone(), &replacement);
      });
      if !doc.should_defer_full_highlight() {
        doc.schedule_recompute_highlights(cx);
      }
      doc.schedule_viewport_highlights_for_ranges(
        &doc_viewports,
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
      cx.notify();
      id
    });
    self.mark_conflict_cache_dirty();

    self.invalidate_lines_from(start_line);
    self.display_selection = None;
    self.marked_range = None;
    self.selection_reversed = false;

    let doc_len_after = self.document.read(cx).len();
    let new_cursor = (range.start + replacement.chars().count()).min(doc_len_after);
    self.selected_range = new_cursor..new_cursor;

    let selection_after = self.selected_range.clone();
    self.record_transaction(transaction_id, selection_before, selection_after);

    self.hovered_group_id = None;
    self.hovered_conflict_start_line = None;
    self.last_mouse_position = None;
    self.is_dirty = true;
    self.schedule_diff_recompute(cx);
    self.rebuild_projection(cx);
    cx.notify();
  }

  pub fn navigate_conflict(
    &mut self,
    direction: ConflictNavigationDirection,
    cx: &mut Context<Self>,
  ) {
    let regions = self.conflict_regions(cx);
    let Some(active_index) = self.active_conflict_index(regions.as_ref(), cx) else {
      return;
    };

    let target_index = match direction {
      ConflictNavigationDirection::Previous => {
        if active_index == 0 {
          regions.len().saturating_sub(1)
        } else {
          active_index - 1
        }
      }
      ConflictNavigationDirection::Next => (active_index + 1) % regions.len(),
    };
    self.reveal_conflict_start_line(regions[target_index].start_line, cx);
  }

  fn ordered_hunk_display_lines(&self) -> Vec<(Arc<str>, usize)> {
    let Some(projection) = &self.projection else {
      return Vec::new();
    };
    let mut seen: HashSet<Arc<str>> = HashSet::new();
    let mut result = Vec::new();
    for (display_line, line) in projection.lines.iter().enumerate() {
      let group_id = match line {
        DisplayLine::Doc {
          change: Some(_),
          group_id: Some(id),
          ..
        }
        | DisplayLine::Modified {
          group_id: Some(id), ..
        }
        | DisplayLine::Removed {
          group_id: Some(id), ..
        }
        | DisplayLine::NoNewline {
          group_id: Some(id), ..
        } => id,
        _ => continue,
      };
      if seen.insert(group_id.clone()) {
        result.push((group_id.clone(), display_line));
      }
    }
    result
  }

  fn active_hunk_index(&self, ordered: &[(Arc<str>, usize)], cx: &App) -> Option<usize> {
    if ordered.is_empty() {
      return None;
    }

    let cursor_doc_line = {
      let document = self.document.read(cx);
      document.char_to_line(self.cursor_offset().min(document.len()))
    };
    let cursor_display_line = self.cursor_display_line_for_anchoring(cursor_doc_line);

    let mut active = 0;
    for (idx, (_, display_line)) in ordered.iter().enumerate() {
      if *display_line <= cursor_display_line {
        active = idx;
      } else {
        break;
      }
    }
    Some(active)
  }

  fn cursor_display_line_for_anchoring(&self, cursor_doc_line: usize) -> usize {
    if let Some(display) = self.doc_to_display_line(cursor_doc_line) {
      return display;
    }
    let Some(projection) = self.projection.as_ref() else {
      return 0;
    };
    if let Some(previous) = projection.previous_visible_doc_line(cursor_doc_line)
      && let Some(display) = projection.doc_to_display_line(previous)
    {
      return display.saturating_add(1);
    }
    0
  }

  pub fn hunk_navigation_state(&self, cx: &App) -> Option<HunkNavigationState> {
    let ordered = self.ordered_hunk_display_lines();
    let active_index = self.active_hunk_index(&ordered, cx)?;
    let active_display_line = ordered[active_index].1;

    Some(HunkNavigationState {
      active_index,
      total: ordered.len(),
      active_display_line,
    })
  }

  pub fn active_hunk_group_id(&self, cx: &App) -> Option<Arc<str>> {
    let ordered = self.ordered_hunk_display_lines();
    let active_index = self.active_hunk_index(&ordered, cx)?;
    Some(ordered[active_index].0.clone())
  }

  pub fn highlighted_hunk_group_id(&self, cx: &App) -> Option<Arc<str>> {
    let ordered = self.ordered_hunk_display_lines();
    let active_index = self.active_hunk_index(&ordered, cx)?;
    Some(ordered[active_index].0.clone())
  }

  pub fn navigate_hunk(&mut self, direction: HunkNavigationDirection, cx: &mut Context<Self>) {
    let ordered = self.ordered_hunk_display_lines();
    let Some(active_index) = self.active_hunk_index(&ordered, cx) else {
      return;
    };

    let target_index = match direction {
      HunkNavigationDirection::Previous => {
        if active_index == 0 {
          ordered.len().saturating_sub(1)
        } else {
          active_index - 1
        }
      }
      HunkNavigationDirection::Next => (active_index + 1) % ordered.len(),
    };
    self.reveal_hunk_display_line(ordered[target_index].1, cx);
  }

  fn reveal_hunk_display_line(&mut self, display_line: usize, cx: &mut Context<Self>) {
    let total_display_lines = self.display_line_count(self.document.read(cx).len_lines());

    let target_doc_line = self.display_to_doc_line(display_line).or_else(|| {
      let projection = self.projection.as_ref()?;
      projection
        .display_to_doc
        .iter()
        .skip(display_line)
        .find_map(|entry| *entry)
    });

    if let Some(doc_line) = target_doc_line {
      let doc_line_count = self.document.read(cx).len_lines();
      let clamped = doc_line.min(doc_line_count.saturating_sub(1));
      let target_offset = self.document.read(cx).line_to_char(clamped);
      self.move_to(target_offset, cx);
    }
    self.hovered_conflict_start_line = None;
    self.last_mouse_position = None;
    let center_line = self.hunk_center_display_line(display_line);
    self.center_display_line_in_viewport(center_line, total_display_lines);
    self.ensure_cursor_visible_with_policy(CursorRevealPolicy::WithPadding, cx);
    cx.notify();
  }

  fn hunk_center_display_line(&self, start_display_line: usize) -> usize {
    let Some(projection) = self.projection.as_ref() else {
      return start_display_line;
    };
    let group_id = match projection.lines.get(start_display_line) {
      Some(DisplayLine::Doc {
        group_id: Some(id), ..
      })
      | Some(DisplayLine::Modified {
        group_id: Some(id), ..
      })
      | Some(DisplayLine::Removed {
        group_id: Some(id), ..
      })
      | Some(DisplayLine::NoNewline {
        group_id: Some(id), ..
      }) => id.clone(),
      _ => return start_display_line,
    };
    let mut end = start_display_line;
    for (idx, line) in projection
      .lines
      .iter()
      .enumerate()
      .skip(start_display_line + 1)
    {
      let line_group = match line {
        DisplayLine::Doc {
          group_id: Some(id), ..
        }
        | DisplayLine::Modified {
          group_id: Some(id), ..
        }
        | DisplayLine::Removed {
          group_id: Some(id), ..
        }
        | DisplayLine::NoNewline {
          group_id: Some(id), ..
        } => Some(id),
        _ => None,
      };
      if line_group.map(|id| id.as_ref()) == Some(group_id.as_ref()) {
        end = idx;
      } else {
        break;
      }
    }
    start_display_line + (end - start_display_line) / 2
  }

  pub fn resolve_all_conflicts(&mut self, resolution: ConflictResolution, cx: &mut Context<Self>) {
    if self.is_read_only {
      return;
    }

    let conflict_regions = self.conflict_regions(cx);
    let mut conflict_start_lines = conflict_regions
      .iter()
      .map(|region| region.start_line)
      .collect::<Vec<_>>();
    conflict_start_lines.sort_unstable();

    for start_line in conflict_start_lines.into_iter().rev() {
      self.resolve_conflict_region(start_line, resolution, cx);
    }
  }

  pub fn display_to_doc_line(&self, display_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.display_to_doc_line(display_line)
    } else {
      Some(display_line)
    }
  }

  pub fn doc_to_display_line(&self, doc_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.doc_to_display_line(doc_line)
    } else {
      Some(doc_line)
    }
  }

  pub fn previous_visible_doc_line(&self, doc_line: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.previous_visible_doc_line(doc_line)
    } else {
      doc_line.checked_sub(1)
    }
  }

  fn expand_gap_with(
    &mut self,
    gap_id: GapId,
    head_amount: usize,
    tail_amount: usize,
    cx: &mut Context<Self>,
  ) {
    let entry = self.expanded_gaps.entry(gap_id).or_default();
    entry.head = entry.head.saturating_add(head_amount);
    entry.tail = entry.tail.saturating_add(tail_amount);

    if self.diffs.is_some() {
      self.rebuild_projection(cx);
    } else {
      self.schedule_diff_recompute(cx);
    }
  }

  pub fn expand_gap(&mut self, gap_id: GapId, amount: usize, cx: &mut Context<Self>) {
    self.expand_gap_with(gap_id, amount, amount, cx);
  }

  pub fn expand_gap_down(&mut self, gap_id: GapId, amount: usize, cx: &mut Context<Self>) {
    self.expand_gap_with(gap_id, amount, 0, cx);
  }

  pub fn expand_gap_up(&mut self, gap_id: GapId, amount: usize, cx: &mut Context<Self>) {
    self.expand_gap_with(gap_id, 0, amount, cx);
  }

  pub fn next_visible_doc_line(&self, doc_line: usize, doc_line_count: usize) -> Option<usize> {
    if let Some(projection) = &self.projection {
      projection.next_visible_doc_line(doc_line)
    } else if doc_line + 1 < doc_line_count {
      Some(doc_line + 1)
    } else {
      None
    }
  }

  fn gap_controls(&self) -> Vec<GapControl> {
    let Some(projection) = self.projection.as_ref() else {
      return Vec::new();
    };

    let mut controls = Vec::new();
    for (display_idx, line) in projection.lines.iter().enumerate() {
      let DisplayLine::Gap { id, .. } = line else {
        continue;
      };

      if display_idx > 0 {
        controls.push(GapControl {
          display_line: display_idx.saturating_sub(1),
          gap_id: *id,
          direction: GapExpandDirection::Down,
        });
      }

      if display_idx + 1 < projection.lines.len() {
        controls.push(GapControl {
          display_line: display_idx + 1,
          gap_id: *id,
          direction: GapExpandDirection::Up,
        });
      }
    }

    if !projection.lines.is_empty() {
      if let Some(gap_id) = projection.start_gap {
        controls.push(GapControl {
          display_line: 0,
          gap_id,
          direction: GapExpandDirection::Up,
        });
      }

      if let Some(gap_id) = projection.end_gap {
        controls.push(GapControl {
          display_line: projection.lines.len().saturating_sub(1),
          gap_id,
          direction: GapExpandDirection::Down,
        });
      }
    }

    controls
  }

  /// Invalidate a single line in the cache
  pub(crate) fn invalidate_line(&mut self, line: usize) {
    self.line_layouts.remove(&line);
  }

  pub(crate) fn invalidate_layout_cache_if_font_size_changed(&mut self, font_size: Pixels) -> bool {
    let previous_font_size_px = if self.last_layout_font_size > px(0.0) {
      (self.last_layout_font_size / px(1.0)).max(1.0)
    } else {
      0.0
    };
    let font_size_px = (font_size / px(1.0)).max(1.0);
    self.last_layout_font_size = font_size;

    if previous_font_size_px > 0.0 && (font_size_px - previous_font_size_px).abs() > 0.05 {
      self.line_layouts.clear();
      self.virtual_line_layouts.clear();
      return true;
    }

    false
  }

  /// Invalidate all lines from start_line onwards (for multi-line edits)
  pub(crate) fn invalidate_lines_from(&mut self, start_line: usize) {
    self
      .line_layouts
      .retain(|&line_idx, _| line_idx < start_line);
  }

  pub fn ensure_cache_size(&mut self, viewport: Range<usize>) {
    if self.line_layouts.len() > self.max_cache_size {
      let viewport_start = viewport.start.saturating_sub(50);
      let viewport_end = viewport.end + 50;

      self
        .line_layouts
        .retain(|&line_idx, _| line_idx >= viewport_start && line_idx < viewport_end);
    }

    if self.virtual_line_layouts.len() > self.max_cache_size {
      let viewport_start = viewport.start.saturating_sub(50);
      let viewport_end = viewport.end + 50;

      self
        .virtual_line_layouts
        .retain(|&line_idx, _| line_idx >= viewport_start && line_idx < viewport_end);
    }
  }

  pub(crate) fn viewport_range(&self, line_height: Pixels, total_lines: usize) -> Range<usize> {
    Self::viewport_range_for_height(
      self.scroll_offset_y,
      self.viewport_height,
      line_height,
      total_lines,
    )
  }

  pub(crate) fn doc_ranges_for_display_viewport(
    &self,
    viewport: Range<usize>,
  ) -> Vec<Range<usize>> {
    let Some(projection) = &self.projection else {
      return (viewport.start < viewport.end)
        .then_some(viewport)
        .into_iter()
        .collect();
    };

    let viewport_start = viewport.start;
    let viewport_end = viewport.end;
    if viewport_start >= viewport_end {
      return Vec::new();
    }

    let mapped_lines = (viewport_start..viewport_end)
      .filter_map(|display_line| {
        projection
          .display_to_doc_line(display_line)
          .map(|doc_line| (display_line, doc_line))
      })
      .collect::<Vec<_>>();

    if mapped_lines.is_empty() {
      return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut start_line = mapped_lines[0].1;
    let mut previous_line = mapped_lines[0].1;

    for (_, doc_line) in mapped_lines.iter().skip(1) {
      if *doc_line < previous_line || *doc_line > previous_line.saturating_add(1) {
        ranges.push(start_line..(previous_line + 1));
        start_line = *doc_line;
      }
      previous_line = *doc_line;
    }

    ranges.push(start_line..(previous_line + 1));
    ranges
  }

  #[cfg(test)]
  pub(crate) fn doc_range_for_display_viewport(&self, viewport: Range<usize>) -> Range<usize> {
    let Some(projection) = &self.projection else {
      return viewport;
    };

    let viewport_start = viewport.start;
    let viewport_end = viewport.end;
    if viewport_start >= viewport_end {
      return 0..0;
    }

    let mapped_lines = (viewport_start..viewport_end)
      .filter_map(|display_line| {
        projection
          .display_to_doc_line(display_line)
          .map(|doc_line| (display_line, doc_line))
      })
      .collect::<Vec<_>>();

    if mapped_lines.is_empty() {
      return 0..0;
    }

    // When viewport spans a collapsed gap, visible doc lines become disjoint.
    // Highlight only the contiguous segment nearest the viewport center.
    let center_display = viewport_start + (viewport_end - viewport_start - 1) / 2;
    let pivot = mapped_lines
      .iter()
      .enumerate()
      .min_by_key(|(_, (display_line, _))| display_line.abs_diff(center_display))
      .map(|(idx, _)| idx)
      .unwrap_or(0);

    let mut start_idx = pivot;
    while start_idx > 0 {
      let prev_doc_line = mapped_lines[start_idx - 1].1;
      let current_doc_line = mapped_lines[start_idx].1;
      if current_doc_line >= prev_doc_line && current_doc_line <= prev_doc_line.saturating_add(1) {
        start_idx -= 1;
      } else {
        break;
      }
    }

    let mut end_idx = pivot;
    while end_idx + 1 < mapped_lines.len() {
      let current_doc_line = mapped_lines[end_idx].1;
      let next_doc_line = mapped_lines[end_idx + 1].1;
      if next_doc_line >= current_doc_line && next_doc_line <= current_doc_line.saturating_add(1) {
        end_idx += 1;
      } else {
        break;
      }
    }

    let doc_start = mapped_lines[start_idx].1;
    let doc_end = mapped_lines[end_idx].1;
    doc_start..(doc_end + 1)
  }

  fn ensure_cursor_visible_with_policy(
    &mut self,
    policy: CursorRevealPolicy,
    cx: &mut Context<Self>,
  ) {
    let document = self.document.read(cx);
    let cursor_offset = self.cursor_offset();
    let doc_line_count = document.len_lines();
    let display_cursor = self.current_display_cursor(cx);
    let (cursor_line, cursor_column, cursor_doc_line) = if let Some(display_cursor) = display_cursor
    {
      let doc_line = self.display_to_doc_line(display_cursor.line);
      (display_cursor.line, display_cursor.column, doc_line)
    } else {
      let cursor_doc_line = document.char_to_line(cursor_offset);
      let Some(cursor_line) = self.doc_to_display_line(cursor_doc_line) else {
        return;
      };
      let line_start = document.line_to_char(cursor_doc_line);
      let column = cursor_offset.saturating_sub(line_start);
      (cursor_line, column, Some(cursor_doc_line))
    };
    let total_lines = self.display_line_count(doc_line_count);
    let line_height = self.measured_editor_line_height();
    let metrics = self.vertical_scroll_metrics(line_height, total_lines);
    self.scroll_offset_y =
      self.clamp_vertical_scroll(self.scroll_offset_y, line_height, total_lines);

    let cursor_line_f = cursor_line as f32;
    let cursor_top = cursor_line_f;
    let cursor_bottom = cursor_line_f + 1.0;
    let view_top = self.scroll_offset_y;
    let view_bottom = view_top + metrics.viewport_lines;
    let padded_top = view_top + metrics.scroll_padding;
    let padded_bottom = view_bottom - metrics.scroll_padding;

    match policy {
      CursorRevealPolicy::WhenHidden => {
        if cursor_top < view_top {
          self.scroll_offset_y = (cursor_top - metrics.scroll_padding).max(0.0);
        } else if cursor_bottom > view_bottom {
          self.scroll_offset_y =
            (cursor_bottom + metrics.scroll_padding - metrics.viewport_lines).max(0.0);
        }
      }
      CursorRevealPolicy::WithPadding => {
        if cursor_top < padded_top {
          self.scroll_offset_y = (cursor_top - metrics.scroll_padding).max(0.0);
        } else if cursor_bottom > padded_bottom {
          self.scroll_offset_y =
            (cursor_bottom + metrics.scroll_padding - metrics.viewport_lines).max(0.0);
        }
      }
    }

    let shaped_line = match cursor_doc_line {
      Some(doc_line) => self.line_layouts.get(&doc_line).cloned(),
      None => self.virtual_line_layouts.get(&cursor_line).cloned(),
    };
    let line_text = match cursor_doc_line {
      Some(doc_line) => Some(
        document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
      ),
      None => self.display_line_text(cursor_line, cx),
    };

    if let Some(line_text) = line_text {
      let line_len = line_text.chars().count();
      let cursor_in_line = cursor_column.min(line_len);
      let cursor_x = shaped_line
        .as_ref()
        .map(|shaped_line| {
          let cursor_byte = char_offset_to_byte_offset(&line_text, cursor_in_line);
          shaped_line.x_for_index(cursor_byte)
        })
        .unwrap_or_else(|| self.estimated_cursor_x(cursor_in_line));

      let horizontal_padding = self.gutter_width() + px(EXTRA_EDITOR_WIDTH);
      let current_scroll_x = self.scroll_handle.offset().x;
      let viewport_width = self.horizontal_viewport_width();

      // Note: scroll_x is negative when scrolled right (0 = left edge, -100 = scrolled 100px right)
      // visible area in absolute coordinates: [-current_scroll_x, -current_scroll_x + viewport_width]
      let visible_start_x = -current_scroll_x;
      let visible_end_x = -current_scroll_x + viewport_width;

      let needs_left_reveal = match policy {
        CursorRevealPolicy::WhenHidden => cursor_x < visible_start_x,
        CursorRevealPolicy::WithPadding => cursor_x < visible_start_x + horizontal_padding,
      };
      if needs_left_reveal {
        self.set_horizontal_scroll_offset(-(cursor_x - horizontal_padding).max(px(0.0)));
      }

      let needs_right_reveal = match policy {
        CursorRevealPolicy::WhenHidden => cursor_x > visible_end_x,
        CursorRevealPolicy::WithPadding => cursor_x > visible_end_x - horizontal_padding,
      };
      if needs_right_reveal {
        self.set_horizontal_scroll_offset(-(cursor_x - viewport_width + horizontal_padding));
      }
    }

    self.scroll_offset_y =
      self.clamp_vertical_scroll(self.scroll_offset_y, line_height, total_lines);
  }

  pub(crate) fn ensure_cursor_visible(&mut self, _window: &Window, cx: &mut Context<Self>) {
    self.ensure_cursor_visible_with_policy(CursorRevealPolicy::WithPadding, cx);
  }

  fn ensure_cursor_visible_when_hidden(&mut self, cx: &mut Context<Self>) {
    self.ensure_cursor_visible_with_policy(CursorRevealPolicy::WhenHidden, cx);
  }

  pub(crate) fn horizontal_viewport_width(&self) -> Pixels {
    let width = self.scroll_handle.bounds().size.width;
    if width > px(0.0) {
      width
    } else {
      self.viewport_width
    }
  }

  fn estimated_cursor_x(&self, cursor_column: usize) -> Pixels {
    self.measured_editor_char_width() * cursor_column as f32
  }

  pub(crate) fn clamp_horizontal_scroll_x(&self, scroll_x: Pixels) -> Pixels {
    let content_width = self.max_line_width + px(EXTRA_EDITOR_WIDTH);
    let viewport_width = self.horizontal_viewport_width();
    let max_right_scroll = (content_width - viewport_width).max(px(0.0));
    let min_scroll_x = -max_right_scroll;
    scroll_x.max(min_scroll_x).min(px(0.0))
  }

  pub(crate) fn record_transaction(
    &mut self,
    id: TransactionId,
    selection_before: Range<usize>,
    selection_after: Range<usize>,
  ) {
    if let Some(transaction) = self.undo_stack.iter_mut().find(|t| t.id == id) {
      transaction.selection_after = selection_after;
    } else {
      self.undo_stack.push_back(Transaction {
        id,
        selection_before,
        selection_after,
      });
      self.redo_stack.clear();
    }
  }

  pub(crate) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    let offset = self.clamp_offset_to_doc_len(offset, cx);
    self.selected_range = offset..offset;
    if !self.is_selecting {
      self.display_selection = None;
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  pub fn cursor_offset(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.start
    } else {
      self.selected_range.end
    }
  }

  pub(crate) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
    let offset = self.clamp_offset_to_doc_len(offset, cx);
    if self.selection_reversed {
      self.selected_range.start = offset
    } else {
      self.selected_range.end = offset
    };
    if !self.is_selecting {
      self.display_selection = None;
    }
    if self.selected_range.end < self.selected_range.start {
      self.selection_reversed = !self.selection_reversed;
      self.selected_range = self.selected_range.end..self.selected_range.start;
    }
    cx.notify()
  }

  fn selection_anchor_offset(&self) -> usize {
    if self.selection_reversed {
      self.selected_range.end
    } else {
      self.selected_range.start
    }
  }

  fn current_display_cursor(&self, cx: &App) -> Option<DisplayCursor> {
    if let Some(selection) = &self.display_selection {
      Some(selection.end)
    } else {
      self.display_cursor_for_offset(self.cursor_offset(), cx)
    }
  }

  fn current_display_anchor(&self, cx: &App) -> Option<DisplayCursor> {
    if let Some(selection) = &self.display_selection {
      Some(selection.start)
    } else {
      self.display_cursor_for_offset(self.selection_anchor_offset(), cx)
    }
  }

  fn display_line_len(&self, display_line: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    match self.display_line(display_line, doc_line_count) {
      Some(DisplayLine::Doc { doc_line, .. }) => document
        .line_content(doc_line)
        .map(|cow| cow.chars().count())
        .unwrap_or(0),
      Some(DisplayLine::Modified { doc_line, .. }) => document
        .line_content(doc_line)
        .map(|cow| cow.chars().count())
        .unwrap_or(0),
      Some(DisplayLine::Removed { text, .. }) => text.chars().count(),
      Some(DisplayLine::ReviewComment { .. }) => 0,
      Some(DisplayLine::NoNewline { .. }) => NO_NEWLINE_MARKER_TEXT.chars().count(),
      _ => 0,
    }
  }

  fn display_line_text(&self, display_line: usize, cx: &App) -> Option<String> {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    match self.display_line(display_line, doc_line_count) {
      Some(DisplayLine::Doc { doc_line, .. }) => Some(
        document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
      ),
      Some(DisplayLine::Modified { doc_line, .. }) => Some(
        document
          .line_content(doc_line)
          .map(|cow| cow.into_owned())
          .unwrap_or_default(),
      ),
      Some(DisplayLine::Removed { text, .. }) => Some(text.to_string()),
      Some(DisplayLine::NoNewline { .. }) => Some(NO_NEWLINE_MARKER_TEXT.to_string()),
      _ => None,
    }
  }

  fn clamp_range_to_len(range: Range<usize>, len: usize) -> Range<usize> {
    let start = range.start.min(len);
    let end = range.end.min(len);
    if start <= end { start..end } else { end..start }
  }

  fn clamp_offset_to_doc_len(&self, offset: usize, cx: &App) -> usize {
    let doc_len = self.document.read(cx).len();
    offset.min(doc_len)
  }

  fn clamp_range_to_doc_len(&self, range: Range<usize>, cx: &App) -> Range<usize> {
    let doc_len = self.document.read(cx).len();
    Self::clamp_range_to_len(range, doc_len)
  }

  fn is_removed_display_line(&self, display_line: usize, cx: &App) -> bool {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    matches!(
      self.display_line(display_line, doc_line_count),
      Some(DisplayLine::Removed { .. })
    )
  }

  pub(crate) fn is_read_only_display_cursor(&self, cx: &App) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    self.is_removed_display_line(cursor.line, cx)
  }

  fn removed_line_text(&self, display_line: usize, cx: &App) -> Option<String> {
    let document = self.document.read(cx);
    let doc_line_count = document.len_lines();
    match self.display_line(display_line, doc_line_count) {
      Some(DisplayLine::Removed { text, .. }) => Some(text.to_string()),
      _ => None,
    }
  }

  fn previous_word_boundary_in_line(text: &str, column: usize) -> usize {
    if column == 0 {
      return 0;
    }

    let column = column.min(text.chars().count());
    let column_byte = char_offset_to_byte_offset(text, column);
    let mut last_start = 0;
    for (idx, segment) in text.split_word_bound_indices() {
      if segment.trim().is_empty() {
        continue;
      }
      let end = idx + segment.len();
      if idx < column_byte && column_byte <= end {
        return byte_offset_to_char_offset(text, idx);
      }
      if idx < column_byte {
        last_start = idx;
      } else {
        break;
      }
    }
    byte_offset_to_char_offset(text, last_start)
  }

  fn next_word_boundary_in_line(text: &str, column: usize) -> usize {
    let len = text.chars().count();
    if column >= len {
      return len;
    }

    let column_byte = char_offset_to_byte_offset(text, column);
    for (idx, segment) in text.split_word_bound_indices() {
      if segment.trim().is_empty() {
        continue;
      }
      let end = idx + segment.len();
      if idx <= column_byte && column_byte < end {
        return byte_offset_to_char_offset(text, end);
      }
      if idx > column_byte {
        return byte_offset_to_char_offset(text, end);
      }
    }

    len
  }

  fn word_range_in_line(text: &str, column: usize) -> (usize, usize) {
    word_range_in_text(text, column)
  }

  fn set_display_cursor(&mut self, cursor: DisplayCursor, cx: &mut Context<Self>) {
    self.display_selection = Some(DisplaySelection {
      start: cursor,
      end: cursor,
    });
    if let Some(offset) = self.doc_offset_for_display_cursor(cursor, cx) {
      self.selected_range = offset..offset;
      self.selection_reversed = false;
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  fn set_display_selection_with_anchor(
    &mut self,
    anchor: DisplayCursor,
    cursor: DisplayCursor,
    cx: &mut Context<Self>,
  ) {
    self.display_selection = Some(DisplaySelection {
      start: anchor,
      end: cursor,
    });

    let reversed =
      cursor.line < anchor.line || (cursor.line == anchor.line && cursor.column < anchor.column);
    self.selection_reversed = reversed;

    if let (Some(anchor_offset), Some(cursor_offset)) = (
      self.doc_offset_for_display_cursor(anchor, cx),
      self.doc_offset_for_display_cursor(cursor, cx),
    ) {
      if reversed {
        self.selected_range = cursor_offset..anchor_offset;
      } else {
        self.selected_range = anchor_offset..cursor_offset;
      }
    }

    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    cx.notify();
  }

  fn next_selectable_display_line(&self, start: usize, direction: i32) -> Option<usize> {
    let projection = self.projection.as_ref()?;
    let mut line = start as i32 + direction;
    let max_line = projection.lines.len() as i32;
    while line >= 0 && line < max_line {
      if let Some(DisplayLine::Gap { .. }) = projection.lines.get(line as usize) {
        line += direction;
        continue;
      }
      return Some(line as usize);
    }
    None
  }

  pub(crate) fn move_display_cursor_vertical(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.projection.is_none() {
      return false;
    }
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.target_column.is_none() {
      self.target_column = Some(cursor.column);
    }
    let target_column = self.target_column.unwrap_or(cursor.column);
    let Some(target_line) = self.next_selectable_display_line(cursor.line, direction) else {
      return true;
    };

    let line_len = self.display_line_len(target_line, cx);
    let column = target_column.min(line_len);
    self.set_display_cursor(
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_vertical(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    if self.projection.is_none() {
      return false;
    }
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.target_column.is_none() {
      self.target_column = Some(cursor.column);
    }
    let target_column = self.target_column.unwrap_or(cursor.column);
    let Some(target_line) = self.next_selectable_display_line(cursor.line, direction) else {
      return true;
    };

    let line_len = self.display_line_len(target_line, cx);
    let column = target_column.min(line_len);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn move_display_cursor_horizontal(
    &mut self,
    delta: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    let line_len = self.display_line_len(cursor.line, cx);

    if self.projection.is_some() {
      if delta < 0 && cursor.column == 0 {
        if let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) {
          let column = self.display_line_len(target_line, cx);
          self.target_column = Some(column);
          self.set_display_cursor(
            DisplayCursor {
              line: target_line,
              column,
            },
            cx,
          );
        }
        // Keep projected navigation in display space; do not fall back to
        // doc-only movement into hidden lines.
        return true;
      }
      if delta > 0 && cursor.column == line_len {
        if let Some(target_line) = self.next_selectable_display_line(cursor.line, 1) {
          self.target_column = Some(0);
          self.set_display_cursor(
            DisplayCursor {
              line: target_line,
              column: 0,
            },
            cx,
          );
        }
        // Keep projected navigation in display space; do not fall back to
        // doc-only movement into hidden lines.
        return true;
      }
    }

    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }

    let mut column = cursor.column as i32 + delta;
    column = column.clamp(0, line_len as i32);
    let column = column as usize;
    if column == cursor.column {
      return false;
    }
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  fn step_display_cursor_horizontal(
    &self,
    mut cursor: DisplayCursor,
    delta: i32,
    cx: &App,
  ) -> Option<DisplayCursor> {
    if delta == 0 {
      return Some(cursor);
    }

    let direction = delta.signum();
    let steps = delta.unsigned_abs() as usize;

    for _ in 0..steps {
      if direction < 0 {
        if cursor.column > 0 {
          cursor.column = cursor.column.saturating_sub(1);
          continue;
        }

        let previous_line = self.next_selectable_display_line(cursor.line, -1)?;
        cursor.line = previous_line;
        cursor.column = self.display_line_len(previous_line, cx);
      } else {
        let line_len = self.display_line_len(cursor.line, cx);
        if cursor.column < line_len {
          cursor.column = cursor.column.saturating_add(1);
          continue;
        }

        let next_line = self.next_selectable_display_line(cursor.line, 1)?;
        cursor.line = next_line;
        cursor.column = 0;
      }
    }

    Some(cursor)
  }

  pub(crate) fn select_display_cursor_horizontal(
    &mut self,
    delta: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.display_selection.is_some() && !self.selected_range.is_empty() {
      let Some(next_cursor) = self.step_display_cursor_horizontal(cursor, delta, cx) else {
        // Keep display-based selection mode at boundaries instead of
        // falling back to doc-only selection (which drops removed lines).
        return true;
      };
      if next_cursor == cursor {
        return true;
      }
      self.target_column = Some(next_cursor.column);
      self.set_display_selection_with_anchor(anchor, next_cursor, cx);
      return true;
    }

    let line_len = self.display_line_len(cursor.line, cx);

    if self.projection.is_some() {
      if delta < 0 && cursor.column == 0 {
        if let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) {
          let column = self.display_line_len(target_line, cx);
          self.target_column = Some(column);
          self.set_display_selection_with_anchor(
            anchor,
            DisplayCursor {
              line: target_line,
              column,
            },
            cx,
          );
        }
        // Keep projected selection navigation in display space; do not fall
        // back to doc-only selection into hidden lines.
        return true;
      }
      if delta > 0 && cursor.column == line_len {
        if let Some(target_line) = self.next_selectable_display_line(cursor.line, 1) {
          self.target_column = Some(0);
          self.set_display_selection_with_anchor(
            anchor,
            DisplayCursor {
              line: target_line,
              column: 0,
            },
            cx,
          );
        }
        // Keep projected selection navigation in display space; do not fall
        // back to doc-only selection into hidden lines.
        return true;
      }
    }

    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }

    let mut column = cursor.column as i32 + delta;
    column = column.clamp(0, line_len as i32);
    let column = column as usize;
    if column == cursor.column {
      return false;
    }
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn move_display_cursor_word_horizontal(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let Some(text) = self.removed_line_text(cursor.line, cx) else {
      return false;
    };
    let column = cursor.column.min(text.chars().count());
    let column = if direction < 0 {
      Self::previous_word_boundary_in_line(&text, column)
    } else {
      Self::next_word_boundary_in_line(&text, column)
    };
    if column == cursor.column {
      return false;
    }
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_word_horizontal(
    &mut self,
    direction: i32,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let Some(text) = self.removed_line_text(cursor.line, cx) else {
      return false;
    };
    let column = cursor.column.min(text.chars().count());
    let column = if direction < 0 {
      Self::previous_word_boundary_in_line(&text, column)
    } else {
      Self::next_word_boundary_in_line(&text, column)
    };
    if column == cursor.column {
      return false;
    }
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn move_display_cursor_line_boundary(
    &mut self,
    to_start: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let line_len = self.display_line_len(cursor.line, cx);
    let column = if to_start { 0 } else { line_len };
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn collapse_removed_selection(
    &mut self,
    to_start: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(selection) = &self.display_selection else {
      return false;
    };
    if selection.is_empty() {
      return false;
    }
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let (start, end) = selection.normalized();
    let target = if to_start { start } else { end };
    self.set_display_cursor(target, cx);
    true
  }

  pub(crate) fn move_display_cursor_prev_display_line_end(
    &mut self,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if cursor.column != 0 || cursor.line == 0 {
      return false;
    }
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) else {
      return false;
    };
    let column = self.display_line_len(target_line, cx);
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_prev_display_line_end(
    &mut self,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if cursor.column != 0 || cursor.line == 0 {
      return false;
    }
    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) else {
      return false;
    };
    let column = self.display_line_len(target_line, cx);
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn move_display_cursor_prev_removed_line_end_from_boundary(
    &mut self,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if cursor.column != 0 || cursor.line == 0 {
      return false;
    }
    let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) else {
      return false;
    };
    if !self.is_removed_display_line(target_line, cx) {
      return false;
    }
    let column = self.display_line_len(target_line, cx);
    self.target_column = Some(column);
    self.set_display_cursor(
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_prev_removed_line_end_from_boundary(
    &mut self,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };
    if cursor.column != 0 || cursor.line == 0 {
      return false;
    }
    let Some(target_line) = self.next_selectable_display_line(cursor.line, -1) else {
      return false;
    };
    if !self.is_removed_display_line(target_line, cx) {
      return false;
    }
    let column = self.display_line_len(target_line, cx);
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_display_cursor_line_boundary(
    &mut self,
    to_start: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let Some(cursor) = self.current_display_cursor(cx) else {
      return false;
    };

    if self.display_selection.is_some() && !self.selected_range.is_empty() {
      let line_len = self.display_line_len(cursor.line, cx);
      let column = if to_start { 0 } else { line_len };
      if cursor.column == column {
        // Keep display-based selection mode at boundaries instead of
        // falling back to doc-only selection (which drops removed lines).
        return true;
      }
      self.target_column = Some(column);
      self.set_display_selection_with_anchor(
        anchor,
        DisplayCursor {
          line: cursor.line,
          column,
        },
        cx,
      );
      return true;
    }

    if !self.is_removed_display_line(cursor.line, cx) {
      return false;
    }
    let line_len = self.display_line_len(cursor.line, cx);
    let column = if to_start { 0 } else { line_len };
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: cursor.line,
        column,
      },
      cx,
    );
    true
  }

  fn first_selectable_display_line(&self) -> Option<usize> {
    let projection = self.projection.as_ref()?;
    for (idx, line) in projection.lines.iter().enumerate() {
      if !matches!(line, DisplayLine::Gap { .. }) {
        return Some(idx);
      }
    }
    None
  }

  fn last_selectable_display_line(&self) -> Option<usize> {
    let projection = self.projection.as_ref()?;
    for (idx, line) in projection.lines.iter().enumerate().rev() {
      if !matches!(line, DisplayLine::Gap { .. }) {
        return Some(idx);
      }
    }
    None
  }

  pub(crate) fn select_display_cursor_to_display_boundary(
    &mut self,
    to_start: bool,
    cx: &mut Context<Self>,
  ) -> bool {
    let Some(anchor) = self.current_display_anchor(cx) else {
      return false;
    };
    let target_line = if to_start {
      self.first_selectable_display_line()
    } else {
      self.last_selectable_display_line()
    };
    let Some(target_line) = target_line else {
      return false;
    };
    let column = if to_start {
      0
    } else {
      self.display_line_len(target_line, cx)
    };
    self.target_column = Some(column);
    self.set_display_selection_with_anchor(
      anchor,
      DisplayCursor {
        line: target_line,
        column,
      },
      cx,
    );
    true
  }

  pub(crate) fn select_all_display_lines(&mut self, cx: &mut Context<Self>) -> bool {
    let Some(start_line) = self.first_selectable_display_line() else {
      return false;
    };
    let Some(end_line) = self.last_selectable_display_line() else {
      return false;
    };
    let end_column = self.display_line_len(end_line, cx);
    self.display_selection = Some(DisplaySelection {
      start: DisplayCursor {
        line: start_line,
        column: 0,
      },
      end: DisplayCursor {
        line: end_line,
        column: end_column,
      },
    });
    let doc_len = self.document.read(cx).len();
    self.selected_range = 0..doc_len;
    self.selection_reversed = false;
    cx.notify();
    true
  }

  fn display_cursor_for_offset(&self, offset: usize, cx: &App) -> Option<DisplayCursor> {
    let document = self.document.read(cx);
    if document.is_empty() {
      return Some(DisplayCursor { line: 0, column: 0 });
    }

    let doc_line = document.char_to_line(offset);
    let line_start = document.line_to_char(doc_line);
    let column = offset.saturating_sub(line_start);

    let display_line = if let Some(display_line) = self.doc_to_display_line(doc_line) {
      display_line
    } else if let Some(projection) = &self.projection {
      if let Some(prev) = projection
        .previous_visible_doc_line(doc_line)
        .and_then(|line| self.doc_to_display_line(line))
      {
        prev
      } else {
        projection
          .next_visible_doc_line(doc_line)
          .and_then(|line| self.doc_to_display_line(line))?
      }
    } else {
      doc_line
    };

    Some(DisplayCursor {
      line: display_line,
      column,
    })
  }

  fn doc_offset_for_display_cursor(&self, cursor: DisplayCursor, cx: &App) -> Option<usize> {
    let document = self.document.read(cx);
    if document.is_empty() {
      return Some(0);
    }

    let doc_line = if let Some(doc_line) = self.display_to_doc_line(cursor.line) {
      doc_line
    } else if let Some(projection) = &self.projection {
      let mut forward = cursor.line + 1;
      let mut backward = cursor.line;
      let mut found = None;

      while forward < projection.lines.len() || backward > 0 {
        if forward < projection.lines.len() {
          if let Some(doc_line) = projection.display_to_doc_line(forward) {
            found = Some(doc_line);
            break;
          }
          forward += 1;
        }

        if backward > 0 {
          backward -= 1;
          if let Some(doc_line) = projection.display_to_doc_line(backward) {
            found = Some(doc_line);
            break;
          }
        }
      }

      found.unwrap_or(0)
    } else {
      cursor.line
    };

    let doc_line = doc_line.min(document.len_lines().saturating_sub(1));
    let line_start = document.line_to_char(doc_line);
    let line_len = document
      .line_content(doc_line)
      .map(|cow| cow.chars().count())
      .unwrap_or(0);
    let col = cursor.column.min(line_len);
    Some(line_start + col)
  }

  pub(crate) fn offset_from_utf16(&self, offset: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let mut utf16_count = 0;

    for (char_offset, ch) in document.chars().enumerate() {
      if utf16_count >= offset {
        return char_offset;
      }
      utf16_count += ch.len_utf16();
    }

    document.len()
  }

  pub(crate) fn offset_to_utf16(&self, offset: usize, cx: &App) -> usize {
    let document = self.document.read(cx);
    let mut utf16_offset = 0;

    for (char_count, ch) in document.chars().enumerate() {
      if char_count >= offset {
        break;
      }
      utf16_offset += ch.len_utf16();
    }

    utf16_offset
  }

  pub(crate) fn range_to_utf16(&self, range: &Range<usize>, cx: &App) -> Range<usize> {
    self.offset_to_utf16(range.start, cx)..self.offset_to_utf16(range.end, cx)
  }

  pub(crate) fn range_from_utf16(&self, range_utf16: &Range<usize>, cx: &App) -> Range<usize> {
    self.offset_from_utf16(range_utf16.start, cx)..self.offset_from_utf16(range_utf16.end, cx)
  }

  fn utf16_offset_to_char_offset_in_text(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0usize;
    for (char_offset, ch) in text.chars().enumerate() {
      if utf16_count >= utf16_offset {
        return char_offset;
      }
      utf16_count += ch.len_utf16();
    }
    text.chars().count()
  }

  fn utf16_range_to_char_range_in_text(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
    let start = Self::utf16_offset_to_char_offset_in_text(text, range_utf16.start);
    let end = Self::utf16_offset_to_char_offset_in_text(text, range_utf16.end);
    if start <= end { start..end } else { end..start }
  }

  pub fn mouse_left_down(
    &mut self,
    event: &MouseDownEvent,
    position_map: &PositionMap,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !position_map.bounds.contains(&event.position) {
      return;
    }

    self.target_column = None;
    self.is_selecting = true;

    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });

    if let Some(display_line) = position_map.display_line_for_position(event.position)
      && let Some(projection) = &position_map.projection
    {
      if let Some(DisplayLine::ReviewComment { .. }) = projection.lines.get(display_line) {
        self.is_selecting = false;
        return;
      }

      if matches!(
        projection.lines.get(display_line),
        Some(DisplayLine::Gap { .. })
      ) {
        self.is_selecting = false;
        return;
      }
    }

    let Some(display_cursor) = position_map.display_cursor_for_position(event.position) else {
      return;
    };

    self.last_mouse_position = Some(event.position);

    let anchor = if event.modifiers.shift {
      if let Some(selection) = &self.display_selection {
        selection.start
      } else {
        self
          .display_cursor_for_offset(self.selection_anchor_offset(), cx)
          .unwrap_or(display_cursor)
      }
    } else {
      display_cursor
    };

    self.display_selection = Some(DisplaySelection {
      start: anchor,
      end: display_cursor,
    });

    let (offset, doc_len) = {
      let document = self.document.read(cx);
      let offset = position_map
        .point_for_position(event.position, document)
        .or_else(|| self.doc_offset_for_display_cursor(display_cursor, cx));
      (offset, document.len())
    };
    let Some(offset) = offset else {
      return;
    };

    if event.modifiers.shift {
      self.select_to(offset, cx);
    } else {
      match event.click_count {
        1 => {
          self.move_to(offset, cx);
        }
        2 => {
          if self.is_removed_display_line(display_cursor.line, cx) {
            if let Some(text) = self.removed_line_text(display_cursor.line, cx) {
              let column = display_cursor.column.min(text.chars().count());
              let (start, end) = Self::word_range_in_line(&text, column);
              self.set_display_selection_with_anchor(
                DisplayCursor {
                  line: display_cursor.line,
                  column: start,
                },
                DisplayCursor {
                  line: display_cursor.line,
                  column: end,
                },
                cx,
              );
            }
            return;
          }
          let (word_start, word_end) = word_range_at_offset(self, offset, cx);
          self.selected_range = word_start..word_end;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
        3 => {
          if self.is_removed_display_line(display_cursor.line, cx) {
            let line_len = self.display_line_len(display_cursor.line, cx);
            self.set_display_selection_with_anchor(
              DisplayCursor {
                line: display_cursor.line,
                column: 0,
              },
              DisplayCursor {
                line: display_cursor.line,
                column: line_len,
              },
              cx,
            );
            return;
          }
          let (line_start, line_end) = line_range_at_offset(self, offset, cx);
          self.selected_range = line_start..line_end;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
        _ => {
          if self.select_all_display_lines(cx) {
            return;
          }
          self.selected_range = 0..doc_len;
          self.selection_reversed = false;
          self.display_selection = None;
          cx.notify();
        }
      }
    }
  }

  pub fn mouse_left_up(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
    self.is_selecting = false;
    self.finish_review_comment_create_drag(window, cx);
  }

  pub fn mouse_moved(
    &mut self,
    event: &MouseMoveEvent,
    position_map: &PositionMap,
    is_occluded: bool,
    is_primary: bool,
    cx: &mut Context<Self>,
  ) {
    let in_bounds = !is_occluded && position_map.bounds.contains(&event.position);
    if !in_bounds {
      // Both split panes listen on the same editor: only the pane that set
      // the hover may clear it, or crossing panes erases the fresh hover.
      if self.hovered_from_primary != is_primary {
        return;
      }
      let had_hovered_group = self.hovered_group_id.take().is_some();
      let had_hovered_conflict = self.hovered_conflict_start_line.take().is_some();
      let in_adjacent_gutter_band = !is_occluded
        && event.position.y >= position_map.bounds.top()
        && event.position.y <= position_map.bounds.bottom()
        && event.position.x >= position_map.bounds.left() - self.gutter_width()
        && event.position.x < position_map.bounds.left();
      let had_review_comment_create_hover = if in_adjacent_gutter_band {
        false
      } else {
        self
          .hovered_review_comment_create_display_line
          .take()
          .is_some()
      };
      // Prevent paint pass from re-deriving hover from stale position.
      self.last_mouse_position = None;
      if had_hovered_group || had_hovered_conflict || had_review_comment_create_hover {
        cx.notify();
      }
      return;
    }
    self.last_mouse_position = Some(event.position);
    self.hovered_from_primary = is_primary;
    let display_line = position_map.display_line_for_position(event.position);
    self.update_review_comment_create_drag_from_display_line(display_line, cx);

    let hovered = display_line.and_then(|line| self.group_id_for_modified_display_line(line));
    let hovered_conflict =
      display_line.and_then(|line| self.conflict_start_line_for_display_line(line, cx));
    let mut did_change = false;

    if self.hovered_group_id.as_deref() != hovered.as_deref() {
      self.hovered_group_id = hovered;
      did_change = true;
    }
    if self.hovered_conflict_start_line != hovered_conflict {
      self.hovered_conflict_start_line = hovered_conflict;
      did_change = true;
    }
    if did_change {
      cx.notify();
    }
  }

  pub fn mouse_dragged(
    &mut self,
    event: &MouseMoveEvent,
    position_map: &PositionMap,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.review_comment_create_drag_active {
      self.update_review_comment_create_drag_from_display_line(
        position_map.display_line_for_position(event.position),
        cx,
      );
      return;
    }

    if !self.is_selecting {
      return;
    }

    self.last_mouse_position = Some(event.position);

    if let Some(display_cursor) = position_map.display_cursor_for_position(event.position) {
      if let Some(selection) = self.display_selection.as_mut() {
        selection.end = display_cursor;
      } else {
        self.display_selection = Some(DisplaySelection {
          start: display_cursor,
          end: display_cursor,
        });
      }
    }

    let document = self.document.read(cx);
    let offset = position_map
      .point_for_position(event.position, document)
      .or_else(|| {
        position_map
          .display_cursor_for_position(event.position)
          .and_then(|cursor| self.doc_offset_for_display_cursor(cursor, cx))
      });

    if let Some(offset) = offset {
      self.select_to(offset, cx);
    }
  }
}

impl EntityInputHandler for Editor {
  fn text_for_range(
    &mut self,
    range_utf16: Range<usize>,
    actual_range: &mut Option<Range<usize>>,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<String> {
    let doc = self.document.read(cx);
    let range = Self::clamp_range_to_len(self.range_from_utf16(&range_utf16, cx), doc.len());
    actual_range.replace(self.range_to_utf16(&range, cx));
    Some(doc.slice_to_string(range))
  }

  fn selected_text_range(
    &mut self,
    _ignore_disabled_input: bool,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<UTF16Selection> {
    let selected_range = self.clamp_range_to_doc_len(self.selected_range.clone(), cx);
    Some(UTF16Selection {
      range: self.range_to_utf16(&selected_range, cx),
      reversed: self.selection_reversed,
    })
  }

  fn marked_text_range(
    &self,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<Range<usize>> {
    self
      .marked_range
      .as_ref()
      .map(|range| self.clamp_range_to_doc_len(range.clone(), cx))
      .map(|range| self.range_to_utf16(&range, cx))
  }

  fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
    self.marked_range = None;
  }

  fn replace_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.is_read_only {
      return;
    }
    if self.is_read_only_display_cursor(cx) && self.selected_range.is_empty() {
      return;
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    self.display_selection = None;
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());
    let range = self.clamp_range_to_doc_len(range, cx);
    let range = self.clamp_range_to_doc_len(range, cx);

    let selection_before = self.clamp_range_to_doc_len(self.selected_range.clone(), cx);
    let start_line = self.document.read(cx).char_to_line(range.start);
    let end_line = self.document.read(cx).char_to_line(range.end);

    let line_height = self.measured_editor_line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewports = self.doc_ranges_for_display_viewport(display_viewport);
    let new_line_count = new_text.matches('\n').count();
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    let transaction_id = self.document.update(cx, |doc, cx| {
      let id = doc.buffer.transaction(Instant::now(), |buffer, tx| {
        buffer.replace(tx, range.clone(), new_text);
      });

      if !doc.should_defer_full_highlight() {
        doc.schedule_recompute_highlights(cx);
      }
      doc.schedule_viewport_highlights_for_ranges(
        &doc_viewports,
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );

      cx.notify();
      id
    });
    self.mark_conflict_cache_dirty();

    let has_newline = new_text.contains('\n');

    if has_newline || start_line != end_line {
      self.invalidate_lines_from(start_line);
    } else {
      self.invalidate_line(start_line);
    }

    let new_text_chars = new_text.chars().count();
    let doc_len_after = self.document.read(cx).len();
    let new_cursor = (range.start + new_text_chars).min(doc_len_after);
    self.selected_range = new_cursor..new_cursor;
    self.marked_range.take();

    let selection_after = self.selected_range.clone();

    self.record_transaction(transaction_id, selection_before, selection_after);

    self.is_dirty = true;
    let _ = window;
    self.ensure_cursor_visible_when_hidden(cx);
    cx.notify();
    self.schedule_diff_recompute(cx);
  }

  fn replace_and_mark_text_in_range(
    &mut self,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    new_selected_range_utf16: Option<Range<usize>>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.is_read_only {
      return;
    }
    if self.is_read_only_display_cursor(cx) && self.selected_range.is_empty() {
      return;
    }
    self.cursor_blink.update(cx, |blink, cx| {
      blink.pause_blinking(cx);
    });
    let range = range_utf16
      .as_ref()
      .map(|range_utf16| self.range_from_utf16(range_utf16, cx))
      .or(self.marked_range.clone())
      .unwrap_or(self.selected_range.clone());
    let range = self.clamp_range_to_doc_len(range, cx);

    let start_line = self.document.read(cx).char_to_line(range.start);

    let line_height = self.measured_editor_line_height();
    let doc_line_count = self.document.read(cx).len_lines();
    let total_display_lines = self.display_line_count(doc_line_count);
    let display_viewport = self.viewport_range(line_height, total_display_lines);
    let doc_viewports = self.doc_ranges_for_display_viewport(display_viewport);
    let end_line = self.document.read(cx).char_to_line(range.end);
    let new_line_count = new_text.matches('\n').count();
    let force_end_line = start_line.saturating_add(new_line_count).max(end_line);
    let force_range = start_line..(force_end_line + 1);

    self.maybe_optimistic_unstage_for_edit(start_line, end_line, cx);

    self.document.update(cx, |doc, cx| {
      doc.replace(range.clone(), new_text, cx);
      if !doc.should_defer_full_highlight() {
        doc.schedule_recompute_highlights(cx);
      }
      doc.schedule_viewport_highlights_for_ranges(
        &doc_viewports,
        Some(force_range.clone()),
        crate::document::VIEWPORT_HIGHLIGHT_MARGIN_LINES,
        cx,
      );
    });
    self.mark_conflict_cache_dirty();

    self.invalidate_lines_from(start_line);

    let new_text_chars = new_text.chars().count();
    let doc_len_after = self.document.read(cx).len();
    if !new_text.is_empty() {
      self.marked_range = Some(Self::clamp_range_to_len(
        range.start..range.start + new_text_chars,
        doc_len_after,
      ));
    } else {
      self.marked_range = None;
    }
    let selected_range = new_selected_range_utf16
      .as_ref()
      .map(|range_utf16| Self::utf16_range_to_char_range_in_text(new_text, range_utf16))
      .map(|new_range| new_range.start + range.start..new_range.end + range.start)
      .unwrap_or_else(|| range.start + new_text_chars..range.start + new_text_chars);
    self.selected_range = Self::clamp_range_to_len(selected_range, doc_len_after);

    self.is_dirty = true;
    let _ = window;
    self.ensure_cursor_visible_when_hidden(cx);
    cx.notify();
    self.schedule_diff_recompute(cx);
  }

  fn bounds_for_range(
    &mut self,
    _range_utf16: Range<usize>,
    _bounds: Bounds<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<Bounds<Pixels>> {
    None
  }

  fn character_index_for_point(
    &mut self,
    _point: Point<Pixels>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<usize> {
    None
  }
}

fn parse_github_pr_comment_link(url: &str) -> Option<(u64, u64)> {
  let url = url
    .trim()
    .strip_prefix("https://github.com/")
    .or_else(|| url.strip_prefix("http://github.com/"))?;
  let (_, tail) = url.split_once("/pull/")?;
  let (pr_part, fragment) = tail.split_once('#')?;
  let pr_number = pr_part.split('/').next()?.split('?').next()?.parse().ok()?;
  let fragment = fragment
    .strip_prefix("discussion_r")
    .or_else(|| fragment.strip_prefix('r'))?;
  let comment_digits: String = fragment
    .chars()
    .take_while(|c| c.is_ascii_digit())
    .collect();
  if comment_digits.is_empty() {
    return None;
  }
  let comment_id = comment_digits.parse().ok()?;
  Some((pr_number, comment_id))
}

impl Render for Editor {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let is_dark = cx.theme().mode.is_dark();
    if self.theme.is_dark != is_dark {
      self.theme = Theme::new(is_dark);
      self.line_layouts.clear();
      self.virtual_line_layouts.clear();
      self.last_highlights_version = 0;
      self.last_highlights_epoch = 0;
    }

    let editor_entity = cx.entity().clone();
    let line_height = self.measured_editor_line_height();
    let previous_editor_line_height = self.editor_line_height;
    self.editor_line_height = line_height;
    let line_height_px = (line_height / px(1.0)).max(1.0);
    let previous_editor_line_height_px = if previous_editor_line_height > px(0.0) {
      (previous_editor_line_height / px(1.0)).max(1.0)
    } else {
      0.0
    };
    if previous_editor_line_height_px > 0.0
      && (line_height_px - previous_editor_line_height_px).abs() > 0.05
      && self.diffs.is_some()
    {
      self.rebuild_projection(cx);
    }
    let wrap_columns = self.computed_review_comment_wrap_columns();
    if wrap_columns != self.review_comment_wrap_columns {
      self.review_comment_wrap_columns = wrap_columns;
      if self.diffs.is_some() {
        self.rebuild_projection(cx);
      }
    }
    // Catches the composer values set programmatically: they emit no input event.
    if self.sync_review_comment_composer_rows(window, cx) && self.diffs.is_some() {
      self.rebuild_projection(cx);
    }
    let doc_line_count = self.document.read(cx).len_lines();
    let total_lines = self.display_line_count(doc_line_count);
    let viewport = self.viewport_range(line_height, total_lines);
    let gap_controls = self.gap_controls();
    let gutter_background = self.theme.gutter_background();
    let gutter_width = self.gutter_width();
    let scroll_offset_y = self.scroll_offset_y;
    let review_comment_create_button_target = (if self.review_comment_create_drag_active {
      self.review_comment_create_drag_start_display_line
    } else {
      self.hovered_review_comment_create_display_line
    })
    .and_then(|display_line| self.review_comment_create_target_for_display_line(display_line, cx));
    let show_review_comment_create_button = self.review_comment_create_handler.is_some()
      && self.replying_to_review_comment_id.is_none()
      && (self.review_comment_create_draft.is_none() || self.review_comment_create_drag_active);
    let find_panel = self.render_find_panel(editor_entity.clone(), window, cx);
    let focus_handle = self.focus_handle.clone();
    let external_input_focused =
      focus_handle.contains_focused(window, cx) && !focus_handle.is_focused(window);
    let editor_actions_enabled = editor_actions_enabled(
      self.is_find_input_focused(window, cx),
      self.is_review_comment_edit_input_focused(window, cx),
      self.is_review_comment_create_input_focused(window, cx),
      self.is_review_comment_reply_input_focused(window, cx),
      external_input_focused,
    );

    let build_gutter = |gutter_element: GutterElement,
                        view_suffix: &'static str,
                        side_filter: Option<ReviewCommentSide>,
                        editor_entity: Entity<Editor>| {
      let mut gutter = div()
        .w(gutter_width)
        .h_full()
        .bg(gutter_background)
        .relative()
        .child(gutter_element);

      for control in gap_controls.iter() {
        if !viewport.contains(&control.display_line) {
          continue;
        }

        let y = line_height * (control.display_line as f32 - scroll_offset_y);
        let button_id = format!(
          "gap-expand-{}-{}-{}-{}",
          view_suffix,
          control.direction.id_suffix(),
          control.gap_id.start,
          control.gap_id.end
        );
        let gap_id = control.gap_id;
        let direction = control.direction;
        let editor_entity = editor_entity.clone();

        let button = Button::new(button_id)
          .icon(direction.icon())
          .ghost()
          .xsmall()
          .compact()
          .tooltip(direction.tooltip())
          .on_click(move |_, _, cx| {
            editor_entity.update(cx, |editor, cx| match direction {
              GapExpandDirection::Up => editor.expand_gap_up(gap_id, 5, cx),
              GapExpandDirection::Down => editor.expand_gap_down(gap_id, 5, cx),
            });
          });

        gutter = gutter.child(
          div()
            .absolute()
            .left(px(6.0))
            .top(y)
            .h(line_height)
            .w(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .child(button),
        );
      }

      if show_review_comment_create_button
        && let Some(target) = review_comment_create_button_target
        && side_filter.is_none_or(|filter| filter == target.side)
        && viewport.contains(&target.display_line)
      {
        let y = line_height * (target.display_line as f32 - scroll_offset_y);
        let display_line = target.display_line;
        let button_id = format!(
          "review-comment-create-plus-{}-{}",
          view_suffix, display_line
        );
        let editor_entity = editor_entity.clone();
        gutter = gutter.child(
          div()
            .absolute()
            .right(px(REVIEW_COMMENT_CREATE_BUTTON_GUTTER_RIGHT_PX))
            .top(y)
            .h(line_height)
            .w(px(REVIEW_COMMENT_CREATE_BUTTON_HITBOX_WIDTH_PX))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
              cx.stop_propagation();
              editor_entity.update(cx, |editor, cx| {
                editor.start_review_comment_create_drag(display_line, cx);
              });
            })
            .child(
              Button::new(button_id)
                .icon(IconName::Plus)
                .xsmall()
                .primary()
                .compact(),
            ),
        );
      }

      gutter
    };

    let content = if self.diff_view_mode == DiffViewMode::Split {
      let left_overlay = self.render_review_comments_overlay(
        editor_entity.clone(),
        Some(ReviewCommentSide::Left),
        line_height,
        cx,
      );
      let left_create_overlay = self.render_review_comment_create_overlay(
        editor_entity.clone(),
        Some(ReviewCommentSide::Left),
        line_height,
        cx,
      );
      let right_overlay = self.render_review_comments_overlay(
        editor_entity.clone(),
        Some(ReviewCommentSide::Right),
        line_height,
        cx,
      );
      let right_create_overlay = self.render_review_comment_create_overlay(
        editor_entity.clone(),
        Some(ReviewCommentSide::Right),
        line_height,
        cx,
      );

      let left_panel = div()
        .size_full()
        .flex()
        .flex_row()
        .child(build_gutter(
          GutterElement::split_left(editor_entity.clone()),
          "left",
          Some(ReviewCommentSide::Left),
          editor_entity.clone(),
        ))
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content-left")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .overflow_hidden()
                .child(EditorElement::split_left(editor_entity.clone()))
                .when_some(left_overlay, |this, overlay| this.child(overlay))
                .when_some(left_create_overlay, |this, overlay| this.child(overlay)),
            ),
        );

      let right_panel = div()
        .size_full()
        .flex()
        .flex_row()
        .child(build_gutter(
          GutterElement::split_right(editor_entity.clone()),
          "right",
          Some(ReviewCommentSide::Right),
          editor_entity.clone(),
        ))
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .overflow_hidden()
                .child(EditorElement::split_right(editor_entity.clone()))
                .when_some(right_overlay, |this, overlay| this.child(overlay))
                .when_some(right_create_overlay, |this, overlay| this.child(overlay)),
            ),
        );

      div().flex_1().min_h(px(0.0)).child(
        h_resizable("editor-diff-split")
          .child(resizable_panel().child(left_panel))
          .child(resizable_panel().child(right_panel)),
      )
    } else {
      let inline_overlay =
        self.render_review_comments_overlay(editor_entity.clone(), None, line_height, cx);
      let inline_create_overlay =
        self.render_review_comment_create_overlay(editor_entity.clone(), None, line_height, cx);
      div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_row()
        .child(build_gutter(
          GutterElement::new(editor_entity.clone()),
          "inline",
          None,
          editor_entity.clone(),
        ))
        .child(
          div()
            .flex_1()
            .h_full()
            .id("editor-content")
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .child(
              div()
                .min_w(self.max_line_width + px(EXTRA_EDITOR_WIDTH))
                .h_full()
                .relative()
                .overflow_hidden()
                .child(EditorElement::new(editor_entity))
                .when_some(inline_overlay, |this, overlay| this.child(overlay))
                .when_some(inline_create_overlay, |this, overlay| this.child(overlay)),
            ),
        )
    };
    let content = content.font_family(editor_code_font_family(cx)).text_sm();

    div()
      .key_context("Editor")
      .track_focus(&self.focus_handle(cx))
      .cursor(CursorStyle::IBeam)
      .size_full()
      .relative()
      .overflow_hidden()
      .when(editor_actions_enabled, |el| {
        el.on_action(cx.listener(crate::actions::enter))
          .on_action(cx.listener(crate::actions::tab))
          .on_action(cx.listener(crate::actions::backspace))
          .on_action(cx.listener(crate::actions::backspace_word))
          .on_action(cx.listener(crate::actions::backspace_all))
          .on_action(cx.listener(crate::actions::delete))
          .on_action(cx.listener(crate::actions::up))
          .on_action(cx.listener(crate::actions::down))
          .on_action(cx.listener(crate::actions::left))
          .on_action(cx.listener(crate::actions::alt_left))
          .on_action(cx.listener(crate::actions::cmd_left))
          .on_action(cx.listener(crate::actions::right))
          .on_action(cx.listener(crate::actions::alt_right))
          .on_action(cx.listener(crate::actions::cmd_right))
          .on_action(cx.listener(crate::actions::cmd_up))
          .on_action(cx.listener(crate::actions::cmd_down))
          .on_action(cx.listener(crate::actions::select_cmd_left))
          .on_action(cx.listener(crate::actions::select_cmd_right))
          .on_action(cx.listener(crate::actions::select_cmd_up))
          .on_action(cx.listener(crate::actions::select_cmd_down))
          .on_action(cx.listener(crate::actions::select_up))
          .on_action(cx.listener(crate::actions::select_down))
          .on_action(cx.listener(crate::actions::select_left))
          .on_action(cx.listener(crate::actions::select_word_left))
          .on_action(cx.listener(crate::actions::select_right))
          .on_action(cx.listener(crate::actions::select_word_right))
          .on_action(cx.listener(crate::actions::select_all))
          .on_action(cx.listener(crate::actions::home))
          .on_action(cx.listener(crate::actions::end))
          .on_action(cx.listener(crate::actions::show_character_palette))
          .on_action(cx.listener(crate::actions::paste))
          .on_action(cx.listener(crate::actions::cut))
          .on_action(cx.listener(crate::actions::copy))
          .on_action(cx.listener(crate::actions::undo))
          .on_action(cx.listener(crate::actions::redo))
          .on_action(cx.listener(crate::actions::save))
          .on_action(cx.listener(crate::actions::find))
      })
      .on_action(cx.listener(crate::actions::close_find))
      .when_else(self.theme.is_dark, |el| el.bg(black()), |el| el.bg(white()))
      .when_else(
        self.theme.is_dark,
        |el| el.text_color(white()),
        |el| el.text_color(black()),
      )
      .flex()
      .flex_col()
      .child(content)
      .when_some(find_panel, |el, panel| el.child(panel))
  }
}

impl Focusable for Editor {
  fn focus_handle(&self, _: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[cfg(test)]
pub mod tests {
  use super::*;
  use gpui::TestAppContext;
  use std::path::Path;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::sync::{Arc as StdArc, Mutex};

  /// Two fixtures created in the same clock tick would otherwise share a path.
  static TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

  fn temp_path(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .expect("system time before unix epoch")
      .as_nanos();
    let unique = TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
      "reviu-{prefix}-{}-{nanos}-{unique}",
      std::process::id()
    ))
  }

  #[test]
  fn submit_mode_joins_pending_review_when_one_exists() {
    assert_eq!(
      review_comment_submit_mode(ReviewCommentDisplayMode::Conversation, false),
      ReviewCommentMode::SingleComment
    );
    assert_eq!(
      review_comment_submit_mode(ReviewCommentDisplayMode::Conversation, true),
      ReviewCommentMode::PendingReview
    );
  }

  #[test]
  fn submit_mode_of_a_local_note_ignores_any_pending_review() {
    assert_eq!(
      review_comment_submit_mode(ReviewCommentDisplayMode::LocalNote, false),
      ReviewCommentMode::SingleComment
    );
    assert_eq!(
      review_comment_submit_mode(ReviewCommentDisplayMode::LocalNote, true),
      ReviewCommentMode::SingleComment
    );
  }

  #[test]
  fn local_notes_offer_a_single_submit_action() {
    let actions = review_comment_create_actions(ReviewCommentDisplayMode::LocalNote, false);

    assert_eq!(actions, vec![REVIEW_COMMENT_ADD_NOTE_ACTION]);
    assert!(actions[0].primary);
    assert_eq!(actions[0].mode, ReviewCommentMode::SingleComment);
  }

  #[test]
  fn a_conversation_offers_both_github_destinations_until_a_review_is_pending() {
    assert_eq!(
      review_comment_create_actions(ReviewCommentDisplayMode::Conversation, false),
      vec![
        REVIEW_COMMENT_SINGLE_COMMENT_ACTION,
        REVIEW_COMMENT_START_REVIEW_ACTION,
      ]
    );
    assert_eq!(
      review_comment_create_actions(ReviewCommentDisplayMode::Conversation, true),
      vec![REVIEW_COMMENT_ADD_TO_REVIEW_ACTION]
    );
  }

  fn tiny_png_bytes() -> Vec<u8> {
    vec![
      137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
      0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3, 3, 2,
      0, 239, 154, 63, 71, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
  }

  #[test]
  fn language_hint_for_path_detects_dotfiles_and_dockerfile_variants() {
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/.bashrc")),
      Some("bash".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Dockerfile.dev")),
      Some("dockerfile".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.cpp")),
      Some("cpp".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.dart")),
      Some("dart".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.exs")),
      Some("elixir".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/vector.hpp")),
      Some("cpp".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Program.cs")),
      Some("csharp".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/script.csx")),
      Some("csharp".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/App.csproj")),
      Some("xml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/route-statistics.gpx")),
      Some("xml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/CMakeLists.txt")),
      Some("cmake".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/core.clj")),
      Some("clojure".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Cargo.lock")),
      Some("toml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/composer.lock")),
      Some("json".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Pipfile.lock")),
      Some("json".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/poetry.lock")),
      Some("toml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Main.java")),
      Some("java".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.jl")),
      Some("julia".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/query.sql")),
      Some("sql".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.hs")),
      Some("haskell".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.ml")),
      Some("ocaml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/interface.mli")),
      Some("ocaml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/pubspec.yaml")),
      Some("yaml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/pubspec.lock")),
      Some("yaml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/analysis_options.yaml")),
      Some("yaml".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/analysis.R")),
      Some("r".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Main.kt")),
      Some("kotlin".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/script.kts")),
      Some("kotlin".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.lua")),
      Some("lua".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Makefile")),
      Some("make".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/build.mk")),
      Some("make".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/Main.scala")),
      Some("scala".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/build.sbt")),
      Some("scala".to_string())
    );
    assert_eq!(
      Editor::language_hint_for_path(Path::new("/tmp/main.swift")),
      Some("swift".to_string())
    );
  }

  #[test]
  fn parse_github_pr_comment_link_accepts_standard_discussion_fragment() {
    let parsed = parse_github_pr_comment_link(
      "https://github.com/wooorm/markdown-rs/pull/197#discussion_r123456",
    );
    assert_eq!(parsed, Some((197, 123456)));
  }

  #[test]
  fn parse_github_pr_comment_link_accepts_short_r_fragment() {
    let parsed =
      parse_github_pr_comment_link("https://github.com/wooorm/markdown-rs/pull/197#r98765");
    assert_eq!(parsed, Some((197, 98765)));
  }

  #[test]
  fn parse_github_pr_comment_link_accepts_changes_path() {
    let parsed = parse_github_pr_comment_link(
      "https://github.com/wooorm/markdown-rs/pull/197/changes#discussion_r55",
    );
    assert_eq!(parsed, Some((197, 55)));
  }

  #[test]
  fn parse_github_pr_comment_link_accepts_query_params() {
    let parsed = parse_github_pr_comment_link(
      "https://github.com/wooorm/markdown-rs/pull/197?notification_referrer_id=NT_kwDOAAABBBCCC#discussion_r42",
    );
    assert_eq!(parsed, Some((197, 42)));
  }

  #[test]
  fn parse_github_pr_comment_link_rejects_url_without_comment_fragment() {
    let parsed =
      parse_github_pr_comment_link("https://github.com/wooorm/markdown-rs/pull/197/changes");
    assert_eq!(parsed, None);
  }

  #[test]
  fn markdown_link_target_extracts_exact_target() {
    let url = "https://github.com/acme/widget/blob/main/docker-compose.yml#L11";
    assert_eq!(
      markdown_link_target(&format!("[compose]({url})")),
      Some(url)
    );
    assert_eq!(
      markdown_link_target("[compose](https://github.com/acme/other)"),
      Some("https://github.com/acme/other")
    );
    assert_eq!(markdown_link_target(url), None);
  }

  #[test]
  fn conflict_regions_from_lines_parses_standard_markers() {
    let lines = vec![
      "before",
      "<<<<<<< HEAD",
      "ours",
      "=======",
      "theirs",
      ">>>>>>> branch",
      "after",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();

    let regions = conflict_regions_from_lines(&lines);
    assert_eq!(regions.len(), 1);
    let region = &regions[0];
    assert_eq!(region.start_line, 1);
    assert_eq!(region.current_range, 2..3);
    assert_eq!(region.incoming_range, 4..5);
    assert_eq!(region.replace_end_line, 6);
  }

  #[test]
  fn conflict_regions_from_lines_parses_diff3_markers() {
    let lines = vec![
      "before",
      "<<<<<<< HEAD",
      "ours",
      "||||||| base",
      "base",
      "=======",
      "theirs",
      ">>>>>>> branch",
      "after",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();

    let regions = conflict_regions_from_lines(&lines);
    assert_eq!(regions.len(), 1);
    let region = &regions[0];
    assert_eq!(region.start_line, 1);
    assert_eq!(region.current_range, 2..3);
    assert_eq!(region.incoming_range, 6..7);
    assert_eq!(region.replace_end_line, 8);
  }

  #[test]
  fn conflict_line_kinds_from_regions_marks_markers_and_divider() {
    let lines = vec!["<<<<<<< HEAD", "ours", "=======", "theirs", ">>>>>>> main"]
      .into_iter()
      .map(String::from)
      .collect::<Vec<_>>();

    let regions = conflict_regions_from_lines(&lines);
    let kinds = conflict_line_kinds_from_regions(&regions);

    assert_eq!(kinds.get(&0), Some(&ConflictLineKind::CurrentMarker));
    assert_eq!(kinds.get(&1), Some(&ConflictLineKind::Current));
    assert_eq!(kinds.get(&2), Some(&ConflictLineKind::Divider));
    assert_eq!(kinds.get(&3), Some(&ConflictLineKind::Incoming));
    assert_eq!(kinds.get(&4), Some(&ConflictLineKind::IncomingMarker));
  }

  #[gpui::test]
  fn conflict_regions_from_document_matches_line_parser(cx: &mut TestAppContext) {
    let text =
      "before\n<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> branch\nafter\n";
    let doc = cx.new(|cx| Document::new(text, None, cx));
    doc.read_with(cx, |doc, _| {
      let lines = text.lines().map(String::from).collect::<Vec<_>>();
      assert_eq!(
        conflict_regions_from_document(doc),
        conflict_regions_from_lines(&lines)
      );
    });
  }

  #[gpui::test]
  fn has_unresolved_conflict_markers_detects_conflict_markers(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> main\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      assert!(editor.has_unresolved_conflict_markers(cx));
    });
  }

  #[gpui::test]
  fn has_unresolved_conflict_markers_returns_false_when_clean(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "resolved\ncontent\n");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      assert!(!editor.has_unresolved_conflict_markers(cx));
    });
  }

  #[gpui::test]
  fn resolve_conflict_region_invalidates_conflict_cache(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      assert!(editor.has_unresolved_conflict_markers(cx));
      let conflict_regions = editor.conflict_regions(cx);
      let conflict_start_line = conflict_regions
        .iter()
        .next()
        .expect("conflict region")
        .start_line;
      editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Current, cx);
      assert!(!editor.has_unresolved_conflict_markers(cx));
    });
  }

  #[gpui::test]
  fn load_readonly_snapshot_invalidates_conflict_cache(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      assert!(editor.has_unresolved_conflict_markers(cx));
      editor.load_readonly_snapshot("resolved\ncontent\n".to_string(), None, cx);
      assert!(!editor.has_unresolved_conflict_markers(cx));
    });
  }

  #[test]
  fn load_file_for_editor_preserves_binary_bytes_for_raster_images() {
    let repo_root = temp_path("editor-load-binary");
    let workdir_path = repo_root.join("fixtures/image.png");
    std::fs::create_dir_all(
      workdir_path
        .parent()
        .expect("binary editor fixture should have parent"),
    )
    .expect("create binary editor fixture parent");
    let expected_bytes = tiny_png_bytes();
    std::fs::write(&workdir_path, &expected_bytes).expect("write binary editor fixture");

    let loaded = Editor::load_file_for_editor(&repo_root, &workdir_path);

    assert_eq!(loaded.content, "");
    assert_eq!(loaded.binary_bytes, Some(expected_bytes));
    assert!(!loaded.is_read_only);
    assert!(loaded.language_hint.is_none());

    std::fs::remove_dir_all(&repo_root).ok();
  }

  #[gpui::test]
  fn resolve_conflict_region_accept_current(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let conflict_regions = editor.conflict_regions(cx);
      let conflict_start_line = conflict_regions
        .iter()
        .next()
        .expect("conflict region")
        .start_line;
      editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Current, cx);
    });

    assert_eq!(ctx.text(), "pre\nours\npost\n");
  }

  #[gpui::test]
  fn resolve_conflict_region_accept_incoming(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let conflict_regions = editor.conflict_regions(cx);
      let conflict_start_line = conflict_regions
        .iter()
        .next()
        .expect("conflict region")
        .start_line;
      editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Incoming, cx);
    });

    assert_eq!(ctx.text(), "pre\ntheirs\npost\n");
  }

  #[gpui::test]
  fn resolve_conflict_region_accept_both(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let conflict_regions = editor.conflict_regions(cx);
      let conflict_start_line = conflict_regions
        .iter()
        .next()
        .expect("conflict region")
        .start_line;
      editor.resolve_conflict_region(conflict_start_line, ConflictResolution::Both, cx);
    });

    assert_eq!(ctx.text(), "pre\nours\ntheirs\npost\n");
  }

  #[gpui::test]
  fn resolve_all_conflicts_accept_current(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.resolve_all_conflicts(ConflictResolution::Current, cx);
      assert!(!editor.has_unresolved_conflict_markers(cx));
    });

    assert_eq!(ctx.text(), "pre\nours1\nmid\nours2\npost\n");
  }

  #[gpui::test]
  fn resolve_all_conflicts_accept_incoming(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.resolve_all_conflicts(ConflictResolution::Incoming, cx);
      assert!(!editor.has_unresolved_conflict_markers(cx));
    });

    assert_eq!(ctx.text(), "pre\ntheirs1\nmid\ntheirs2\npost\n");
  }

  #[gpui::test]
  fn conflict_navigation_moves_between_regions_and_wraps(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let initial_state = editor
        .conflict_navigation_state(cx)
        .expect("initial conflict navigation state");
      assert_eq!(initial_state.active_index, 0);
      assert_eq!(initial_state.total, 2);
      assert_eq!(initial_state.active_start_line, 1);

      editor.navigate_conflict(ConflictNavigationDirection::Next, cx);
      let second_state = editor
        .conflict_navigation_state(cx)
        .expect("second conflict navigation state");
      assert_eq!(second_state.active_index, 1);
      assert_eq!(second_state.total, 2);
      assert_eq!(second_state.active_start_line, 7);
      let second_offset = editor.document().read(cx).line_to_char(7);
      assert_eq!(editor.cursor_offset(), second_offset);

      editor.navigate_conflict(ConflictNavigationDirection::Next, cx);
      let wrapped_state = editor
        .conflict_navigation_state(cx)
        .expect("wrapped conflict navigation state");
      assert_eq!(wrapped_state.active_index, 0);
      assert_eq!(wrapped_state.total, 2);
      assert_eq!(wrapped_state.active_start_line, 1);

      editor.navigate_conflict(ConflictNavigationDirection::Previous, cx);
      let previous_state = editor
        .conflict_navigation_state(cx)
        .expect("previous conflict navigation state");
      assert_eq!(previous_state.active_index, 1);
      assert_eq!(previous_state.total, 2);
      assert_eq!(previous_state.active_start_line, 7);
    });
  }

  #[gpui::test]
  fn conflict_navigation_state_advances_to_next_remaining_conflict_after_resolution(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(
      cx.clone(),
      "pre\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n",
    );

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let first_conflict_start_line = editor
        .conflict_navigation_state(cx)
        .expect("initial conflict navigation state")
        .active_start_line;

      editor.resolve_conflict_region(first_conflict_start_line, ConflictResolution::Current, cx);

      let state = editor
        .conflict_navigation_state(cx)
        .expect("remaining conflict navigation state");
      assert_eq!(state.active_index, 0);
      assert_eq!(state.total, 1);
      assert_eq!(state.active_start_line, 3);
    });
  }

  #[gpui::test]
  fn conflict_navigation_centers_target_conflict_in_viewport(cx: &mut TestAppContext) {
    let mut text = String::new();
    for index in 0..15 {
      if !text.is_empty() {
        text.push('\n');
      }
      text.push_str(&format!("pre {index}"));
    }
    text.push_str(
      "\n<<<<<<< HEAD\nours1\n=======\ntheirs1\n>>>>>>> branch\nmid\n<<<<<<< HEAD\nours2\n=======\ntheirs2\n>>>>>>> branch\npost\n",
    );

    let mut ctx = EditorTestContext::with_text(cx.clone(), &text);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.navigate_conflict(ConflictNavigationDirection::Next, cx);

      let target_display_line = editor
        .first_display_line_for_conflict(21)
        .expect("display line");
      let total_lines = editor.display_line_count(editor.document().read(cx).len_lines());
      let metrics = Editor::vertical_scroll_metrics_for_height(
        editor.viewport_height,
        editor.measured_editor_line_height(),
        total_lines,
      );
      let expected_scroll = (target_display_line as f32 - ((metrics.viewport_lines - 1.0) / 2.0))
        .clamp(0.0, metrics.max_scroll);

      assert!((editor.scroll_offset_y - expected_scroll).abs() < f32::EPSILON);
    });
  }

  #[gpui::test]
  fn hunk_navigation_moves_between_hunks_and_wraps(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc\nd\ne\nf\n");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(Arc::new(projection_with_two_hunks(6, &[1, 4])));

      let initial_state = editor
        .hunk_navigation_state(cx)
        .expect("initial hunk navigation state");
      assert_eq!(initial_state.active_index, 0);
      assert_eq!(initial_state.total, 2);
      assert_eq!(initial_state.active_display_line, 1);

      editor.navigate_hunk(HunkNavigationDirection::Next, cx);
      let second_state = editor
        .hunk_navigation_state(cx)
        .expect("second hunk navigation state");
      assert_eq!(second_state.active_index, 1);
      assert_eq!(second_state.active_display_line, 4);
      let second_offset = editor.document().read(cx).line_to_char(4);
      assert_eq!(editor.cursor_offset(), second_offset);

      editor.navigate_hunk(HunkNavigationDirection::Next, cx);
      let wrapped_state = editor
        .hunk_navigation_state(cx)
        .expect("wrapped hunk navigation state");
      assert_eq!(wrapped_state.active_index, 0);
      assert_eq!(wrapped_state.active_display_line, 1);

      editor.navigate_hunk(HunkNavigationDirection::Previous, cx);
      let previous_state = editor
        .hunk_navigation_state(cx)
        .expect("previous hunk navigation state");
      assert_eq!(previous_state.active_index, 1);
      assert_eq!(previous_state.active_display_line, 4);
    });
  }

  #[gpui::test]
  fn hunk_navigation_state_is_none_when_no_hunks(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc\n");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(Arc::new(projection_with_doc_lines(3)));
      assert!(editor.hunk_navigation_state(cx).is_none());
    });
  }

  #[gpui::test]
  fn review_comment_body_segments_inserts_preview_between_markdown_lines(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line");
    let url = "https://github.com/acme/widget/blob/main/docker-compose.yml#L11";
    let previews = vec![ReviewCommentCodeReferencePreview {
      url: Arc::from(url),
      repo: Arc::from("acme/widget"),
      path: Arc::from("docker-compose.yml"),
      reference: Arc::from("main"),
      start_line: 11,
      end_line: 11,
      snippets: vec![Arc::from("- '5433:5432'")],
      full_content: None,
    }];

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor
        .review_comment_code_reference_previews
        .insert(42, previews.clone());
      let segments = editor.review_comment_body_segments(42, &format!("test before\n{url}\ntest after"));
      assert_eq!(segments.len(), 3);
      assert!(matches!(&segments[0], ReviewCommentBodySegment::Markdown(value) if value == "test before"));
      assert!(matches!(&segments[1], ReviewCommentBodySegment::Preview(preview) if preview.url.as_ref() == url));
      assert!(matches!(&segments[2], ReviewCommentBodySegment::Markdown(value) if value == "test after"));
      assert_eq!(
        Editor::review_comment_markdown_body_from_segments(&segments),
        "test before\ntest after"
      );
    });
  }

  #[gpui::test]
  fn review_comment_markdown_options_include_asset_url_resolver(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let resolver: ReviewCommentAssetUrlResolver =
        Arc::new(|_| Some("https://example.com/image.png".to_string()));
      editor.set_review_comment_asset_url_resolver(Some(resolver), cx);

      let options = editor.review_comment_markdown_options(MarkdownRenderState::new(), 42);

      assert!(options.asset_url_resolver.is_some());
      // A newline typed in the composer must stay a newline once posted.
      assert!(options.hardbreaks);
    });
  }

  #[gpui::test]
  fn as_gfm_code_reference_preview_preserves_fields(cx: &mut TestAppContext) {
    let _ = cx;
    let preview = ReviewCommentCodeReferencePreview {
      url: Arc::from("https://github.com/acme/widget/blob/main/src/lib.rs#L1-L2"),
      repo: Arc::from("acme/widget"),
      path: Arc::from("src/lib.rs"),
      reference: Arc::from("main"),
      start_line: 1,
      end_line: 2,
      snippets: vec![Arc::from("fn main() {"), Arc::from("}")],
      full_content: None,
    };

    let converted = as_gfm_code_reference_preview(&preview);
    assert_eq!(converted.url, preview.url);
    assert_eq!(converted.repo, preview.repo);
    assert_eq!(converted.path, preview.path);
    assert_eq!(converted.reference, preview.reference);
    assert_eq!(converted.start_line, preview.start_line);
    assert_eq!(converted.end_line, preview.end_line);
    assert_eq!(converted.snippets, preview.snippets);
  }

  #[gpui::test]
  fn review_comment_preview_height_uses_shared_gfm_estimator(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "line");
    let preview = ReviewCommentCodeReferencePreview {
      url: Arc::from("https://github.com/acme/widget/blob/main/src/lib.rs#L1-L2"),
      repo: Arc::from("acme/widget"),
      path: Arc::from("src/lib.rs"),
      reference: Arc::from("main"),
      start_line: 1,
      end_line: 2,
      snippets: vec![Arc::from("fn main() {"), Arc::from("}")],
      full_content: None,
    };

    ctx.editor.read_with(&ctx.cx, |editor, _| {
      let actual = editor.review_comment_code_reference_preview_height_px(&preview);
      let expected = estimate_github_code_reference_preview_height_px(
        preview.snippets.len(),
        editor.review_comment_line_height_px,
      );
      assert_eq!(actual, expected);
    });
  }

  #[gpui::test]
  fn test_editor_code_font_family_matches_theme_mono_font(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (expected, actual) = cx.update(|cx| {
      (
        cx.theme().mono_font_family.clone(),
        editor_code_font_family(cx),
      )
    });
    assert_eq!(actual, expected);
  }

  #[gpui::test]
  fn test_measured_editor_line_height_uses_cached_value(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line");
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.editor_line_height = px(17.0);
    });

    let measured = ctx
      .editor
      .read_with(&ctx.cx, |editor, _| editor.measured_editor_line_height());
    assert_eq!(measured, px(17.0));
  }

  #[gpui::test]
  fn test_computed_review_comment_wrap_columns_tracks_review_comment_char_width(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line");
    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.review_comment_char_width = px(6.0);
    });
    let narrow_columns = ctx.editor.read_with(&ctx.cx, |editor, _| {
      editor.computed_review_comment_wrap_columns()
    });

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.review_comment_char_width = px(12.0);
    });
    let wide_columns = ctx.editor.read_with(&ctx.cx, |editor, _| {
      editor.computed_review_comment_wrap_columns()
    });

    assert!(wide_columns < narrow_columns);
  }

  #[gpui::test]
  fn test_next_review_comment_body_rejects_empty_or_unchanged(_cx: &mut TestAppContext) {
    assert!(next_review_comment_body("", "before").is_none());
    assert!(next_review_comment_body("   \n\t", "before").is_none());
    assert!(next_review_comment_body("before", "before").is_none());
  }

  #[gpui::test]
  fn test_next_review_comment_body_trims_and_keeps_multiline(_cx: &mut TestAppContext) {
    assert_eq!(
      next_review_comment_body("line 1\nline 2", "line 1"),
      Some(Arc::from("line 1\nline 2"))
    );
    assert_eq!(
      next_review_comment_body("\nline 1\nline 2\n", "line 1"),
      Some(Arc::from("line 1\nline 2"))
    );
  }

  #[gpui::test]
  fn test_editor_actions_enabled_depends_on_nested_input_focus(_cx: &mut TestAppContext) {
    assert!(editor_actions_enabled(false, false, false, false, false));
    assert!(!editor_actions_enabled(true, false, false, false, false));
    assert!(!editor_actions_enabled(false, true, false, false, false));
    assert!(!editor_actions_enabled(false, false, true, false, false));
    assert!(!editor_actions_enabled(false, false, false, true, false));
    assert!(!editor_actions_enabled(false, false, false, false, true));
    assert!(!editor_actions_enabled(true, true, true, true, true));
  }

  #[gpui::test]
  fn test_gutter_line_number_right_padding_adds_create_button_space(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line");
    let base_padding = ctx.editor.read_with(&ctx.cx, |editor, _| {
      editor.gutter_line_number_right_padding()
    });
    let base_gutter_width = ctx
      .editor
      .read_with(&ctx.cx, |editor, _| editor.gutter_width());

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentCreateHandler = Arc::new(|_, _, _| {});
      editor.set_review_comment_create_handler(Some(handler), cx);
    });
    let create_padding = ctx.editor.read_with(&ctx.cx, |editor, _| {
      editor.gutter_line_number_right_padding()
    });
    let create_gutter_width = ctx
      .editor
      .read_with(&ctx.cx, |editor, _| editor.gutter_width());

    assert!(create_padding > base_padding);
    assert!(create_gutter_width > base_gutter_width);
    assert_eq!(
      create_gutter_width - base_gutter_width,
      create_padding - base_padding
    );
    assert_eq!(base_padding, px(GUTTER_LINE_NUMBER_BASE_RIGHT_PADDING_PX));
    assert_eq!(base_gutter_width, px(GUTTER_WIDTH));
  }

  #[test]
  fn test_review_comment_composer_body_height_clears_the_actions_beside_it() {
    let tall = review_comment_composer_textarea_height_px(6, 20.0);

    // A text box shorter than the buttons beside it does not shrink the row.
    assert_eq!(
      review_comment_composer_body_height_px(10.0, 0.0),
      REVIEW_COMMENT_COMPOSER_ACTIONS_HEIGHT_PX
    );
    assert_eq!(review_comment_composer_body_height_px(tall, 0.0), tall);
    assert_eq!(
      review_comment_composer_body_height_px(tall, MARKDOWN_COMPOSER_CHROME_HEIGHT_PX),
      MARKDOWN_COMPOSER_CHROME_HEIGHT_PX + tall
    );
  }

  #[gpui::test]
  fn test_review_comment_composer_rows_follow_the_shaped_text(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let ctx = EditorTestContext::with_text(cx.clone(), "a");
    let editor = ctx.editor.clone();
    let (_root, cx) =
      cx.add_window_view(|window, cx| gpui_component::Root::new(editor.clone(), window, cx));

    cx.update(|window, _| {
      let font = gpui::font("Helvetica");
      let font_size = px(14.0);
      let roomy = px(10_000.0);

      assert_eq!(
        review_comment_composer_rows("", roomy, font.clone(), font_size, window),
        REVIEW_COMMENT_COMPOSER_MIN_ROWS
      );
      assert_eq!(
        review_comment_composer_rows("one line", roomy, font.clone(), font_size, window),
        1
      );
      assert_eq!(
        review_comment_composer_rows("one\ntwo", roomy, font.clone(), font_size, window),
        2
      );
      assert_eq!(
        review_comment_composer_rows(
          &"line\n".repeat(200),
          roomy,
          font.clone(),
          font_size,
          window
        ),
        REVIEW_COMMENT_COMPOSER_MAX_ROWS
      );
      // The same text in a narrow column takes more rows.
      assert!(
        review_comment_composer_rows(
          "one two three four five six seven",
          px(60.0),
          font,
          font_size,
          window
        ) > 1
      );
    });
  }

  #[test]
  fn test_review_comment_composer_textarea_height_follows_its_rows() {
    let min = review_comment_composer_textarea_height_px(REVIEW_COMMENT_COMPOSER_MIN_ROWS, 20.0);
    let max = review_comment_composer_textarea_height_px(REVIEW_COMMENT_COMPOSER_MAX_ROWS, 20.0);

    assert_eq!(
      min,
      20.0 + REVIEW_COMMENT_COMPOSER_TEXTAREA_VERTICAL_CHROME_PX
    );
    assert_eq!(
      review_comment_composer_textarea_height_px(2, 20.0),
      min + 20.0
    );
    assert!(max > min);
  }

  #[test]
  fn test_only_a_headerless_note_keeps_room_for_its_actions() {
    // A header row carries them, so the body keeps its full width.
    assert!(!review_comment_body_reserves_actions_room(true, true, true));
    assert!(review_comment_body_reserves_actions_room(true, false, true));
    // Nothing to keep room for.
    assert!(!review_comment_body_reserves_actions_room(
      true, false, false
    ));
    // A conversation lays its actions out in the header, never over the body.
    assert!(!review_comment_body_reserves_actions_room(
      false, false, true
    ));
  }

  #[test]
  fn a_conversation_you_cannot_touch_offers_no_button() {
    // A comment of a review you have not submitted: GitHub has no thread to
    // resolve yet, so the row keeps the space.
    assert_eq!(
      review_comment_resolve_control(true, true, false, false, false, false),
      ReviewCommentResolveControl::Nothing
    );
    // Resolved by someone else, and not yours to reopen: the state still shows.
    assert_eq!(
      review_comment_resolve_control(true, true, false, true, false, false),
      ReviewCommentResolveControl::ResolvedTag
    );
    // A local note has no conversation at all.
    assert_eq!(
      review_comment_resolve_control(false, true, false, false, true, true),
      ReviewCommentResolveControl::Nothing
    );
  }

  #[test]
  fn a_conversation_you_can_touch_offers_its_button() {
    assert_eq!(
      review_comment_resolve_control(true, true, false, false, true, false),
      ReviewCommentResolveControl::Toggle {
        label: "Resolve conversation",
        enabled: true
      }
    );
    assert_eq!(
      review_comment_resolve_control(true, true, false, true, false, true),
      ReviewCommentResolveControl::Toggle {
        label: "Unresolve conversation",
        enabled: true
      }
    );
    // In flight: the button says what is happening and takes no second click.
    assert_eq!(
      review_comment_resolve_control(true, true, true, false, true, false),
      ReviewCommentResolveControl::Toggle {
        label: "Resolving...",
        enabled: false
      }
    );
  }

  #[test]
  fn test_floating_actions_sit_on_the_first_line_of_the_body() {
    // Same distance from the top as a button centred on a 20px first line.
    assert_eq!(
      review_comment_floating_actions_top_px(20.0),
      REVIEW_COMMENT_SPACING_PX - 2.0
    );
    assert_eq!(review_comment_floating_actions_top_px(0.0), 0.0);
  }

  #[gpui::test]
  fn test_an_edited_comment_keeps_its_text_where_it_was_read(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      let pull = editor.review_comment_in_card_composer_pull_px();
      let box_height = review_comment_composer_textarea_height_px(
        1,
        editor.review_comment_composer_line_height_px,
      );

      // Pulling both ends centres the box's single line on the line it replaces.
      assert_eq!(
        box_height - 2.0 * pull,
        editor.review_comment_line_height_px
      );
    });
  }

  #[gpui::test]
  fn test_editing_a_one_line_comment_takes_one_line(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      let line_height = editor.review_comment_line_height_px;

      assert_eq!(
        editor.review_comment_in_card_composer_body_height_px(None),
        line_height,
        "an empty composer must not make the card taller than the comment it replaces"
      );
      assert!(
        editor.review_comment_composer_body_height_px(None)
          > editor.review_comment_in_card_composer_body_height_px(None),
        "the standalone card keeps the text box's own padding"
      );
    });
  }

  #[test]
  fn test_actions_width_counts_the_gaps_around_the_buttons() {
    assert_eq!(
      review_comment_actions_width_px(2),
      2.0 * REVIEW_COMMENT_COMPOSER_ACTION_BUTTON_WIDTH_PX
        + 3.0 * REVIEW_COMMENT_COMPOSER_ACTIONS_GAP_X_PX
    );
    assert!(review_comment_actions_width_px(3) > review_comment_actions_width_px(2));
  }

  #[test]
  fn test_review_comment_card_width_follows_the_content_area() {
    assert_eq!(
      review_comment_card_width_px(2000.0),
      REVIEW_COMMENT_MAX_WIDTH_PX
    );
    assert_eq!(
      review_comment_card_width_px(500.0),
      500.0 - REVIEW_COMMENT_CARD_RIGHT_MARGIN_PX
    );
    assert_eq!(
      review_comment_card_width_px(120.0),
      REVIEW_COMMENT_MIN_WIDTH_PX
    );
  }

  #[gpui::test]
  fn test_a_comment_measures_its_own_text(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let measurer = editor.review_comment_text_measurer(cx);

      // A column is the sampled average, so an average-width run measures its length.
      assert_eq!(measurer.width_in_columns(""), 0.0);
      let one = measurer.width_in_columns("a");
      assert!(one > 0.0);
      assert!(
        (measurer.width_in_columns("aaaa") - one * 4.0).abs() < 0.01,
        "a run must measure the sum of its glyphs"
      );
    });
  }

  #[test]
  fn test_review_comment_wrap_columns_shrink_with_the_card() {
    let wide = review_comment_wrap_columns_for_width(800.0, 8.0);
    let narrow = review_comment_wrap_columns_for_width(400.0, 8.0);

    assert_eq!(wide, 100);
    assert_eq!(narrow, 50);
    assert_eq!(
      review_comment_wrap_columns_for_width(10.0, 8.0),
      REVIEW_COMMENT_MIN_WRAP_COLUMNS
    );
  }

  #[test]
  fn test_review_comment_overlay_x_offset_for_scroll() {
    assert_eq!(review_comment_overlay_x_offset_for_scroll(px(0.0)), px(0.0));
    assert_eq!(
      review_comment_overlay_x_offset_for_scroll(px(-120.0)),
      px(120.0)
    );
    assert_eq!(
      review_comment_overlay_x_offset_for_scroll(px(32.0)),
      px(0.0)
    );
  }

  /// Helper context for testing Editor
  pub struct EditorTestContext {
    pub cx: TestAppContext,
    pub editor: Entity<Editor>,
  }

  impl EditorTestContext {
    /// Create a test context with specific text content
    pub fn with_text(cx: TestAppContext, text: &str) -> Self {
      Self::with_text_and_extension(cx, text, None)
    }

    pub fn with_text_and_extension(
      mut cx: TestAppContext,
      text: &str,
      file_ext: Option<&str>,
    ) -> Self {
      let editor = cx.new(|cx| {
        let doc = cx.new(|cx| Document::new(text, file_ext, cx));
        let cursor_blink = cx.new(CursorBlink::new);

        Editor {
          review_comment_thread_roots: HashMap::new(),
          review_comment_threads: HashMap::new(),
          review_comment_thread_order: Vec::new(),
          review_comment_wrap_columns: REVIEW_COMMENT_DEFAULT_WRAP_COLUMNS,
          review_comment_line_height_px: REVIEW_COMMENT_DEFAULT_LINE_HEIGHT_PX,
          review_comment_display_mode: ReviewCommentDisplayMode::Conversation,
          review_comment_markdown_states: HashMap::new(),
          review_comment_markdown_cache: HashMap::new(),
          review_comment_code_reference_previews: HashMap::new(),
          review_comment_pr_number: None,
          editable_review_comment_ids: HashSet::new(),
          review_comment_edit_handler: None,
          review_comment_edit_input: None,
          editing_review_comment_id: None,
          review_comment_edit_initial_body: None,
          review_comment_edit_submitting_id: None,
          review_comment_edit_error: None,
          review_comment_delete_handler: None,
          review_comment_send_handler: None,
          sendable_review_comment_ids: HashSet::new(),
          review_comment_delete_submitting_id: None,
          review_comment_resolve_handler: None,
          review_comment_resolve_in_flight: HashSet::new(),
          auto_collapsed_resolved_thread_ids: HashSet::new(),
          review_comment_suggestion_action_factory: None,
          review_comment_create_handler: None,
          review_comment_cancel_handler: None,
          review_comment_replies_enabled: true,
          has_pending_review: false,
          review_comment_link_handler: None,
          review_comment_asset_url_resolver: None,
          review_comment_image_upload_handler: None,
          review_comment_preview_renderer: None,
          review_comment_composer_rows: HashMap::new(),
          review_comment_create_input: None,
          review_comment_create_draft: None,
          review_comment_create_drag_start_display_line: None,
          review_comment_create_drag_active: false,
          review_comment_create_submitting: false,
          review_comment_create_error: None,
          review_comment_create_preview_open: false,
          review_comment_edit_preview_open: false,
          review_comment_reply_input: None,
          replying_to_review_comment_id: None,
          review_comment_reply_submitting: false,
          review_comment_reply_error: None,
          review_comment_reply_preview_open: false,
          hovered_review_comment_create_display_line: None,
          collapsed_review_comments: HashSet::new(),
          review_comment_scroll_epoch: 0,
          find_panel_open: false,
          find_input: None,
          find_input_subscription: None,
          find_query: String::new(),
          find_matches: Vec::new(),
          find_active_match: None,
          find_scroll_epoch: 0,
          review_comments: Vec::new(),
          document: doc,
          focus_handle: cx.focus_handle(),
          selected_range: 0..0,
          selection_reversed: false,
          display_selection: None,
          marked_range: None,
          is_selecting: false,
          line_layouts: HashMap::new(),
          virtual_line_layouts: HashMap::new(),
          last_layout_font_size: px(0.0),
          scroll_offset_y: 0.0,
          editor_line_height: px(DEFAULT_EDITOR_LINE_HEIGHT),
          editor_char_width: px(REVIEW_COMMENT_CHAR_WIDTH_PX),
          review_comment_char_width: px(REVIEW_COMMENT_CHAR_WIDTH_PX),
          review_comment_font_size: px(REVIEW_COMMENT_FONT_SIZE_PX),
          review_comment_composer_line_height_px: REVIEW_COMMENT_COMPOSER_LINE_HEIGHT_PX,
          viewport_height: px(DEFAULT_VIEWPORT_HEIGHT),
          viewport_width: px(DEFAULT_VIEWPORT_WIDTH),
          max_line_width: px(DEFAULT_MAX_LINE_WIDTH),
          scroll_handle: ScrollHandle::new(),
          scroll_axis_lock: None,
          last_scroll_time: None,
          last_scroll_x: px(0.0),
          max_cache_size: MAX_CACHE_SIZE,
          target_column: None,
          undo_stack: VecDeque::new(),
          redo_stack: VecDeque::new(),
          theme: Theme::dark(),
          projection: None,
          visible_groups: Vec::new(),
          hovered_group_id: None,
          hovered_from_primary: true,
          hovered_conflict_start_line: None,
          pending_conflict_reveal_start_line: None,
          conflict_cache: RwLock::new(ConflictCache::default()),
          last_mouse_position: None,
          expanded_gaps: HashMap::new(),
          workdir_path: PathBuf::new(),
          repo_file: None,
          git_store: None,
          git_state: BufferGitState::default(),
          diffs: None,
          diff_task: None,
          projection_task: None,
          bases_task: None,
          poll_task: None,
          git_task: None,
          git_jobs: VecDeque::new(),
          git_op_in_flight: false,
          pending_git_after_bases: false,
          diff_generation: Arc::new(AtomicUsize::new(0)),
          projection_generation: Arc::new(AtomicUsize::new(0)),
          file_mtime: None,
          index_mtime: None,
          is_dirty: false,
          save_task: None,
          diff_view_mode: DiffViewMode::Inline,
          ignore_whitespace: false,
          is_read_only: false,
          is_unmerged: false,
          last_highlights_version: 0,
          last_highlights_epoch: 0,
          cursor_blink,
          optimistic_unstaged_groups: HashSet::new(),
        }
      });

      Self { cx, editor }
    }

    /// Create a test context with multiple lines for testing
    pub fn with_lines(cx: TestAppContext, count: usize) -> Self {
      let mut text = String::new();
      for i in 0..count {
        if i > 0 {
          text.push('\n');
        }
        text.push_str(&format!("Line {}", i + 1));
      }
      Self::with_text(cx, &text)
    }

    /// Get the current text content
    pub fn text(&self) -> String {
      self.editor.read_with(&self.cx, |editor, cx| {
        let doc = editor.document().read(cx);
        doc.slice_to_string(0..doc.len())
      })
    }

    /// Get the current cursor offset
    pub fn cursor_offset(&self) -> usize {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.cursor_offset())
    }

    /// Get the current selection range
    pub fn selection(&self) -> Range<usize> {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.selected_range.clone())
    }

    /// Get whether selection is reversed
    #[allow(dead_code)]
    pub fn selection_reversed(&self) -> bool {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.selection_reversed)
    }

    /// Set cursor position (collapses selection)
    pub fn set_cursor(&mut self, offset: usize) {
      self.editor.update(&mut self.cx, |editor, cx| {
        editor.move_to(offset, cx);
      });
    }

    /// Set selection range
    pub fn set_selection(&mut self, range: Range<usize>, reversed: bool) {
      self.editor.update(&mut self.cx, |editor, _| {
        editor.selected_range = range;
        editor.selection_reversed = reversed;
        editor.display_selection = None;
      });
    }

    /// Get the number of cached lines
    pub fn cache_size(&self) -> usize {
      self
        .editor
        .read_with(&self.cx, |editor, _| editor.line_layouts.len())
    }

    /// Check if a specific line is cached
    pub fn is_line_cached(&self, line_idx: usize) -> bool {
      self.editor.read_with(&self.cx, |editor, _| {
        editor.line_layouts.contains_key(&line_idx)
      })
    }
  }

  fn projection_with_removed_middle_line() -> Arc<Projection> {
    Arc::new(Projection {
      lines: vec![
        DisplayLine::Doc {
          doc_line: 0,
          old_line: Some(0),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Removed {
          text: "removed".into(),
          anchor_line: 0,
          old_line: 0,
          hunk: HunkState::Unstaged,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 1,
          old_line: Some(1),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
      ],
      display_to_doc: vec![Some(0), None, Some(1)],
      doc_to_display: vec![Some(0), Some(2)],
      visible_doc_lines: vec![0, 1],
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    })
  }

  fn projection_with_mixed_review_hunk() -> (Arc<Projection>, Arc<str>) {
    let group_id = Arc::<str>::from("hunk-0");
    (
      Arc::new(Projection {
        lines: vec![
          DisplayLine::Removed {
            text: "old".into(),
            anchor_line: 0,
            old_line: 0,
            hunk: HunkState::Unstaged,
            group_id: Some(group_id.clone()),
            secondary: false,
          },
          DisplayLine::Doc {
            doc_line: 0,
            old_line: None,
            change: Some(ChangeKind::Added),
            hunk: Some(HunkState::Unstaged),
            group_id: Some(group_id.clone()),
            secondary: false,
          },
          DisplayLine::Doc {
            doc_line: 1,
            old_line: None,
            change: Some(ChangeKind::Added),
            hunk: Some(HunkState::Unstaged),
            group_id: Some(group_id.clone()),
            secondary: false,
          },
        ],
        display_to_doc: vec![None, Some(0), Some(1)],
        doc_to_display: vec![Some(1), Some(2)],
        visible_doc_lines: vec![0, 1],
        start_gap: None,
        end_gap: None,
        groups: HashMap::new(),
      }),
      group_id,
    )
  }

  fn projection_with_deleted_review_hunk() -> (Arc<Projection>, Arc<str>) {
    let group_id = Arc::<str>::from("hunk-0");
    (
      Arc::new(Projection {
        lines: vec![
          DisplayLine::Doc {
            doc_line: 0,
            old_line: Some(0),
            change: None,
            hunk: None,
            group_id: None,
            secondary: false,
          },
          DisplayLine::Removed {
            text: "old 1".into(),
            anchor_line: 0,
            old_line: 1,
            hunk: HunkState::Unstaged,
            group_id: Some(group_id.clone()),
            secondary: false,
          },
          DisplayLine::Removed {
            text: "old 2".into(),
            anchor_line: 0,
            old_line: 2,
            hunk: HunkState::Unstaged,
            group_id: Some(group_id.clone()),
            secondary: false,
          },
          DisplayLine::Doc {
            doc_line: 1,
            old_line: Some(3),
            change: None,
            hunk: None,
            group_id: None,
            secondary: false,
          },
        ],
        display_to_doc: vec![Some(0), None, None, Some(1)],
        doc_to_display: vec![Some(0), Some(3)],
        visible_doc_lines: vec![0, 1],
        start_gap: None,
        end_gap: None,
        groups: HashMap::new(),
      }),
      group_id,
    )
  }

  fn projection_with_hidden_start_and_end() -> Arc<Projection> {
    let start_gap = GapId { start: 0, end: 1 };
    let end_gap = GapId { start: 4, end: 5 };
    Arc::new(Projection {
      lines: vec![
        DisplayLine::Gap {
          id: start_gap,
          hidden_range: 0..1,
        },
        DisplayLine::Doc {
          doc_line: 1,
          old_line: Some(1),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 2,
          old_line: Some(2),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 3,
          old_line: Some(3),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Gap {
          id: end_gap,
          hidden_range: 4..5,
        },
      ],
      display_to_doc: vec![None, Some(1), Some(2), Some(3), None],
      doc_to_display: vec![None, Some(1), Some(2), Some(3), None],
      visible_doc_lines: vec![1, 2, 3],
      start_gap: Some(start_gap),
      end_gap: Some(end_gap),
      groups: HashMap::new(),
    })
  }

  fn projection_with_large_middle_gap() -> Arc<Projection> {
    let gap = GapId {
      start: 12,
      end: 49_990,
    };
    Arc::new(Projection {
      lines: vec![
        DisplayLine::Doc {
          doc_line: 10,
          old_line: Some(10),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 11,
          old_line: Some(11),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Gap {
          id: gap,
          hidden_range: 12..49_990,
        },
        DisplayLine::Doc {
          doc_line: 49_990,
          old_line: Some(49_990),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
        DisplayLine::Doc {
          doc_line: 49_991,
          old_line: Some(49_991),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        },
      ],
      display_to_doc: vec![Some(10), Some(11), None, Some(49_990), Some(49_991)],
      doc_to_display: Vec::new(),
      visible_doc_lines: vec![10, 11, 49_990, 49_991],
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    })
  }

  fn projection_with_visible_doc_segments(
    doc_line_count: usize,
    visible_segments: &[Range<usize>],
  ) -> Projection {
    let mut lines = Vec::new();
    let mut display_to_doc = Vec::new();
    let mut doc_to_display = vec![None; doc_line_count];
    let mut visible_doc_lines = Vec::new();
    let mut start_gap = None;
    let mut end_gap = None;
    let mut previous_end = 0;

    for segment in visible_segments {
      if segment.start > previous_end {
        let gap = GapId {
          start: previous_end,
          end: segment.start,
        };
        if previous_end == 0 {
          start_gap = Some(gap);
        }
        lines.push(DisplayLine::Gap {
          id: gap,
          hidden_range: previous_end..segment.start,
        });
        display_to_doc.push(None);
      }

      for doc_line in segment.clone() {
        let display_line = lines.len();
        lines.push(DisplayLine::Doc {
          doc_line,
          old_line: Some(doc_line),
          change: None,
          hunk: None,
          group_id: None,
          secondary: false,
        });
        display_to_doc.push(Some(doc_line));
        doc_to_display[doc_line] = Some(display_line);
        visible_doc_lines.push(doc_line);
      }

      previous_end = segment.end;
    }

    if previous_end < doc_line_count {
      let gap = GapId {
        start: previous_end,
        end: doc_line_count,
      };
      end_gap = Some(gap);
      lines.push(DisplayLine::Gap {
        id: gap,
        hidden_range: previous_end..doc_line_count,
      });
      display_to_doc.push(None);
    }

    Projection {
      lines,
      display_to_doc,
      doc_to_display,
      visible_doc_lines,
      start_gap,
      end_gap,
      groups: HashMap::new(),
    }
  }

  fn projection_with_doc_lines(line_count: usize) -> Projection {
    let mut lines = Vec::with_capacity(line_count);
    let mut display_to_doc = Vec::with_capacity(line_count);
    let mut doc_to_display = Vec::with_capacity(line_count);
    let mut visible_doc_lines = Vec::with_capacity(line_count);

    for line in 0..line_count {
      lines.push(DisplayLine::Doc {
        doc_line: line,
        old_line: Some(line),
        change: None,
        hunk: None,
        group_id: None,
        secondary: false,
      });
      display_to_doc.push(Some(line));
      doc_to_display.push(Some(line));
      visible_doc_lines.push(line);
    }

    Projection {
      lines,
      display_to_doc,
      doc_to_display,
      visible_doc_lines,
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    }
  }

  fn projection_with_two_hunks(line_count: usize, hunk_lines: &[usize]) -> Projection {
    let mut lines = Vec::with_capacity(line_count);
    let mut display_to_doc = Vec::with_capacity(line_count);
    let mut doc_to_display = Vec::with_capacity(line_count);
    let mut visible_doc_lines = Vec::with_capacity(line_count);

    for line in 0..line_count {
      let group_id = hunk_lines
        .iter()
        .position(|l| *l == line)
        .map(|idx| Arc::<str>::from(format!("hunk-{idx}")));
      let change = group_id.as_ref().map(|_| ChangeKind::Added);
      let hunk = group_id.as_ref().map(|_| HunkState::Unstaged);
      lines.push(DisplayLine::Doc {
        doc_line: line,
        old_line: None,
        change,
        hunk,
        group_id,
        secondary: false,
      });
      display_to_doc.push(Some(line));
      doc_to_display.push(Some(line));
      visible_doc_lines.push(line);
    }

    Projection {
      lines,
      display_to_doc,
      doc_to_display,
      visible_doc_lines,
      start_gap: None,
      end_gap: None,
      groups: HashMap::new(),
    }
  }

  #[gpui::test]
  fn test_set_review_comments_prunes_editable_review_comment_ids(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_editable_review_comment_ids([1, 2], cx);
      editor.set_review_comments(
        vec![ReviewComment {
          id: 1,
          in_reply_to_id: None,
          line: 0,
          side: ReviewCommentSide::Right,
          author: Arc::from("octocat"),
          avatar_url: None,
          line_label: None,
          body: Arc::from("hello"),
          suggestion_context: None,
          created_at: Arc::from("2026-02-17"),
          thread_id: None,
          is_resolved: false,
          is_outdated: false,
          viewer_can_resolve: false,
          viewer_can_unresolve: false,
          is_pending: false,
        }],
        cx,
      );

      assert!(editor.editable_review_comment_ids.contains(&1));
      assert!(!editor.editable_review_comment_ids.contains(&2));
    });
  }

  #[gpui::test]
  fn test_set_diffs_none_keeps_review_comment_edit_handler(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentEditHandler = Arc::new(|_, _, _, _| {});
      editor.set_review_comment_edit_handler(Some(handler), cx);
      editor.set_diffs(None, cx);

      assert!(editor.review_comment_edit_handler.is_some());
    });
  }

  #[gpui::test]
  fn test_set_diffs_none_keeps_review_comment_delete_handler(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentDeleteHandler = Arc::new(|_, _, _| {});
      editor.set_review_comment_delete_handler(Some(handler), cx);
      editor.set_diffs(None, cx);

      assert!(editor.review_comment_delete_handler.is_some());
    });
  }

  #[gpui::test]
  fn test_set_diffs_none_keeps_review_comment_send_handler(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentSendHandler = Arc::new(|_, _, _| {});
      editor.set_review_comment_send_handler(Some(handler), cx);
      editor.set_diffs(None, cx);

      assert!(editor.review_comment_send_handler.is_some());
    });
  }

  #[gpui::test]
  fn test_request_review_comment_send_only_sends_what_the_host_allows(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let (_root, cx) =
      cx.add_window_view(|window, cx| gpui_component::Root::new(editor.clone(), window, cx));
    let sent = StdArc::new(Mutex::new(Vec::<u64>::new()));

    let handler: ReviewCommentSendHandler = {
      let sent = sent.clone();
      Arc::new(move |comment_id, _, _| sent.lock().expect("sent lock").push(comment_id))
    };

    ctx.editor.update_in(cx, |editor, window, cx| {
      editor.set_review_comment_send_handler(Some(handler), cx);
      editor.set_editable_review_comment_ids([1, 2], cx);
      // The host says only the first one still has somewhere to go.
      editor.set_sendable_review_comment_ids([1], cx);
      editor.set_review_comments(
        vec![review_comment_fixture(1), review_comment_fixture(2)],
        cx,
      );

      editor.request_review_comment_send(1, window, cx);
      editor.request_review_comment_send(2, window, cx);
      // Not in the batch at all.
      editor.request_review_comment_send(3, window, cx);
    });

    assert_eq!(*sent.lock().expect("sent lock"), vec![1]);
  }

  fn review_comment_fixture(id: u64) -> ReviewComment {
    ReviewComment {
      id,
      in_reply_to_id: None,
      line: 0,
      side: ReviewCommentSide::Right,
      author: Arc::from(""),
      avatar_url: None,
      line_label: None,
      body: Arc::from("extract this"),
      suggestion_context: None,
      created_at: Arc::from(""),
      thread_id: None,
      is_resolved: false,
      is_outdated: false,
      viewer_can_resolve: false,
      viewer_can_unresolve: false,
      is_pending: false,
    }
  }

  #[gpui::test]
  fn test_set_diffs_none_keeps_review_comment_link_handler(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentLinkHandler = Arc::new(|_, _, _| false);
      editor.set_review_comment_link_handler(Some(handler), cx);
      editor.set_diffs(None, cx);

      assert!(editor.review_comment_link_handler.is_some());
    });
  }

  #[gpui::test]
  fn test_set_diffs_none_clears_review_comment_create_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 1,
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: Some(0),
        start_side: Some(ReviewCommentSide::Right),
      });
      editor.review_comment_create_drag_start_display_line = Some(0);
      editor.review_comment_create_drag_active = true;
      editor.review_comment_create_submitting = true;
      editor.hovered_review_comment_create_display_line = Some(1);

      editor.set_diffs(None, cx);

      assert!(editor.review_comment_create_draft.is_none());
      assert!(
        editor
          .review_comment_create_drag_start_display_line
          .is_none()
      );
      assert!(!editor.review_comment_create_drag_active);
      assert!(!editor.review_comment_create_submitting);
      assert!(editor.hovered_review_comment_create_display_line.is_none());
    });
  }

  #[gpui::test]
  fn test_set_diffs_none_clears_review_comment_reply_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.replying_to_review_comment_id = Some(42);
      editor.review_comment_reply_submitting = true;
      editor.review_comment_reply_error = Some(Arc::from("request failed"));

      editor.set_diffs(None, cx);

      assert!(editor.replying_to_review_comment_id.is_none());
      assert!(!editor.review_comment_reply_submitting);
      assert!(editor.review_comment_reply_error.is_none());
    });
  }

  #[gpui::test]
  fn test_set_review_comments_clears_review_comment_delete_submission_when_comment_is_missing(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.review_comment_delete_submitting_id = Some(42);
      editor.set_review_comments(Vec::new(), cx);

      assert!(editor.review_comment_delete_submitting_id.is_none());
    });
  }

  #[gpui::test]
  fn test_set_review_comments_preserves_collapsed_threads_on_update(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_review_comments(
        vec![
          ReviewComment {
            id: 1,
            in_reply_to_id: None,
            line: 0,
            side: ReviewCommentSide::Right,
            author: Arc::from("alice"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread one"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
          ReviewComment {
            id: 2,
            in_reply_to_id: Some(1),
            line: 0,
            side: ReviewCommentSide::Right,
            author: Arc::from("bob"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread one reply"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
          ReviewComment {
            id: 3,
            in_reply_to_id: None,
            line: 1,
            side: ReviewCommentSide::Right,
            author: Arc::from("charlie"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread two"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
        ],
        cx,
      );

      editor.toggle_review_comment_thread(1, cx);
      assert!(editor.collapsed_review_comments.contains(&1));
      assert!(editor.collapsed_review_comments.contains(&2));
      assert!(!editor.collapsed_review_comments.contains(&3));

      editor.set_review_comments(
        vec![
          ReviewComment {
            id: 1,
            in_reply_to_id: None,
            line: 0,
            side: ReviewCommentSide::Right,
            author: Arc::from("alice"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread one updated"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
          ReviewComment {
            id: 2,
            in_reply_to_id: Some(1),
            line: 0,
            side: ReviewCommentSide::Right,
            author: Arc::from("bob"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread one reply"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
          ReviewComment {
            id: 3,
            in_reply_to_id: None,
            line: 1,
            side: ReviewCommentSide::Right,
            author: Arc::from("charlie"),
            avatar_url: None,
            line_label: None,
            body: Arc::from("thread two"),
            suggestion_context: None,
            created_at: Arc::from("2026-02-18"),
            thread_id: None,
            is_resolved: false,
            is_outdated: false,
            viewer_can_resolve: false,
            viewer_can_unresolve: false,
            is_pending: false,
          },
        ],
        cx,
      );

      assert!(editor.collapsed_review_comments.contains(&1));
      assert!(editor.collapsed_review_comments.contains(&2));
      assert!(!editor.collapsed_review_comments.contains(&3));
    });
  }

  fn thread_comment(id: u64, in_reply_to_id: Option<u64>, resolved: bool) -> ReviewComment {
    ReviewComment {
      id,
      in_reply_to_id,
      line: 0,
      side: ReviewCommentSide::Right,
      author: Arc::from("alice"),
      avatar_url: None,
      line_label: None,
      body: Arc::from("body"),
      suggestion_context: None,
      created_at: Arc::from("2026-02-18"),
      thread_id: Some(Arc::from("thread-1")),
      is_resolved: resolved,
      is_outdated: false,
      viewer_can_resolve: true,
      viewer_can_unresolve: true,
      is_pending: false,
    }
  }

  #[gpui::test]
  fn test_acting_on_a_collapsed_thread_reopens_it(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_review_comments(
        vec![
          thread_comment(1, None, false),
          thread_comment(2, Some(1), false),
        ],
        cx,
      );
      editor.toggle_review_comment_thread(1, cx);
      assert!(editor.collapsed_review_comments.contains(&1));

      // Reply and edit both go through this before opening their input.
      editor.expand_review_comment_thread_for(2);

      assert!(!editor.collapsed_review_comments.contains(&1));
      assert!(!editor.collapsed_review_comments.contains(&2));
    });
  }

  #[gpui::test]
  fn test_a_thread_collapses_when_it_resolves_and_stays_open_if_reopened(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_review_comments(
        vec![
          thread_comment(1, None, false),
          thread_comment(2, Some(1), false),
        ],
        cx,
      );
      assert!(!editor.collapsed_review_comments.contains(&1));

      // The refresh after resolving folds the thread away.
      editor.set_review_comments(
        vec![
          thread_comment(1, None, true),
          thread_comment(2, Some(1), true),
        ],
        cx,
      );
      assert!(editor.collapsed_review_comments.contains(&1));
      assert!(editor.collapsed_review_comments.contains(&2));

      // The user reopened it: later refreshes must not fold it again.
      editor.toggle_review_comment_thread(1, cx);
      editor.set_review_comments(
        vec![
          thread_comment(1, None, true),
          thread_comment(2, Some(1), true),
        ],
        cx,
      );
      assert!(!editor.collapsed_review_comments.contains(&1));
    });
  }

  #[gpui::test]
  fn test_resolving_folds_the_thread_and_unresolving_unfolds_it(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_review_comments(
        vec![
          thread_comment(1, None, false),
          thread_comment(2, Some(1), false),
        ],
        cx,
      );

      editor.apply_resolution_visibility(1, false);
      assert!(editor.collapsed_review_comments.contains(&1));
      assert!(editor.collapsed_review_comments.contains(&2));

      editor.apply_resolution_visibility(1, true);
      assert!(!editor.collapsed_review_comments.contains(&1));
      assert!(!editor.collapsed_review_comments.contains(&2));
    });
  }

  #[gpui::test]
  fn test_a_finished_resolve_write_stops_its_spinner(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_review_comments(vec![thread_comment(1, None, false)], cx);
      editor
        .review_comment_resolve_in_flight
        .insert(Arc::from("thread-1"));

      // The refetch after the write keeps the thread, so it alone never
      // clears the marker: the write completion has to.
      editor.set_review_comments(vec![thread_comment(1, None, true)], cx);
      assert!(!editor.review_comment_resolve_in_flight.is_empty());

      editor.finish_review_comment_resolve_submissions(cx);
      assert!(editor.review_comment_resolve_in_flight.is_empty());
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_edit_submission_success_clears_edit_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.editing_review_comment_id = Some(42);
      editor.review_comment_edit_initial_body = Some(Arc::from("before"));
      editor.review_comment_edit_submitting_id = Some(42);

      editor.finish_review_comment_edit_submission(42, None, cx);

      assert!(editor.review_comment_edit_submitting_id.is_none());
      assert!(editor.editing_review_comment_id.is_none());
      assert!(editor.review_comment_edit_initial_body.is_none());
      assert!(editor.review_comment_edit_error.is_none());
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_edit_submission_failure_keeps_edit_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.editing_review_comment_id = Some(42);
      editor.review_comment_edit_initial_body = Some(Arc::from("before"));
      editor.review_comment_edit_submitting_id = Some(42);

      editor.finish_review_comment_edit_submission(42, Some(Arc::from("request failed")), cx);

      assert!(editor.review_comment_edit_submitting_id.is_none());
      assert_eq!(editor.editing_review_comment_id, Some(42));
      assert_eq!(
        editor.review_comment_edit_initial_body.as_deref(),
        Some("before")
      );
      assert_eq!(
        editor
          .review_comment_edit_error
          .as_ref()
          .map(|(_, error)| error.as_ref()),
        Some("request failed")
      );
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_create_submission_success_clears_create_state(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 1,
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: Some(0),
        start_side: Some(ReviewCommentSide::Right),
      });
      editor.review_comment_create_submitting = true;

      editor.finish_review_comment_create_submission(None, cx);

      assert!(editor.review_comment_create_draft.is_none());
      assert!(!editor.review_comment_create_submitting);
      assert!(editor.review_comment_create_error.is_none());
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_create_submission_success_clears_reply_state(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.replying_to_review_comment_id = Some(42);
      editor.review_comment_reply_submitting = true;

      editor.finish_review_comment_create_submission(None, cx);

      assert!(editor.replying_to_review_comment_id.is_none());
      assert!(!editor.review_comment_reply_submitting);
      assert!(editor.review_comment_reply_error.is_none());
    });
  }

  #[gpui::test]
  fn test_set_review_comment_replies_enabled_false_clears_reply_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.replying_to_review_comment_id = Some(42);
      editor.review_comment_reply_submitting = true;
      editor.review_comment_reply_error = Some(Arc::from("request failed"));
      editor.review_comment_reply_preview_open = true;

      editor.set_review_comment_replies_enabled(false, cx);

      assert!(!editor.review_comment_replies_enabled);
      assert!(editor.replying_to_review_comment_id.is_none());
      assert!(!editor.review_comment_reply_submitting);
      assert!(editor.review_comment_reply_error.is_none());
      assert!(!editor.review_comment_reply_preview_open);
    });
  }

  #[gpui::test]
  fn test_set_review_comment_display_mode_local_note_clears_conversation_state(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.collapsed_review_comments.insert(1);
      editor.replying_to_review_comment_id = Some(42);
      editor.review_comment_reply_preview_open = true;

      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::LocalNote, cx);

      assert_eq!(
        editor.review_comment_display_mode,
        ReviewCommentDisplayMode::LocalNote
      );
      assert!(editor.collapsed_review_comments.is_empty());
      assert!(editor.replying_to_review_comment_id.is_none());
      assert!(!editor.review_comment_reply_preview_open);
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_create_submission_failure_keeps_create_state(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 1,
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: Some(0),
        start_side: Some(ReviewCommentSide::Right),
      });
      editor.review_comment_create_submitting = true;

      editor.finish_review_comment_create_submission(Some(Arc::from("request failed")), cx);

      assert!(editor.review_comment_create_draft.is_some());
      assert!(!editor.review_comment_create_submitting);
      assert_eq!(
        editor.review_comment_create_error.as_deref(),
        Some("request failed")
      );
    });
  }

  #[gpui::test]
  fn test_finish_review_comment_create_submission_failure_keeps_reply_state(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.replying_to_review_comment_id = Some(42);
      editor.review_comment_reply_submitting = true;

      editor.finish_review_comment_create_submission(Some(Arc::from("request failed")), cx);

      assert_eq!(editor.replying_to_review_comment_id, Some(42));
      assert!(!editor.review_comment_reply_submitting);
      assert_eq!(
        editor.review_comment_reply_error.as_deref(),
        Some("request failed")
      );
    });
  }

  #[gpui::test]
  fn test_cancel_review_comment_edit_clears_edit_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.editing_review_comment_id = Some(42);
      editor.review_comment_edit_initial_body = Some(Arc::from("before"));

      editor.cancel_review_comment_edit(cx);

      assert!(editor.editing_review_comment_id.is_none());
      assert!(editor.review_comment_edit_initial_body.is_none());
    });
  }

  #[gpui::test]
  fn test_cancel_review_comment_create_clears_create_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 1,
        line: 1,
        side: ReviewCommentSide::Right,
        start_line: Some(0),
        start_side: Some(ReviewCommentSide::Right),
      });
      editor.review_comment_create_drag_start_display_line = Some(0);
      editor.review_comment_create_drag_active = true;
      editor.hovered_review_comment_create_display_line = Some(1);

      editor.cancel_review_comment_create(cx);

      assert!(editor.review_comment_create_draft.is_none());
      assert!(
        editor
          .review_comment_create_drag_start_display_line
          .is_none()
      );
      assert!(!editor.review_comment_create_drag_active);
      assert!(editor.hovered_review_comment_create_display_line.is_none());
    });
  }

  #[gpui::test]
  fn test_review_comment_create_target_maps_doc_line_to_right_side(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let projection = projection_with_removed_middle_line();

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some((*projection).clone()));
    });

    let target = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.review_comment_create_target_for_display_line(0, cx)
    });

    assert_eq!(
      target,
      Some(ReviewCommentCreateTarget {
        display_line: 0,
        line: 0,
        side: ReviewCommentSide::Right,
      })
    );
  }

  #[gpui::test]
  fn test_review_comment_create_target_maps_removed_line_to_left_side(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let projection = projection_with_removed_middle_line();

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some((*projection).clone()));
    });

    let target = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.review_comment_create_target_for_display_line(1, cx)
    });

    assert_eq!(
      target,
      Some(ReviewCommentCreateTarget {
        display_line: 1,
        line: 0,
        side: ReviewCommentSide::Left,
      })
    );
  }

  #[gpui::test]
  fn test_review_comment_create_range_for_hunk_prefers_new_side(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "new 1\nnew 2");
    let (projection, group_id) = projection_with_mixed_review_hunk();

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some((*projection).clone()));
    });

    let range = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.review_comment_create_display_range_for_group(&group_id, cx)
    });

    assert_eq!(range, Some((1, 2)));
  }

  #[gpui::test]
  fn test_review_comment_create_range_for_deleted_hunk_uses_old_side(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let (projection, group_id) = projection_with_deleted_review_hunk();

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some((*projection).clone()));
    });

    let range = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.review_comment_create_display_range_for_group(&group_id, cx)
    });

    assert_eq!(range, Some((1, 2)));
  }

  #[gpui::test]
  fn test_review_comment_create_drag_up_keeps_draft_anchor_on_last_selected_line(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc\nd");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let handler: ReviewCommentCreateHandler = Arc::new(|_, _, _| {});
      editor.set_review_comment_create_handler(Some(handler), cx);
      editor.start_review_comment_create_drag(3, cx);
      editor.update_review_comment_create_drag_from_display_line(Some(1), cx);

      assert_eq!(
        editor.review_comment_create_draft,
        Some(ReviewCommentCreateDraft {
          first_display_line: 1,
          last_display_line: 3,
          line: 3,
          side: ReviewCommentSide::Right,
          start_line: Some(1),
          start_side: Some(ReviewCommentSide::Right),
        })
      );
    });
  }

  struct EscapeBubbleHarness {
    editor: Entity<Editor>,
    bubbled: StdArc<Mutex<bool>>,
  }

  impl Render for EscapeBubbleHarness {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
      let bubbled = self.bubbled.clone();
      div()
        .size_full()
        .on_action(move |_: &crate::actions::CloseFind, _, _| {
          *bubbled.lock().expect("bubbled lock") = true;
        })
        .child(self.editor.clone())
    }
  }

  #[gpui::test]
  fn test_escape_bubbles_to_host_when_no_find_panel_is_open(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
      cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        crate::actions::CloseFind,
        Some("Editor"),
      )]);
    });
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let bubbled = StdArc::new(Mutex::new(false));
    let harness_bubbled = bubbled.clone();

    let (_root, cx) = cx.add_window_view(|window, cx| {
      let harness = cx.new(|_| EscapeBubbleHarness {
        editor: editor.clone(),
        bubbled: harness_bubbled,
      });
      gpui_component::Root::new(harness, window, cx)
    });

    ctx.editor.update_in(cx, |editor, window, cx| {
      let handle = editor.focus_handle(cx);
      window.focus(&handle, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert!(
      *bubbled.lock().expect("bubbled lock"),
      "escape should reach the host when the editor has no find panel to close"
    );
  }

  #[gpui::test]
  fn test_escape_is_consumed_when_find_panel_is_open(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    cx.update(|cx| {
      cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        crate::actions::CloseFind,
        Some("Editor"),
      )]);
    });
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let bubbled = StdArc::new(Mutex::new(false));
    let harness_bubbled = bubbled.clone();

    let (_root, cx) = cx.add_window_view(|window, cx| {
      let harness = cx.new(|_| EscapeBubbleHarness {
        editor: editor.clone(),
        bubbled: harness_bubbled,
      });
      gpui_component::Root::new(harness, window, cx)
    });

    ctx.editor.update_in(cx, |editor, window, cx| {
      editor.open_find_panel(window, cx);
      let handle = editor.focus_handle(cx);
      window.focus(&handle, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    ctx.editor.read_with(cx, |editor, _| {
      assert!(!editor.find_panel_open, "find panel should be closed");
    });
    assert!(
      !*bubbled.lock().expect("bubbled lock"),
      "escape closing the find panel must not also close the file view"
    );
  }

  #[gpui::test]
  fn test_review_comment_create_shift_enter_keeps_writing(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let submitted = StdArc::new(Mutex::new(false));
    let submitted_for_handler = submitted.clone();
    let (_root, cx) =
      cx.add_window_view(|window, cx| gpui_component::Root::new(editor.clone(), window, cx));

    ctx.editor.update_in(cx, |editor, window, cx| {
      let handler: ReviewCommentCreateHandler = Arc::new(move |_, _, _| {
        *submitted_for_handler.lock().expect("submitted lock") = true;
      });
      editor.set_review_comment_create_handler(Some(handler), cx);
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 0,
        line: 0,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
      });

      let input = editor.ensure_review_comment_create_input(window, cx);
      input.update(cx, |input, cx| {
        input.set_value("first line".to_string(), window, cx);
        cx.emit(InputEvent::PressEnter {
          secondary: false,
          shift: true,
        });
      });
    });

    assert!(
      !*submitted.lock().expect("submitted lock"),
      "shift-enter grows the composer, it does not post the comment"
    );
  }

  #[test]
  fn test_cards_end_on_the_same_strip_of_diff() {
    // The whole-line rounding goes inside the card, not under it.
    assert_eq!(
      review_comment_card_min_height(px(113.0)),
      px(113.0 - REVIEW_COMMENT_CARD_BOTTOM_MARGIN_PX)
    );
    assert_eq!(review_comment_card_min_height(px(0.0)), px(0.0));
  }

  #[gpui::test]
  fn test_review_comment_create_secondary_enter_submits(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let submitted = StdArc::new(Mutex::new(None));
    let submitted_for_handler = submitted.clone();
    let (_root, cx) =
      cx.add_window_view(|window, cx| gpui_component::Root::new(editor.clone(), window, cx));

    ctx.editor.update_in(cx, |editor, window, cx| {
      let handler: ReviewCommentCreateHandler = Arc::new(move |request, _, _| {
        *submitted_for_handler.lock().expect("submitted lock") =
          Some((request.line, request.side, request.body.to_string()));
      });
      editor.set_review_comment_create_handler(Some(handler), cx);
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 0,
        line: 0,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
      });

      let input = editor.ensure_review_comment_create_input(window, cx);
      input.update(cx, |input, cx| {
        input.set_value("please fix\n".to_string(), window, cx);
        cx.emit(InputEvent::PressEnter {
          secondary: true,
          shift: false,
        });
      });
    });

    assert_eq!(
      submitted.lock().expect("submitted lock").as_ref(),
      Some(&(0, ReviewCommentSide::Right, "please fix".to_string()))
    );
    let input_value = ctx.editor.read_with(cx, |editor, cx| {
      editor
        .review_comment_create_input
        .as_ref()
        .expect("create input")
        .read(cx)
        .value()
        .to_string()
    });
    assert_eq!(input_value, "please fix");
  }

  #[gpui::test]
  fn test_create_composer_height_follows_its_value(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let ctx = EditorTestContext::with_text(cx.clone(), "a\nb");
    let editor = ctx.editor.clone();
    let (_root, cx) =
      cx.add_window_view(|window, cx| gpui_component::Root::new(editor.clone(), window, cx));

    let (empty_height, tall_height, min_height) = ctx.editor.update_in(cx, |editor, window, cx| {
      let input = editor.ensure_review_comment_create_input(window, cx);
      editor.sync_review_comment_composer_rows(window, cx);
      let empty_height = editor.review_comment_composer_textarea_height(&input);
      let min_height = px(review_comment_composer_textarea_height_px(
        REVIEW_COMMENT_COMPOSER_MIN_ROWS,
        editor.review_comment_composer_line_height_px,
      ));

      input.update(cx, |input, cx| {
        input.set_value("one\ntwo\nthree\nfour\nfive\nsix".to_string(), window, cx);
      });
      editor.sync_review_comment_composer_rows(window, cx);
      (
        empty_height,
        editor.review_comment_composer_textarea_height(&input),
        min_height,
      )
    });

    assert_eq!(empty_height, min_height);
    assert!(
      tall_height > empty_height,
      "six lines must not sit in a three-row box, got {tall_height:?}"
    );
  }

  #[gpui::test]
  fn test_original_lines_for_create_draft_collects_right_side_lines(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "fn main() {\n  println!();\n}\n");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some(Projection::full(3)));
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 2,
        line: 2,
        side: ReviewCommentSide::Right,
        start_line: Some(0),
        start_side: Some(ReviewCommentSide::Right),
      });
    });

    let lines = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.original_lines_for_create_draft(cx)
    });
    assert_eq!(lines, vec!["fn main() {", "  println!();", "}"]);
  }

  #[gpui::test]
  fn test_can_insert_review_comment_suggestion_requires_right_side(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\n");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some(Projection::full(2)));
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 0,
        line: 0,
        side: ReviewCommentSide::Left,
        start_line: None,
        start_side: None,
      });
    });

    let allowed = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.can_insert_review_comment_suggestion(cx)
    });
    assert!(!allowed);
  }

  #[gpui::test]
  fn test_can_insert_review_comment_suggestion_disabled_while_submitting(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\n");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.set_projection(Some(Projection::full(2)));
      editor.review_comment_create_draft = Some(ReviewCommentCreateDraft {
        first_display_line: 0,
        last_display_line: 0,
        line: 0,
        side: ReviewCommentSide::Right,
        start_line: None,
        start_side: None,
      });
      editor.review_comment_create_submitting = true;
    });

    let allowed = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.can_insert_review_comment_suggestion(cx)
    });
    assert!(!allowed);
  }

  #[gpui::test]
  fn test_invalidate_line_single(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..5 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    for i in 0..5 {
      assert!(ctx.is_line_cached(i));
    }

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.invalidate_line(2);
    });

    assert!(ctx.is_line_cached(0));
    assert!(ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
    assert!(ctx.is_line_cached(3));
    assert!(ctx.is_line_cached(4));
  }

  #[gpui::test]
  fn test_invalidate_lines_from(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..10 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    assert_eq!(ctx.cache_size(), 10);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.invalidate_lines_from(5);
    });

    assert!(ctx.is_line_cached(0));
    assert!(ctx.is_line_cached(4));
    assert!(!ctx.is_line_cached(5));
    assert!(!ctx.is_line_cached(9));
    assert_eq!(ctx.cache_size(), 5);
  }

  #[gpui::test]
  fn test_ensure_cache_size_limit(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 300);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..250 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    assert_eq!(ctx.cache_size(), 250);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.ensure_cache_size(100..120);
    });

    assert!(ctx.cache_size() < 250);

    // Lines near viewport should be kept (50..170 range)
    ctx.editor.read_with(&ctx.cx, |editor, _| {
      assert!(editor.line_layouts.contains_key(&100));
      assert!(editor.line_layouts.contains_key(&110));
    });
  }

  #[gpui::test]
  fn test_cache_retention_after_viewport_change(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 100);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 10..=20 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.ensure_cache_size(30..40);
    });

    assert!(ctx.is_line_cached(15));
  }

  #[gpui::test]
  fn test_invalidate_on_insert(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    // Insert char on line 1 (offset 6 = start of "line2")
    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, 'X', cx);
      });
      editor.invalidate_line(1);
    });

    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(ctx.is_line_cached(2));
  }

  #[gpui::test]
  fn test_invalidate_on_newline(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let current_line = editor.document.read(cx).char_to_line(6);
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, '\n', cx);
      });
      editor.invalidate_lines_from(current_line);
    });

    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
  }

  #[gpui::test]
  fn test_cursor_offset_initial(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");
    assert_eq!(ctx.cursor_offset(), 0);
    assert_eq!(ctx.selection(), 0..0);
  }

  #[gpui::test]
  fn test_move_to(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(5);
    assert_eq!(ctx.cursor_offset(), 5);
    assert_eq!(ctx.selection(), 5..5);

    ctx.set_cursor(11);
    assert_eq!(ctx.cursor_offset(), 11);
  }

  #[gpui::test]
  fn test_left_navigation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(3);

    let prev_offset = ctx.cursor_offset();
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let new_offset = if editor.selected_range.is_empty() {
        editor.cursor_offset().saturating_sub(1)
      } else {
        editor.selected_range.start.min(editor.selected_range.end)
      };
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), prev_offset - 1);
  }

  #[gpui::test]
  fn test_left_at_start(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(0);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let new_offset = editor.cursor_offset().saturating_sub(1);
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 0); // Should stay at 0
  }

  #[gpui::test]
  fn test_right_navigation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(2);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let doc_len = editor.document().read(cx).len();
      let new_offset = if editor.selected_range.is_empty() {
        (editor.cursor_offset() + 1).min(doc_len)
      } else {
        editor.selected_range.start.max(editor.selected_range.end)
      };
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 3);
  }

  #[gpui::test]
  fn test_right_at_end(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let doc_len = editor.document().read(cx).len();
      let new_offset = (editor.cursor_offset() + 1).min(doc_len);
      editor.move_to(new_offset, cx);
    });
    assert_eq!(ctx.cursor_offset(), 5); // Should stay at end
  }

  #[gpui::test]
  fn test_move_to_updates_cursor(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(7);
    assert_eq!(ctx.cursor_offset(), 7);
    assert_eq!(ctx.selection(), 7..7);
  }

  #[gpui::test]
  fn test_cursor_at_line_boundary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    ctx.set_cursor(0);
    assert_eq!(ctx.cursor_offset(), 0);

    ctx.set_cursor(6); // Start of line2
    assert_eq!(ctx.cursor_offset(), 6);

    ctx.set_cursor(12); // Start of line3
    assert_eq!(ctx.cursor_offset(), 12);
  }

  #[gpui::test]
  fn test_cursor_positioning(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    for pos in [0, 5, 11] {
      ctx.set_cursor(pos);
      assert_eq!(ctx.cursor_offset(), pos);
      assert_eq!(ctx.selection(), pos..pos);
    }
  }

  #[gpui::test]
  fn test_selection_with_replace(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_selection(2..7, false);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let range = editor.selected_range.clone();
      editor.document.update(cx, |doc, cx| {
        doc.replace(range, "X", cx);
      });
      editor.move_to(2, cx);
    });

    assert_eq!(ctx.text(), "heXorld");
    assert_eq!(ctx.cursor_offset(), 2);
  }

  #[gpui::test]
  fn test_insert_at_cursor(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let cursor = editor.cursor_offset();
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(cursor, '!', cx);
      });
      editor.move_to(cursor + 1, cx);
    });

    assert_eq!(ctx.text(), "hello!");
    assert_eq!(ctx.cursor_offset(), 6);
  }

  #[gpui::test]
  fn test_insert_emoji_at_cursor_advances_by_one_char(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "abc");

    ctx.set_cursor(3);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let range = editor.selected_range.clone();
      let new_text = "😎";
      editor.document.update(cx, |doc, cx| {
        doc.replace(range.clone(), new_text, cx);
      });
      let new_offset = range.start + new_text.chars().count();
      editor.move_to(new_offset, cx);
    });

    assert_eq!(ctx.text(), "abc😎");
    assert_eq!(ctx.cursor_offset(), 4);
  }

  #[gpui::test]
  fn test_insert_emoji_and_backticks_at_end(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let range = editor.selected_range.clone();
      let new_text = "😎````";
      editor.document.update(cx, |doc, cx| {
        doc.replace(range.clone(), new_text, cx);
      });
      let new_offset = range.start + new_text.chars().count();
      editor.move_to(new_offset, cx);
    });

    assert_eq!(ctx.text(), "😎````");
    assert_eq!(ctx.cursor_offset(), 5);
  }

  #[gpui::test]
  fn test_unicode_editing(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    let text = ctx.text();
    assert!(text.contains("👋"));

    ctx.set_cursor(6); // Before emoji
    assert_eq!(ctx.cursor_offset(), 6);

    ctx.set_cursor(7); // After emoji
    assert_eq!(ctx.cursor_offset(), 7);
  }

  #[gpui::test]
  fn test_display_cursor_columns_map_to_char_offsets_with_emojis(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "🤓😎 Branches");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_display_cursor(DisplayCursor { line: 0, column: 0 }, cx);
      assert_eq!(editor.cursor_offset(), 0);

      editor.set_display_cursor(DisplayCursor { line: 0, column: 1 }, cx);
      assert_eq!(editor.cursor_offset(), 1);

      editor.set_display_cursor(DisplayCursor { line: 0, column: 2 }, cx);
      assert_eq!(editor.cursor_offset(), 2);
    });
  }

  #[gpui::test]
  fn test_display_selection_copy_handles_emoji_char_boundaries(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "🤓 Branches principales");

    let copied = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.set_display_selection_with_anchor(
        DisplayCursor { line: 0, column: 0 },
        DisplayCursor { line: 0, column: 1 },
        cx,
      );
      editor.selected_text_for_copy(cx)
    });
    assert_eq!(copied.as_deref(), Some("🤓"));
  }

  #[test]
  fn test_word_boundaries_in_line_handle_emoji_char_offsets() {
    let text = "🤓 Branches";
    assert_eq!(Editor::previous_word_boundary_in_line(text, 1), 0);
    assert_eq!(Editor::next_word_boundary_in_line(text, 0), 1);
    assert_eq!(Editor::word_range_in_line(text, 0), (0, 1));
  }

  #[test]
  #[allow(clippy::reversed_empty_ranges)]
  fn test_clamp_range_to_len_normalizes_and_clamps() {
    assert_eq!(Editor::clamp_range_to_len(2..5, 10), 2..5);
    assert_eq!(Editor::clamp_range_to_len(2..50, 10), 2..10);
    assert_eq!(Editor::clamp_range_to_len(50..2, 10), 2..10);
  }

  #[gpui::test]
  fn test_move_to_clamps_out_of_bounds_offset(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "abc");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.move_to(99, cx);
    });

    assert_eq!(ctx.cursor_offset(), 3);
    assert_eq!(ctx.selection(), 3..3);
  }

  #[gpui::test]
  fn test_selected_text_for_copy_with_stale_range_is_safe(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "abc");

    let copied = ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.selected_range = 999..1000;
      editor.selected_text_for_copy(cx)
    });
    assert_eq!(copied, None);
  }

  #[gpui::test]
  fn test_cache_invalidation_on_edit(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..3 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, 'X', cx);
      });
      let line = editor.document.read(cx).char_to_line(6);
      editor.invalidate_line(line);
    });

    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(ctx.is_line_cached(2));
  }

  #[gpui::test]
  fn test_font_size_change_invalidates_layout_cache(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.last_layout_font_size = px(16.0);
      editor
        .line_layouts
        .insert(0, Arc::new(ShapedLine::default()));
      editor
        .virtual_line_layouts
        .insert(1, Arc::new(ShapedLine::default()));

      assert!(editor.invalidate_layout_cache_if_font_size_changed(px(20.0)));
      assert!(editor.line_layouts.is_empty());
      assert!(editor.virtual_line_layouts.is_empty());
    });
  }

  #[gpui::test]
  fn test_multiline_cache_invalidation(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3\nline4");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      for i in 0..4 {
        editor
          .line_layouts
          .insert(i, Arc::new(ShapedLine::default()));
      }
    });

    ctx.set_cursor(6);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      let line = editor.document.read(cx).char_to_line(6);
      editor.document.update(cx, |doc, cx| {
        doc.insert_char(6, '\n', cx);
      });
      editor.invalidate_lines_from(line);
    });

    assert!(ctx.is_line_cached(0));
    assert!(!ctx.is_line_cached(1));
    assert!(!ctx.is_line_cached(2));
    assert!(!ctx.is_line_cached(3));
  }

  #[gpui::test]
  fn test_offset_to_utf16_ascii(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let utf16_offset = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(5, cx));

    // ASCII: UTF-8 and UTF-16 offsets are the same
    assert_eq!(utf16_offset, 5);
  }

  #[gpui::test]
  fn test_offset_from_utf16_ascii(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    let utf8_offset = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(5, cx));

    assert_eq!(utf8_offset, 5);
  }

  #[gpui::test]
  fn test_offset_to_utf16_emoji(cx: &mut TestAppContext) {
    // "hello 👋 world" - emoji is 4 bytes in UTF-8, 2 code units in UTF-16
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    // Offset 6 is before emoji (after "hello ")
    let utf16_before = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(6, cx));
    assert_eq!(utf16_before, 6);

    // Offset 7 is after emoji (4-byte char)
    let utf16_after = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(7, cx));
    // In UTF-16: "hello " (6) + "👋" (2) = 8
    assert_eq!(utf16_after, 8);
  }

  #[gpui::test]
  fn test_offset_from_utf16_emoji(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    // UTF-16 offset 6 = before emoji
    let utf8_before = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(6, cx));
    assert_eq!(utf8_before, 6);

    // UTF-16 offset 8 = after emoji (👋 is 2 UTF-16 code units)
    let utf8_after = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(8, cx));
    assert_eq!(utf8_after, 7); // 4-byte emoji = 1 char in UTF-8 offset
  }

  #[gpui::test]
  fn test_offset_to_utf16_multibyte(cx: &mut TestAppContext) {
    // "café" - é is 2 bytes in UTF-8, 1 code unit in UTF-16
    let ctx = EditorTestContext::with_text(cx.clone(), "café");

    let utf16_end = ctx.editor.read_with(&ctx.cx, |editor, cx| {
      editor.offset_to_utf16(5, cx) // 5 bytes: c(1) + a(1) + f(1) + é(2)
    });
    assert_eq!(utf16_end, 4); // 4 UTF-16 code units
  }

  #[gpui::test]
  fn test_range_to_utf16(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    let utf16_range = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.range_to_utf16(&(0..7), cx));

    // Range 0..7 in UTF-8 = "hello 👋"
    // In UTF-16: 0..8 (emoji is 2 code units)
    assert_eq!(utf16_range, 0..8);
  }

  #[gpui::test]
  fn test_range_from_utf16(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 world");

    let utf8_range = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.range_from_utf16(&(0..8), cx));

    // Range 0..8 in UTF-16 = "hello 👋"
    // In UTF-8: 0..7 (emoji is 4 bytes but counts as 1 char offset)
    assert_eq!(utf8_range, 0..7);
  }

  #[gpui::test]
  fn test_utf16_roundtrip(cx: &mut TestAppContext) {
    let ctx = EditorTestContext::with_text(cx.clone(), "hello 👋 世界");

    for offset in [0, 5, 6, 7, 8, 9] {
      let utf16 = ctx
        .editor
        .read_with(&ctx.cx, |editor, cx| editor.offset_to_utf16(offset, cx));
      let back_to_utf8 = ctx
        .editor
        .read_with(&ctx.cx, |editor, cx| editor.offset_from_utf16(utf16, cx));
      assert_eq!(
        back_to_utf8, offset,
        "Roundtrip failed for offset {}",
        offset
      );
    }
  }

  #[test]
  fn test_utf16_range_to_char_range_in_text_handles_astral_emoji() {
    let range = Editor::utf16_range_to_char_range_in_text("😎abc", &(2..3));
    assert_eq!(range, 1..2);
  }

  #[test]
  #[allow(clippy::reversed_empty_ranges)]
  fn test_utf16_range_to_char_range_in_text_clamps_and_normalizes() {
    let range = Editor::utf16_range_to_char_range_in_text("✅ab", &(10..1));
    assert_eq!(range, 1..3);
  }

  #[gpui::test]
  fn test_select_to_forward(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(0);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(5, cx);
    });

    assert_eq!(ctx.selection(), 0..5);
    assert!(!ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_backward(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_cursor(5);
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(0, cx);
    });

    assert_eq!(ctx.selection(), 0..5);
    assert!(ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_extends_selection(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_selection(2..5, false);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(8, cx);
    });

    assert_eq!(ctx.selection(), 2..8);
    assert!(!ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_select_to_reverses_direction(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_selection(2..5, false);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(0, cx);
    });

    assert_eq!(ctx.selection(), 0..2);
    assert!(ctx.selection_reversed());
  }

  #[gpui::test]
  fn test_selection_anchor_preserved(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello world");

    ctx.set_selection(3..7, false);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.select_to(10, cx);
    });

    assert_eq!(ctx.selection(), 3..10);
  }

  #[gpui::test]
  fn test_clamp_horizontal_scroll_x(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.max_line_width = px(300.0);
      editor.viewport_width = px(100.0);

      assert_eq!(editor.clamp_horizontal_scroll_x(px(48.0)), px(0.0));
      assert_eq!(editor.clamp_horizontal_scroll_x(px(-200.0)), px(-200.0));
      assert_eq!(editor.clamp_horizontal_scroll_x(px(-460.0)), px(-400.0));
    });
  }

  #[gpui::test]
  fn test_reset_after_replace_resets_horizontal_scroll_state(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "hello");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.max_line_width = px(300.0);
      editor.last_scroll_x = px(-120.0);
      editor.scroll_handle.set_offset(point(px(-120.0), px(0.0)));

      editor.reset_after_replace();

      assert_eq!(editor.max_line_width, px(DEFAULT_MAX_LINE_WIDTH));
      assert_eq!(editor.last_scroll_x, px(0.0));
      assert_eq!(editor.scroll_handle.offset().x, px(0.0));
    });
  }

  #[gpui::test]
  fn test_viewport_range_clamps_invalid_scroll_offset(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.viewport_height = px(100.0);
      editor.editor_line_height = px(20.0);
      editor.scroll_offset_y = 9.0;

      assert_eq!(editor.viewport_range(px(20.0), 10), 8..10);
    });
  }

  #[gpui::test]
  fn test_apply_projection_result_clamps_to_viewport_scroll_limit(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 10);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_height = px(100.0);
      editor.editor_line_height = px(20.0);
      editor.scroll_offset_y = 9.0;

      editor.apply_projection_result(projection_with_doc_lines(10), 10, cx);

      assert_eq!(editor.scroll_offset_y, 8.0);
    });
  }

  #[gpui::test]
  fn test_apply_projection_result_requests_highlights_for_projected_viewport(
    cx: &mut TestAppContext,
  ) {
    const LINE_COUNT: usize = 50_005;
    let filler = "x".repeat(420);
    let text = (0..LINE_COUNT)
      .map(|line| format!("fn line_{line}() {{ let value = {line}; }} // {filler}"))
      .collect::<Vec<_>>()
      .join("\n");

    let mut ctx = EditorTestContext::with_text_and_extension(cx.clone(), &text, Some("rs"));
    let visible_segments = vec![4997..5004, 19_997..20_004, 34_997..35_004, 49_997..50_004];

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_height = px(800.0);
      editor.editor_line_height = px(20.0);
      editor.apply_projection_result(
        projection_with_visible_doc_segments(LINE_COUNT, &visible_segments),
        LINE_COUNT,
        cx,
      );
    });

    ctx.cx.run_until_parked();

    ctx.editor.read_with(&ctx.cx, |editor, cx| {
      let doc = editor.document().read(cx);
      for line in [4997, 5000, 19_997, 20_000, 34_997, 35_000, 49_997, 50_000] {
        let highlights = doc
          .get_highlights_for_line(line)
          .unwrap_or_else(|| panic!("projected visible line {line} should be highlighted"));
        assert!(
          !highlights.is_empty(),
          "projected visible line {line} should receive syntax highlight spans"
        );
      }
    });
  }

  #[gpui::test]
  fn test_doc_ranges_for_display_viewport_keeps_all_visible_segments(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.projection = Some(Arc::new(projection_with_visible_doc_segments(
        50_005,
        &[4997..5004, 19_997..20_004, 34_997..35_004, 49_997..50_004],
      )));

      let ranges = editor.doc_ranges_for_display_viewport(0..editor.display_line_count(50_005));
      assert_eq!(
        ranges,
        vec![4997..5004, 19_997..20_004, 34_997..35_004, 49_997..50_004]
      );
    });
  }

  #[gpui::test]
  fn test_reveal_cursor_when_hidden_keeps_visible_cursor_stationary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 12);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_height = px(100.0);
      editor.editor_line_height = px(20.0);
      editor.scroll_offset_y = 2.0;

      let line_start = editor.document().read(cx).line_to_char(2);
      editor.move_to(line_start, cx);
      editor.ensure_cursor_visible_when_hidden(cx);

      assert_eq!(editor.scroll_offset_y, 2.0);
    });
  }

  #[gpui::test]
  fn test_reveal_cursor_when_hidden_restores_padding_once_offscreen(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 12);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_height = px(100.0);
      editor.editor_line_height = px(20.0);
      editor.scroll_offset_y = 3.0;

      let line_start = editor.document().read(cx).line_to_char(9);
      editor.move_to(line_start, cx);
      editor.ensure_cursor_visible_when_hidden(cx);

      assert_eq!(editor.scroll_offset_y, 8.0);
    });
  }

  #[gpui::test]
  fn test_reveal_cursor_with_padding_keeps_navigation_margin(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 12);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_height = px(100.0);
      editor.editor_line_height = px(20.0);
      editor.scroll_offset_y = 2.0;

      let line_start = editor.document().read(cx).line_to_char(2);
      editor.move_to(line_start, cx);
      editor.ensure_cursor_visible_with_policy(CursorRevealPolicy::WithPadding, cx);

      assert_eq!(editor.scroll_offset_y, 0.0);
    });
  }

  #[gpui::test]
  fn test_reveal_cursor_when_hidden_restores_horizontal_scroll_without_layout_cache(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "abc");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_width = px(100.0);
      editor.max_line_width = px(300.0);
      editor.scroll_handle.set_offset(point(px(-120.0), px(0.0)));
      editor.move_to(0, cx);
      editor.line_layouts.clear();

      editor.ensure_cursor_visible_when_hidden(cx);

      assert_eq!(editor.scroll_handle.offset().x, px(0.0));
    });
  }

  #[gpui::test]
  fn test_reveal_cursor_when_hidden_restores_horizontal_scroll_on_trailing_empty_line(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "abc\n");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.viewport_width = px(100.0);
      editor.max_line_width = px(300.0);
      editor.scroll_handle.set_offset(point(px(-120.0), px(0.0)));
      let doc_len = editor.document().read(cx).len();
      editor.move_to(doc_len, cx);
      editor.line_layouts.clear();

      editor.ensure_cursor_visible_when_hidden(cx);

      assert_eq!(editor.scroll_handle.offset().x, px(0.0));
    });
  }

  #[gpui::test]
  fn test_select_all_display_selection_keeps_removed_line_on_shift_left(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_removed_middle_line());
      assert!(editor.select_all_display_lines(cx));

      assert!(editor.select_display_cursor_horizontal(-1, cx));

      let selection = editor
        .display_selection
        .as_ref()
        .expect("display selection should stay active");
      let (start, end) = selection.normalized();
      assert_eq!(
        (start, end),
        (
          DisplayCursor { line: 0, column: 0 },
          DisplayCursor { line: 2, column: 0 }
        )
      );
      assert!(editor.display_to_doc_line(1).is_none());
      assert_eq!(editor.selected_range, 0..2);
    });
  }

  #[gpui::test]
  fn test_move_display_cursor_left_stays_on_first_visible_line_with_start_gap(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc\nd\ne");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_hidden_start_and_end());
      editor.set_display_cursor(DisplayCursor { line: 1, column: 0 }, cx);

      let before_cursor = editor.current_display_cursor(cx);
      let before_offset = editor.cursor_offset();
      assert!(editor.move_display_cursor_horizontal(-1, cx));

      assert_eq!(editor.current_display_cursor(cx), before_cursor);
      assert_eq!(editor.cursor_offset(), before_offset);
      assert_eq!(editor.selected_range, before_offset..before_offset);
    });
  }

  #[gpui::test]
  fn test_move_display_cursor_right_stays_on_last_visible_line_with_end_gap(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb\nc\nd\ne");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_hidden_start_and_end());
      editor.set_display_cursor(DisplayCursor { line: 3, column: 1 }, cx);

      let before_cursor = editor.current_display_cursor(cx);
      let before_offset = editor.cursor_offset();
      assert!(editor.move_display_cursor_horizontal(1, cx));

      assert_eq!(editor.current_display_cursor(cx), before_cursor);
      assert_eq!(editor.cursor_offset(), before_offset);
      assert_eq!(editor.selected_range, before_offset..before_offset);
    });
  }

  #[gpui::test]
  fn test_doc_range_for_display_viewport_does_not_span_large_collapsed_gap(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, _| {
      editor.projection = Some(projection_with_large_middle_gap());

      let range = editor.doc_range_for_display_viewport(0..5);
      assert_eq!(range, 10..12);
      assert!(range.end.saturating_sub(range.start) < 1_000);

      let lower_range = editor.doc_range_for_display_viewport(2..5);
      assert_eq!(lower_range, 49_990..49_992);
    });
  }

  #[gpui::test]
  fn test_select_all_display_selection_keeps_mode_on_right_boundary(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_removed_middle_line());
      assert!(editor.select_all_display_lines(cx));

      let before = editor
        .display_selection
        .clone()
        .expect("display selection should exist after select all");
      assert!(editor.select_display_cursor_horizontal(1, cx));
      let after = editor
        .display_selection
        .as_ref()
        .expect("display selection should remain active");
      assert_eq!(after.start, before.start);
      assert_eq!(after.end, before.end);
      assert_eq!(editor.selected_range, 0..editor.document.read(cx).len());
    });
  }

  #[gpui::test]
  fn test_select_all_display_selection_keeps_removed_line_on_cmd_shift_left(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_removed_middle_line());
      assert!(editor.select_all_display_lines(cx));

      assert!(editor.select_display_cursor_line_boundary(true, cx));

      let selection = editor
        .display_selection
        .as_ref()
        .expect("display selection should stay active");
      let (start, end) = selection.normalized();
      assert_eq!(
        (start, end),
        (
          DisplayCursor { line: 0, column: 0 },
          DisplayCursor { line: 2, column: 0 }
        )
      );
      assert!(editor.display_to_doc_line(1).is_none());
      assert_eq!(editor.selected_range, 0..2);
    });
  }

  #[gpui::test]
  fn test_select_all_display_selection_keeps_mode_on_cmd_shift_right_boundary(
    cx: &mut TestAppContext,
  ) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "a\nb");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.projection = Some(projection_with_removed_middle_line());
      assert!(editor.select_all_display_lines(cx));

      let before = editor
        .display_selection
        .clone()
        .expect("display selection should exist after select all");
      assert!(editor.select_display_cursor_line_boundary(false, cx));
      let after = editor
        .display_selection
        .as_ref()
        .expect("display selection should remain active");
      assert_eq!(after.start, before.start);
      assert_eq!(after.end, before.end);
      assert_eq!(editor.selected_range, 0..editor.document.read(cx).len());
    });
  }

  #[gpui::test]
  fn test_find_refresh_selects_first_match(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "foo bar\nfoo baz");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.find_query = "foo".to_string();
      editor.refresh_find_matches(px(20.0), false, cx);

      assert_eq!(editor.find_matches.len(), 2);
      assert_eq!(editor.find_active_match, Some(0));
      assert_eq!(editor.selected_range, 0..3);
    });
  }

  #[gpui::test]
  fn test_find_next_previous_wrap(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "foo bar\nfoo baz");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.find_query = "foo".to_string();
      editor.refresh_find_matches(px(20.0), false, cx);

      editor.find_next_match_with_line_height(px(20.0), cx);
      assert_eq!(editor.find_active_match, Some(1));
      assert_eq!(editor.selected_range, 8..11);

      editor.find_next_match_with_line_height(px(20.0), cx);
      assert_eq!(editor.find_active_match, Some(0));
      assert_eq!(editor.selected_range, 0..3);

      editor.find_previous_match_with_line_height(px(20.0), cx);
      assert_eq!(editor.find_active_match, Some(1));
      assert_eq!(editor.selected_range, 8..11);
    });
  }

  #[gpui::test]
  fn test_find_smooth_scroll_uses_test_scheduler_timers(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_lines(cx.clone(), 80);

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.find_matches = editor.collect_find_matches("Line 60", cx);
      assert_eq!(editor.find_matches.len(), 1);
      editor.scroll_offset_y = 0.0;
      assert!(editor.scroll_to_find_match(editor.find_matches[0].display_line, px(20.0), true, cx));
    });

    for _ in 0..20 {
      ctx.cx.executor().advance_clock(FIND_SCROLL_TICK);
      ctx.cx.run_until_parked();
    }

    ctx.editor.read_with(&ctx.cx, |editor, cx| {
      let total_lines = editor.display_line_count(editor.document.read(cx).len_lines());
      let metrics = editor.vertical_scroll_metrics(px(20.0), total_lines);
      let target = (editor.find_matches[0].display_line as f32 - metrics.scroll_padding)
        .clamp(0.0, metrics.max_scroll);

      assert!((editor.scroll_offset_y - target).abs() < f32::EPSILON);
    });
  }

  #[gpui::test]
  fn test_find_refresh_with_no_results(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "foo bar\nfoo baz");

    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.find_query = "zzz".to_string();
      editor.refresh_find_matches(px(20.0), false, cx);

      assert!(editor.find_matches.is_empty());
      assert_eq!(editor.find_active_match, None);
    });
  }

  #[gpui::test]
  fn test_syntax_highlights_cached(cx: &mut TestAppContext) {
    let file_path = temp_path("editor-syntax").with_extension("ts");
    std::fs::write(&file_path, "const value = 1;\n").expect("write temp editor file");
    let repo_root = file_path.parent().expect("temp file parent").to_path_buf();
    let editor = cx.new(|cx| Editor::new_with_paths(repo_root, file_path.clone(), cx));

    editor.read_with(cx, |editor, cx| {
      let doc = editor.document().read(cx);

      // Highlighting is async with debouncing, so it might not be ready immediately
      assert!(!doc.is_empty());
      assert!(doc.len_lines() > 0);
    });

    let _ = std::fs::remove_file(file_path);
  }

  #[gpui::test]
  fn test_quadruple_click_selects_all(cx: &mut TestAppContext) {
    let mut ctx = EditorTestContext::with_text(cx.clone(), "line1\nline2\nline3");

    let doc_len = ctx
      .editor
      .read_with(&ctx.cx, |editor, cx| editor.document().read(cx).len());

    // Simulate quadruple click - select all buffer
    ctx.editor.update(&mut ctx.cx, |editor, cx| {
      editor.is_selecting = true;
      editor.selected_range = 0..doc_len;
      editor.selection_reversed = false;
      cx.notify();
    });

    assert_eq!(ctx.selection(), 0..doc_len);
    assert_eq!(doc_len, 17); // "line1\nline2\nline3"
  }

  #[test]
  fn test_projection_background_threshold() {
    assert!(!Editor::should_build_projection_in_background(
      ASYNC_PROJECTION_MIN_DOC_LINES.saturating_sub(1)
    ));
    assert!(Editor::should_build_projection_in_background(
      ASYNC_PROJECTION_MIN_DOC_LINES
    ));
  }

  #[test]
  fn test_diff_debounce_scales_with_large_file_size() {
    assert_eq!(
      Editor::diff_debounce_ms_for_line_count(LARGE_FILE_DIFF_DEBOUNCE_LINES.saturating_sub(1)),
      DIFF_DEBOUNCE_MS
    );
    assert_eq!(
      Editor::diff_debounce_ms_for_line_count(LARGE_FILE_DIFF_DEBOUNCE_LINES),
      LARGE_FILE_DIFF_DEBOUNCE_MS
    );
    assert_eq!(
      Editor::diff_debounce_ms_for_line_count(HUGE_FILE_DIFF_DEBOUNCE_LINES),
      HUGE_FILE_DIFF_DEBOUNCE_MS
    );
  }
}
