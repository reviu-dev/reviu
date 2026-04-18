use std::{collections::HashMap, ops::Range};

use gpui::{
  App, AppContext as _, Context, Empty, Entity, EntityInputHandler as _, Focusable as _, Global,
  Half as _, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
  RenderOnce, SharedString, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
  ActiveTheme as _, Sizable, Size, h_flex,
  input::{self, Input, InputState},
};

const MAX_EMOJI_COMPLETIONS: usize = 8;
const MAX_EMOJI_QUERY_LEN: usize = 64;
const EMOJI_MENU_WIDTH_PX: f32 = 300.0;
const EMOJI_MENU_MAX_HEIGHT_PX: f32 = 220.0;
const EMOJI_MENU_LINE_HEIGHT_PX: f32 = 20.0;
const EMOJI_MENU_TOP_OFFSET_PX: f32 = 30.0;
const EMOJI_MENU_LEFT_OFFSET_PX: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GithubEmoji {
  name: &'static str,
  emoji: &'static str,
  aliases: &'static [&'static str],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EmojiShortcodeTrigger {
  range: Range<usize>,
  query: String,
}

#[derive(Clone)]
struct EmojiCompletionSnapshot {
  trigger: EmojiShortcodeTrigger,
  items: Vec<&'static GithubEmoji>,
}

fn emoji_completion_label(emoji: &GithubEmoji) -> String {
  format!("{} {}", emoji.emoji, emoji.name.replace('_', " "))
}

#[derive(IntoElement)]
pub struct GithubEmojiInput {
  input: Input,
  input_state: Entity<InputState>,
}

impl GithubEmojiInput {
  pub fn new(input_state: &Entity<InputState>) -> Self {
    Self {
      input: Input::new(input_state),
      input_state: input_state.clone(),
    }
  }

  pub fn disabled(mut self, disabled: bool) -> Self {
    self.input = self.input.disabled(disabled);
    self
  }

  pub fn h(mut self, height: impl Into<gpui::DefiniteLength>) -> Self {
    self.input = self.input.h(height);
    self
  }
}

impl Styled for GithubEmojiInput {
  fn style(&mut self) -> &mut StyleRefinement {
    self.input.style()
  }
}

impl Sizable for GithubEmojiInput {
  fn with_size(mut self, size: impl Into<Size>) -> Self {
    self.input = self.input.with_size(size);
    self
  }
}

impl RenderOnce for GithubEmojiInput {
  fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
    let overlay = github_emoji_overlay_for_input(&self.input_state, cx);
    let overlay_for_actions = overlay.clone();

    div()
      .id(("github-emoji-input", self.input_state.entity_id()))
      .relative()
      .w_full()
      .capture_action({
        let overlay = overlay_for_actions.clone();
        move |action: &input::Enter, window, cx| {
          overlay.update(cx, |overlay, cx| {
            overlay.on_action_enter(action, window, cx);
          });
        }
      })
      .capture_action({
        let overlay = overlay_for_actions.clone();
        move |action: &input::MoveUp, window, cx| {
          overlay.update(cx, |overlay, cx| {
            overlay.on_action_move_up(action, window, cx);
          });
        }
      })
      .capture_action({
        let overlay = overlay_for_actions.clone();
        move |action: &input::MoveDown, window, cx| {
          overlay.update(cx, |overlay, cx| {
            overlay.on_action_move_down(action, window, cx);
          });
        }
      })
      .capture_action(move |action: &input::Escape, window, cx| {
        overlay_for_actions.update(cx, |overlay, cx| {
          overlay.on_action_escape(action, window, cx);
        });
      })
      .child(self.input)
      .child(overlay)
  }
}

#[derive(Default)]
struct GithubEmojiOverlayRegistry {
  overlays: HashMap<String, Entity<GithubEmojiCompletionOverlay>>,
}

impl Global for GithubEmojiOverlayRegistry {}

