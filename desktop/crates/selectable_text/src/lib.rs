mod element;
mod state;
mod word;

pub use element::{
  SelectableText, apply_selection_to_runs, extend_selection, mode_for_click_count, selection_range,
};
pub use state::{ActiveSelection, SelectionMode, SelectionRegistry};
pub use word::{CharType, clamp_to_char_boundary, is_word_char, line_range_at, word_range_at};
