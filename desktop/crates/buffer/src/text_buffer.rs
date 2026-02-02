use std::{
  borrow::Cow,
  collections::VecDeque,
  ops::Range,
  time::{Duration, Instant},
};

use ropey::Rope;

const DEFAULT_GROUP_INTERVAL_MS: u64 = 300;

pub type TransactionId = usize;

/// Context passed to transaction closures to collect operations
pub struct TransactionContext {
  operations: Vec<TextOperation>,
}

impl TransactionContext {
  fn new() -> Self {
    Self {
      operations: Vec::new(),
    }
  }
}

/// A single text operation (insert, delete, or replace)
#[derive(Clone, Debug)]
pub struct TextOperation {
  pub range: Range<usize>,
  pub before: String,
  pub after: String,
}

impl TextOperation {
  pub fn undo(&self) -> Self {
    TextOperation {
      range: self.range.start..(self.range.start + self.after.len()),
      before: self.after.clone(),
      after: self.before.clone(),
    }
  }
}

/// A transaction groups one or more text operations
#[derive(Clone, Debug)]
struct Transaction {
  id: TransactionId,
  timestamp: Instant,
  operations: Vec<TextOperation>,
}

#[derive(Clone, Debug)]
pub struct TextBuffer {
  text: Rope,
  next_transaction_id: usize,
  undo_stack: VecDeque<Transaction>,
  redo_stack: VecDeque<Transaction>,
  group_interval: Duration,
}

impl Default for TextBuffer {
  fn default() -> Self {
    Self::new()
  }
}

impl TextBuffer {
  pub fn new() -> Self {
    Self {
      text: Rope::new(),
      next_transaction_id: 0,
      undo_stack: VecDeque::new(),
      redo_stack: VecDeque::new(),
      group_interval: Duration::from_millis(DEFAULT_GROUP_INTERVAL_MS),
    }
  }

  pub fn from_text(text: &str) -> Self {
    let mut rope = Rope::new();
    rope.insert(0, text);
    Self {
      text: rope,
      next_transaction_id: 0,
      undo_stack: VecDeque::new(),
      redo_stack: VecDeque::new(),
      group_interval: Duration::from_millis(DEFAULT_GROUP_INTERVAL_MS),
    }
  }

  pub fn len(&self) -> usize {
    self.text.len_chars()
  }

  pub fn is_empty(&self) -> bool {
    self.text.len_chars() == 0
  }

  pub fn chars(&self) -> impl Iterator<Item = char> {
    self.text.chars()
  }

  pub fn char_to_line(&self, char_idx: usize) -> usize {
    self.text.char_to_line(char_idx)
  }

  pub fn line_to_char(&self, line_idx: usize) -> usize {
    self.text.line_to_char(line_idx)
  }

  pub fn slice_to_string(&self, range: Range<usize>) -> String {
    self.text.slice(range).to_string()
  }

  pub fn len_lines(&self) -> usize {
    self.text.len_lines()
  }