fn github_emoji_overlay_for_input(
  input_state: &Entity<InputState>,
  cx: &mut App,
) -> Entity<GithubEmojiCompletionOverlay> {
  if !cx.has_global::<GithubEmojiOverlayRegistry>() {
    cx.set_global(GithubEmojiOverlayRegistry::default());
  }

  let key = format!("{:?}", input_state.entity_id());
  if let Some(overlay) = cx
    .global::<GithubEmojiOverlayRegistry>()
    .overlays
    .get(&key)
    .cloned()
  {
    return overlay;
  }

  let input = input_state.clone();
  let overlay = cx.new(|cx| GithubEmojiCompletionOverlay::new(input, cx));
  cx.global_mut::<GithubEmojiOverlayRegistry>()
    .overlays
    .insert(key, overlay.clone());
  overlay
}

struct GithubEmojiCompletionOverlay {
  input: Entity<InputState>,
  selected_ix: usize,
  dismissed_trigger: Option<EmojiShortcodeTrigger>,
  _subscriptions: Vec<gpui::Subscription>,
}

impl GithubEmojiCompletionOverlay {
  fn new(input: Entity<InputState>, cx: &mut Context<Self>) -> Self {
    let _subscriptions = vec![cx.observe(&input, |this, _, cx| {
      this.sync_selection(cx);
    })];

    Self {
      input,
      selected_ix: 0,
      dismissed_trigger: None,
      _subscriptions,
    }
  }

  fn sync_selection(&mut self, cx: &mut Context<Self>) {
    if let Some(snapshot) = self.snapshot(cx) {
      if self.selected_ix >= snapshot.items.len() {
        self.selected_ix = 0;
      }
    } else {
      self.selected_ix = 0;
      self.dismissed_trigger = None;
    }
    cx.notify();
  }

  fn snapshot(&self, cx: &App) -> Option<EmojiCompletionSnapshot> {
    let input = self.input.read(cx);
    let text = input.value();
    let trigger = emoji_shortcode_trigger_at_cursor(text.as_ref(), input.cursor())?;
    if self
      .dismissed_trigger
      .as_ref()
      .is_some_and(|dismissed| dismissed == &trigger)
    {
      return None;
    }

    let items = matching_emojis(trigger.query.as_str())
      .into_iter()
      .take(MAX_EMOJI_COMPLETIONS)
      .collect::<Vec<_>>();
    if items.is_empty() {
      return None;
    }

    Some(EmojiCompletionSnapshot { trigger, items })
  }

  fn on_action_enter(
    &mut self,
    action: &input::Enter,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if action.secondary {
      return;
    }

    let Some(snapshot) = self.snapshot(cx) else {
      return;
    };

    self.insert_selected(&snapshot, window, cx);
    cx.stop_propagation();
  }

  fn on_action_move_up(&mut self, _: &input::MoveUp, _: &mut Window, cx: &mut Context<Self>) {
    let Some(snapshot) = self.snapshot(cx) else {
      return;
    };

    self.selected_ix = previous_selection_index(self.selected_ix, snapshot.items.len());
    cx.stop_propagation();
    cx.notify();
  }

  fn on_action_move_down(&mut self, _: &input::MoveDown, _: &mut Window, cx: &mut Context<Self>) {
    let Some(snapshot) = self.snapshot(cx) else {
      return;
    };

    self.selected_ix = next_selection_index(self.selected_ix, snapshot.items.len());
    cx.stop_propagation();
    cx.notify();
  }

  fn on_action_escape(&mut self, _: &input::Escape, _: &mut Window, cx: &mut Context<Self>) {
    let Some(snapshot) = self.snapshot(cx) else {
      return;
    };

    self.dismissed_trigger = Some(snapshot.trigger);
    cx.stop_propagation();
    cx.notify();
  }

