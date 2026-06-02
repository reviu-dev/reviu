mod element;
mod state;
mod word;

pub use element::SelectableText;
pub use state::{SelectionMode, SelectionRegistry};
pub use word::{CharType, is_word_char, word_range_at};