  /// Get line content without trailing newlines
  pub fn line_content(&self, line_idx: usize) -> Option<Cow<'_, str>> {
    if line_idx < self.len_lines() {
      let line_slice = self.text.line(line_idx);

      // Try fast path: borrow if line is contiguous in memory and has no newlines
      if let Some(line_str) = line_slice.as_str()
        && !line_str.ends_with('\n')
        && !line_str.ends_with('\r')
      {
        return Some(Cow::Borrowed(line_str));
      }

      // Slow path: line crosses chunk boundaries or has newlines, must allocate
      let mut owned = line_slice.to_string();
      if owned.ends_with('\n') {
        owned.pop();
        if owned.ends_with('\r') {
          owned.pop();
        }
      }
      Some(Cow::Owned(owned))
    } else {
      None
    }
  }

  pub fn line_range(&self, line_idx: usize) -> Option<Range<usize>> {
    if line_idx < self.len_lines() {
      let start = self.line_to_char(line_idx);
      let end = if line_idx + 1 < self.len_lines() {
        self.line_to_char(line_idx + 1)
      } else {
        self.len()
      };
      Some(start..end)
    } else {
      None
    }
  }

  /// Insert text with transaction context
  pub fn insert(&mut self, tx: &mut TransactionContext, offset: usize, text: &str) {
    tx.operations.push(TextOperation {
      range: offset..offset,
      before: String::new(),
      after: text.to_string(),
    });

    self.text.insert(offset, text);
  }

  /// Remove text with transaction context
  pub fn remove(&mut self, tx: &mut TransactionContext, range: Range<usize>) {
    let before = self.slice_to_string(range.clone());

    tx.operations.push(TextOperation {
      range: range.clone(),
      before,
      after: String::new(),
    });

    self.text.remove(range);
  }

  /// Replace text with transaction context
  pub fn replace(&mut self, tx: &mut TransactionContext, range: Range<usize>, text: &str) {
    self.remove(tx, range.clone());
    self.insert(tx, range.start, text);
  }

  /// Execute a transaction with automatic commit
  pub fn transaction<F>(&mut self, now: Instant, f: F) -> TransactionId
  where
    F: FnOnce(&mut Self, &mut TransactionContext),
  {
    let transaction_id = self.next_transaction_id;
    let mut tx = TransactionContext::new();

    f(self, &mut tx);

    if !tx.operations.is_empty() {
      self.commit_transaction(tx.operations, now)
    } else {
      transaction_id
    }
  }

  fn commit_transaction(&mut self, operations: Vec<TextOperation>, now: Instant) -> TransactionId {
    let transaction_id = self.next_transaction_id;
    self.next_transaction_id += 1;

    // Try to group with last transaction if within time window
    if let Some(last) = self.undo_stack.back_mut()
      && !self.group_interval.is_zero()
      && now.saturating_duration_since(last.timestamp) < self.group_interval
    {
      last.operations.extend(operations);
      last.timestamp = now;
      self.redo_stack.clear();
      return last.id;
    }

    // Create new transaction
    self.undo_stack.push_back(Transaction {
      id: transaction_id,
      timestamp: now,
      operations,
    });
    self.redo_stack.clear();
    transaction_id
  }

  fn exec_operation(&mut self, operation: &TextOperation) {
    if operation.before.is_empty() && !operation.after.is_empty() {
      // Insert
      self.text.insert(operation.range.start, &operation.after);
    } else if !operation.before.is_empty() && operation.after.is_empty() {
      // Delete
      self.text.remove(operation.range.clone());
    } else if !operation.before.is_empty() && !operation.after.is_empty() {
      // Replace
      self.text.remove(operation.range.clone());
      self.text.insert(operation.range.start, &operation.after);
    }
    // If both empty, do nothing
  }

  pub fn undo(&mut self) -> Option<TransactionId> {
    let tx = self.undo_stack.pop_back()?;

    // Execute operations in reverse order with inverted operations
    for operation in tx.operations.iter().rev() {
      self.exec_operation(&operation.undo());
    }

    let id = tx.id;
    self.redo_stack.push_back(tx);
    Some(id)
  }

  pub fn redo(&mut self) -> Option<TransactionId> {
    let tx = self.redo_stack.pop_back()?;

    // Execute operations in forward order
    for operation in &tx.operations {
      self.exec_operation(operation);
    }

    let id = tx.id;
    self.undo_stack.push_back(tx);
    Some(id)
  }

  pub fn can_undo(&self) -> bool {
    !self.undo_stack.is_empty()
  }

  pub fn can_redo(&self) -> bool {
    !self.redo_stack.is_empty()
  }

  pub fn set_group_interval(&mut self, interval: Duration) {
    self.group_interval = interval;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_new_buffer() {
    let buffer = TextBuffer::new();
    assert_eq!(buffer.len(), 0);
    assert!(buffer.is_empty());
    assert_eq!(buffer.len_lines(), 1);
  }

  #[test]
  fn test_transaction_insert() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "hello");
    });

    assert_eq!(buffer.len(), 5);
    assert_eq!(buffer.slice_to_string(0..5), "hello");
  }

  #[test]
  fn test_transaction_replace() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "hello");
    });

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.replace(tx, 0..5, "world");
    });

    assert_eq!(buffer.slice_to_string(0..5), "world");
  }

  #[test]
  fn test_undo_insert() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 1, "b");
    });

    assert_eq!(buffer.slice_to_string(0..2), "ab");

    assert!(buffer.undo().is_some());
    assert_eq!(buffer.slice_to_string(0..1), "a");

    assert!(buffer.undo().is_some());
    assert_eq!(buffer.len(), 0);
  }

  #[test]
  fn test_redo_after_undo() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    buffer.undo();

    assert_eq!(buffer.len(), 0);

    assert!(buffer.redo().is_some());
    assert_eq!(buffer.slice_to_string(0..1), "a");
  }

  #[test]
  fn test_redo_cleared_after_new_edit() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    buffer.undo();

    // New edit should clear redo stack
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "b");
    });

    assert!(!buffer.can_redo());
  }

  #[test]
  fn test_edit_grouping() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(100));

    let now = Instant::now();
    buffer.transaction(now, |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    buffer.transaction(now, |buf, tx| {
      buf.insert(tx, 1, "b");
    });
    buffer.transaction(now, |buf, tx| {
      buf.insert(tx, 2, "c");
    });

    assert_eq!(buffer.slice_to_string(0..3), "abc");

    // All 3 inserts should be grouped into 1 undo
    assert!(buffer.undo().is_some());
    assert_eq!(buffer.len(), 0);
    assert!(!buffer.can_undo());
  }

  #[test]
  fn test_replace_undo() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "hello");
    });
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.replace(tx, 0..5, "world");
    });

    assert_eq!(buffer.slice_to_string(0..5), "world");

    assert!(buffer.undo().is_some());
    assert_eq!(buffer.slice_to_string(0..5), "hello");
  }

  #[test]
  fn test_multiple_operations_in_transaction() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "hello");
      buf.insert(tx, 5, " ");
      buf.insert(tx, 6, "world");
    });

    assert_eq!(buffer.slice_to_string(0..11), "hello world");

    // All operations in one transaction
    buffer.undo();
    assert_eq!(buffer.len(), 0);
  }

  #[test]
  fn test_can_undo_can_redo() {
    let mut buffer = TextBuffer::new();
    assert!(!buffer.can_undo());
    assert!(!buffer.can_redo());

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    assert!(buffer.can_undo());
    assert!(!buffer.can_redo());

    buffer.undo();
    assert!(!buffer.can_undo());
    assert!(buffer.can_redo());
  }

  #[test]
  fn test_multiple_undo_redo() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "one");
    });
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 3, " two");
    });
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 7, " three");
    });

    assert_eq!(buffer.slice_to_string(0..buffer.len()), "one two three");

    buffer.undo();
    assert_eq!(buffer.slice_to_string(0..buffer.len()), "one two");

    buffer.undo();
    assert_eq!(buffer.slice_to_string(0..buffer.len()), "one");

    buffer.redo();
    assert_eq!(buffer.slice_to_string(0..buffer.len()), "one two");
  }

  #[test]
  fn test_undo_empty_stack() {
    let mut buffer = TextBuffer::new();
    assert!(buffer.undo().is_none());
  }

  #[test]
  fn test_redo_empty_stack() {
    let mut buffer = TextBuffer::new();
    assert!(buffer.redo().is_none());
  }

  #[test]
  fn test_transaction_id_returned() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(0));

    let id1 = buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "hello");
    });
    let id2 = buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 5, " world");
    });

    assert_ne!(id1, id2);
  }

  #[test]
  fn test_transaction_id_grouping() {
    let mut buffer = TextBuffer::new();
    buffer.set_group_interval(Duration::from_millis(100));

    let now = Instant::now();
    let id1 = buffer.transaction(now, |buf, tx| {
      buf.insert(tx, 0, "a");
    });
    let id2 = buffer.transaction(now, |buf, tx| {
      buf.insert(tx, 1, "b");
    });

    // Same ID when grouped
    assert_eq!(id1, id2);
  }

  #[test]
  fn test_line_content() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "first line\nsecond line\nthird line");
    });

    assert_eq!(buffer.line_content(0).as_deref(), Some("first line"));
    assert_eq!(buffer.line_content(1).as_deref(), Some("second line"));
    assert_eq!(buffer.line_content(2).as_deref(), Some("third line"));
    assert_eq!(buffer.line_content(3), None);
  }

  #[test]
  fn test_line_range() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "abc\ndef\nghi");
    });

    assert_eq!(buffer.line_range(0), Some(0..4));
    assert_eq!(buffer.line_range(1), Some(4..8));
    assert_eq!(buffer.line_range(2), Some(8..11));
    assert_eq!(buffer.line_range(3), None);
  }

  #[test]
  fn test_unicode_handling() {
    let mut buffer = TextBuffer::new();
    buffer.transaction(Instant::now(), |buf, tx| {
      buf.insert(tx, 0, "héllo 世界");
    });

    assert_eq!(buffer.len(), 8);
    assert_eq!(buffer.slice_to_string(0..5), "héllo");
    assert_eq!(buffer.slice_to_string(6..8), "世界");
  }

  #[test]
  fn test_invert_operation() {
    let op = TextOperation {
      range: 0..5,
      before: "hello".to_string(),
      after: "world".to_string(),
    };

    let inverted = op.undo();
    assert_eq!(inverted.range, 0..5);
    assert_eq!(inverted.before, "world");
    assert_eq!(inverted.after, "hello");
  }
}