  fn insert_item_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(snapshot) = self.snapshot(cx) else {
      return;
    };
    self.selected_ix = ix.min(snapshot.items.len().saturating_sub(1));
    self.insert_selected(&snapshot, window, cx);
  }

  fn insert_selected(
    &mut self,
    snapshot: &EmojiCompletionSnapshot,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(emoji) = snapshot.items.get(self.selected_ix).copied() else {
      return;
    };

    let text = self.input.read(cx).value();
    let replace_range = byte_range_to_utf16_range(text.as_ref(), snapshot.trigger.range.clone());
    self.input.update(cx, |input, cx| {
      input.replace_text_in_range(Some(replace_range), emoji.emoji, window, cx);
      input.focus(window, cx);
    });

    self.selected_ix = 0;
    self.dismissed_trigger = None;
    cx.notify();
  }
}

impl Render for GithubEmojiCompletionOverlay {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let (is_focused, cursor_position) = {
      let input = self.input.read(cx);
      (
        input.focus_handle(cx).is_focused(window),
        input.cursor_position(),
      )
    };
    if !is_focused {
      return Empty.into_any_element();
    }

    let Some(snapshot) = self.snapshot(cx) else {
      return Empty.into_any_element();
    };

    let selected_ix = self.selected_ix.min(snapshot.items.len().saturating_sub(1));
    let top = EMOJI_MENU_TOP_OFFSET_PX + cursor_position.line as f32 * EMOJI_MENU_LINE_HEIGHT_PX;
    let overlay = cx.entity();

    div()
      .id(("github-emoji-completion-menu", self.input.entity_id()))
      .absolute()
      .left(px(EMOJI_MENU_LEFT_OFFSET_PX))
      .top(px(top))
      .w(px(EMOJI_MENU_WIDTH_PX))
      .max_h(px(EMOJI_MENU_MAX_HEIGHT_PX))
      .overflow_hidden()
      .occlude()
      .bg(cx.theme().popover)
      .text_color(cx.theme().popover_foreground)
      .border_1()
      .border_color(cx.theme().border)
      .rounded(cx.theme().radius)
      .shadow_lg()
      .p_1()
      .children(snapshot.items.into_iter().enumerate().map(|(ix, emoji)| {
        let selected = ix == selected_ix;
        let overlay_for_click = overlay.clone();
        let overlay_for_hover = overlay.clone();
        h_flex()
          .id(("github-emoji-completion-item", ix))
          .w_full()
          .items_center()
          .gap_2()
          .px_2()
          .py_1()
          .rounded(cx.theme().radius.half())
          .text_xs()
          .line_height(gpui::relative(1.1))
          .cursor_pointer()
          .when(selected, |this| {
            this
              .bg(cx.theme().accent)
              .text_color(cx.theme().accent_foreground)
          })
          .hover(|this| this.bg(cx.theme().accent.opacity(0.8)))
          .on_mouse_move(move |_, _, cx| {
            overlay_for_hover.update(cx, |overlay, cx| {
              if overlay.selected_ix != ix {
                overlay.selected_ix = ix;
                cx.notify();
              }
            });
          })
          .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            overlay_for_click.update(cx, |overlay, cx| {
              overlay.insert_item_at(ix, window, cx);
              cx.stop_propagation();
            });
          })
          .child(SharedString::from(emoji_completion_label(emoji)))
      }))
      .into_any_element()
  }
}

fn byte_range_to_utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
  byte_offset_to_utf16_offset(text, range.start)..byte_offset_to_utf16_offset(text, range.end)
}

fn byte_offset_to_utf16_offset(text: &str, offset: usize) -> usize {
  text[..offset].chars().map(char::len_utf16).sum()
}

fn emoji_shortcode_trigger_at_cursor(text: &str, cursor: usize) -> Option<EmojiShortcodeTrigger> {
  if cursor > text.len() || !text.is_char_boundary(cursor) {
    return None;
  }

  let before_cursor = &text[..cursor];
  for (ix, ch) in before_cursor.char_indices().rev() {
    if ch == ':' {
      let query = &text[ix + ch.len_utf8()..cursor];
      if query.len() > MAX_EMOJI_QUERY_LEN || !query.chars().all(is_shortcode_query_char) {
        return None;
      }

      let previous = before_cursor[..ix].chars().next_back();
      if !is_shortcode_boundary(previous) {
        return None;
      }

      return Some(EmojiShortcodeTrigger {
        range: ix..cursor,
        query: query.to_ascii_lowercase(),
      });
    }

    if !is_shortcode_query_char(ch) {
      return None;
    }
  }

  None
}

fn is_shortcode_query_char(ch: char) -> bool {
  ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-')
}

fn is_shortcode_boundary(ch: Option<char>) -> bool {
  ch.is_none_or(|ch| !is_shortcode_query_char(ch))
}

fn previous_selection_index(selected_ix: usize, items_len: usize) -> usize {
  if items_len == 0 {
    return 0;
  }

  if selected_ix == 0 {
    items_len - 1
  } else {
    selected_ix - 1
  }
}

fn next_selection_index(selected_ix: usize, items_len: usize) -> usize {
  if items_len == 0 {
    return 0;
  }

  (selected_ix + 1) % items_len
}

fn matching_emojis(query: &str) -> Vec<&'static GithubEmoji> {
  let query = query.to_ascii_lowercase();
  let mut matches = GITHUB_EMOJIS
    .iter()
    .filter_map(|emoji| emoji_match_score(emoji, query.as_str()).map(|score| (score, emoji)))
    .collect::<Vec<_>>();

  matches.sort_by(|(left_score, left), (right_score, right)| {
    left_score
      .cmp(right_score)
      .then_with(|| left.name.cmp(right.name))
  });

  matches
    .into_iter()
    .map(|(_, emoji)| emoji)
    .collect::<Vec<_>>()
}

fn emoji_match_score(emoji: &GithubEmoji, query: &str) -> Option<u8> {
  if query.is_empty() {
    return Some(0);
  }

  let candidates = std::iter::once(emoji.name).chain(emoji.aliases.iter().copied());
  let mut best_score = None;
  for candidate in candidates {
    let candidate = candidate.to_ascii_lowercase();
    let score = if candidate == query {
      0
    } else if candidate.starts_with(query) {
      1
    } else if candidate.contains(query) {
      2
    } else {
      continue;
    };
    best_score = Some(best_score.map_or(score, |best: u8| best.min(score)));
  }

  best_score
}

const GITHUB_EMOJIS: &[GithubEmoji] = &[
  GithubEmoji {
    name: "+1",
    emoji: "👍",
    aliases: &["thumbsup"],
  },
  GithubEmoji {
    name: "-1",
    emoji: "👎",
    aliases: &["thumbsdown"],
  },
  GithubEmoji {
    name: "100",
    emoji: "💯",
    aliases: &[],
  },
  GithubEmoji {
    name: "accessibility",
    emoji: "♿",
    aliases: &[],
  },
  GithubEmoji {
    name: "airplane",
    emoji: "✈️",
    aliases: &[],
  },
  GithubEmoji {
    name: "alarm_clock",
    emoji: "⏰",
    aliases: &[],
  },
  GithubEmoji {
    name: "alien",
    emoji: "👽",
    aliases: &[],
  },
  GithubEmoji {
    name: "ambulance",
    emoji: "🚑",
    aliases: &[],
  },
  GithubEmoji {
    name: "anchor",
    emoji: "⚓",
    aliases: &[],
  },
  GithubEmoji {
    name: "apple",
    emoji: "🍎",
    aliases: &[],
  },
  GithubEmoji {
    name: "arrow_down",
    emoji: "⬇️",
    aliases: &[],
  },
  GithubEmoji {
    name: "arrow_left",
    emoji: "⬅️",
    aliases: &[],
  },
  GithubEmoji {
    name: "arrow_right",
    emoji: "➡️",
    aliases: &[],
  },
  GithubEmoji {
    name: "arrow_up",
    emoji: "⬆️",
    aliases: &[],
  },
  GithubEmoji {
    name: "art",
    emoji: "🎨",
    aliases: &[],
  },
  GithubEmoji {
    name: "baby",
    emoji: "👶",
    aliases: &[],
  },
  GithubEmoji {
    name: "balloon",
    emoji: "🎈",
    aliases: &[],
  },
  GithubEmoji {
    name: "beers",
    emoji: "🍻",
    aliases: &[],
  },
  GithubEmoji {
    name: "bell",
    emoji: "🔔",
    aliases: &[],
  },
  GithubEmoji {
    name: "bento",
    emoji: "🍱",
    aliases: &[],
  },
  GithubEmoji {
    name: "bike",
    emoji: "🚲",
    aliases: &[],
  },
  GithubEmoji {
    name: "blossom",
    emoji: "🌼",
    aliases: &[],
  },
  GithubEmoji {
    name: "bomb",
    emoji: "💣",
    aliases: &[],
  },
  GithubEmoji {
    name: "boom",
    emoji: "💥",
    aliases: &["collision"],
  },
  GithubEmoji {
    name: "bow",
    emoji: "🙇",
    aliases: &[],
  },
  GithubEmoji {
    name: "bowtie",
    emoji: "🎀",
    aliases: &[],
  },
  GithubEmoji {
    name: "bug",
    emoji: "🐛",
    aliases: &[],
  },
  GithubEmoji {
    name: "bulb",
    emoji: "💡",
    aliases: &[],
  },
  GithubEmoji {
    name: "business_suit_levitating",
    emoji: "🕴️",
    aliases: &[],
  },
  GithubEmoji {
    name: "bus",
    emoji: "🚌",
    aliases: &[],
  },
  GithubEmoji {
    name: "busstop",
    emoji: "🚏",
    aliases: &[],
  },
  GithubEmoji {
    name: "calendar",
    emoji: "📆",
    aliases: &[],
  },
  GithubEmoji {
    name: "camera",
    emoji: "📷",
    aliases: &[],
  },
  GithubEmoji {
    name: "car",
    emoji: "🚗",
    aliases: &[],
  },
  GithubEmoji {
    name: "chart_with_upwards_trend",
    emoji: "📈",
    aliases: &[],
  },
  GithubEmoji {
    name: "checkered_flag",
    emoji: "🏁",
    aliases: &[],
  },
  GithubEmoji {
    name: "cherry_blossom",
    emoji: "🌸",
    aliases: &[],
  },
  GithubEmoji {
    name: "clap",
    emoji: "👏",
    aliases: &[],
  },
  GithubEmoji {
    name: "classical_building",
    emoji: "🏛️",
    aliases: &[],
  },
  GithubEmoji {
    name: "closed_lock_with_key",
    emoji: "🔐",
    aliases: &[],
  },
  GithubEmoji {
    name: "coffee",
    emoji: "☕",
    aliases: &[],
  },
  GithubEmoji {
    name: "computer",
    emoji: "💻",
    aliases: &[],
  },
  GithubEmoji {
    name: "construction",
    emoji: "🚧",
    aliases: &[],
  },
  GithubEmoji {
    name: "construction_worker",
    emoji: "👷",
    aliases: &[],
  },
  GithubEmoji {
    name: "cookie",
    emoji: "🍪",
    aliases: &[],
  },
  GithubEmoji {
    name: "crossed_fingers",
    emoji: "🤞",
    aliases: &[],
  },
  GithubEmoji {
    name: "cry",
    emoji: "😢",
    aliases: &[],
  },
  GithubEmoji {
    name: "dart",
    emoji: "🎯",
    aliases: &[],
  },
  GithubEmoji {
    name: "dash",
    emoji: "💨",
    aliases: &[],
  },
  GithubEmoji {
    name: "dog",
    emoji: "🐶",
    aliases: &[],
  },
  GithubEmoji {
    name: "dolphin",
    emoji: "🐬",
    aliases: &[],
  },
  GithubEmoji {
    name: "eyes",
    emoji: "👀",
    aliases: &[],
  },
  GithubEmoji {
    name: "facepalm",
    emoji: "🤦",
    aliases: &[],
  },
  GithubEmoji {
    name: "fire",
    emoji: "🔥",
    aliases: &[],
  },
  GithubEmoji {
    name: "fish",
    emoji: "🐟",
    aliases: &[],
  },
  GithubEmoji {
    name: "gift",
    emoji: "🎁",
    aliases: &[],
  },
  GithubEmoji {
    name: "globe_with_meridians",
    emoji: "🌐",
    aliases: &[],
  },
  GithubEmoji {
    name: "green_heart",
    emoji: "💚",
    aliases: &[],
  },
  GithubEmoji {
    name: "hammer",
    emoji: "🔨",
    aliases: &[],
  },
  GithubEmoji {
    name: "handshake",
    emoji: "🤝",
    aliases: &[],
  },
  GithubEmoji {
    name: "heart",
    emoji: "❤️",
    aliases: &[],
  },
  GithubEmoji {
    name: "heavy_check_mark",
    emoji: "✔️",
    aliases: &["check"],
  },
  GithubEmoji {
    name: "hourglass",
    emoji: "⌛",
    aliases: &[],
  },
  GithubEmoji {
    name: "joy",
    emoji: "😂",
    aliases: &[],
  },
  GithubEmoji {
    name: "key",
    emoji: "🔑",
    aliases: &[],
  },
  GithubEmoji {
    name: "label",
    emoji: "🏷️",
    aliases: &[],
  },
  GithubEmoji {
    name: "laughing",
    emoji: "😆",
    aliases: &["satisfied"],
  },
  GithubEmoji {
    name: "lock",
    emoji: "🔒",
    aliases: &[],
  },
  GithubEmoji {
    name: "mag",
    emoji: "🔍",
    aliases: &[],
  },
  GithubEmoji {
    name: "memo",
    emoji: "📝",
    aliases: &["pencil"],
  },
  GithubEmoji {
    name: "muscle",
    emoji: "💪",
    aliases: &[],
  },
  GithubEmoji {
    name: "no_entry",
    emoji: "⛔",
    aliases: &[],
  },
  GithubEmoji {
    name: "octocat",
    emoji: "🐙",
    aliases: &[],
  },
  GithubEmoji {
    name: "ok_hand",
    emoji: "👌",
    aliases: &[],
  },
  GithubEmoji {
    name: "package",
    emoji: "📦",
    aliases: &[],
  },
  GithubEmoji {
    name: "partying_face",
    emoji: "🥳",
    aliases: &[],
  },
  GithubEmoji {
    name: "pencil2",
    emoji: "✏️",
    aliases: &[],
  },
  GithubEmoji {
    name: "pray",
    emoji: "🙏",
    aliases: &[],
  },
  GithubEmoji {
    name: "pushpin",
    emoji: "📌",
    aliases: &[],
  },
  GithubEmoji {
    name: "question",
    emoji: "❓",
    aliases: &[],
  },
  GithubEmoji {
    name: "recycle",
    emoji: "♻️",
    aliases: &[],
  },
  GithubEmoji {
    name: "rocket",
    emoji: "🚀",
    aliases: &[],
  },
  GithubEmoji {
    name: "rotating_light",
    emoji: "🚨",
    aliases: &[],
  },
  GithubEmoji {
    name: "see_no_evil",
    emoji: "🙈",
    aliases: &[],
  },
  GithubEmoji {
    name: "seedling",
    emoji: "🌱",
    aliases: &[],
  },
  GithubEmoji {
    name: "ship",
    emoji: "🚢",
    aliases: &[],
  },
  GithubEmoji {
    name: "sparkles",
    emoji: "✨",
    aliases: &[],
  },
  GithubEmoji {
    name: "speech_balloon",
    emoji: "💬",
    aliases: &[],
  },
  GithubEmoji {
    name: "star",
    emoji: "⭐",
    aliases: &[],
  },
  GithubEmoji {
    name: "tada",
    emoji: "🎉",
    aliases: &[],
  },
  GithubEmoji {
    name: "test_tube",
    emoji: "🧪",
    aliases: &[],
  },
  GithubEmoji {
    name: "thinking",
    emoji: "🤔",
    aliases: &[],
  },
  GithubEmoji {
    name: "thread",
    emoji: "🧵",
    aliases: &[],
  },
  GithubEmoji {
    name: "trophy",
    emoji: "🏆",
    aliases: &[],
  },
  GithubEmoji {
    name: "truck",
    emoji: "🚚",
    aliases: &[],
  },
  GithubEmoji {
    name: "unlock",
    emoji: "🔓",
    aliases: &[],
  },
  GithubEmoji {
    name: "warning",
    emoji: "⚠️",
    aliases: &[],
  },
  GithubEmoji {
    name: "wave",
    emoji: "👋",
    aliases: &[],
  },
  GithubEmoji {
    name: "white_check_mark",
    emoji: "✅",
    aliases: &[],
  },
  GithubEmoji {
    name: "wrench",
    emoji: "🔧",
    aliases: &[],
  },
  GithubEmoji {
    name: "x",
    emoji: "❌",
    aliases: &[],
  },
  GithubEmoji {
    name: "zap",
    emoji: "⚡",
    aliases: &[],
  },
];

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn emoji_shortcode_trigger_detects_query_before_cursor() {
    let text = "Looks good :rock";

    let trigger = emoji_shortcode_trigger_at_cursor(text, text.len()).expect("trigger");

    assert_eq!(trigger.range, 11..16);
    assert_eq!(trigger.query, "rock");
  }

  #[test]
  fn emoji_shortcode_trigger_requires_boundary_before_colon() {
    assert!(emoji_shortcode_trigger_at_cursor("http://github.com", 5).is_none());
    assert!(emoji_shortcode_trigger_at_cursor("word:smile", "word:smile".len()).is_none());
  }

  #[test]
  fn matching_emojis_uses_substring_search_like_github() {
    let names = matching_emojis("ss")
      .into_iter()
      .take(5)
      .map(|emoji| emoji.name)
      .collect::<Vec<_>>();

    assert_eq!(
      names,
      vec![
        "accessibility",
        "blossom",
        "business_suit_levitating",
        "busstop",
        "cherry_blossom"
      ]
    );
  }

  #[test]
  fn byte_range_to_utf16_range_handles_text_before_shortcode() {
    let text = "🚀 Ship :rocket";
    let trigger = emoji_shortcode_trigger_at_cursor(text, text.len()).expect("trigger");

    let range = byte_range_to_utf16_range(text, trigger.range);

    assert_eq!(range, 8..15);
  }

  #[test]
  fn emoji_completion_label_keeps_long_names_readable() {
    let emoji = matching_emojis("chart_with_up")
      .into_iter()
      .next()
      .expect("emoji");

    let label = emoji_completion_label(emoji);

    assert_eq!(label, "📈 chart with upwards trend");
  }

  #[test]
  fn selection_navigation_wraps_at_menu_edges() {
    assert_eq!(previous_selection_index(0, 8), 7);
    assert_eq!(previous_selection_index(3, 8), 2);
    assert_eq!(next_selection_index(7, 8), 0);
    assert_eq!(next_selection_index(3, 8), 4);
  }
}
