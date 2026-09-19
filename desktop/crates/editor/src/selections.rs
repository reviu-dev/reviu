use std::ops::Range;

use gpui::Pixels;

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
  pub id: u64,
  pub range: Range<usize>,
  pub reversed: bool,
  pub goal: Option<Pixels>,
}

impl Selection {
  pub fn head(&self) -> usize {
    if self.reversed {
      self.range.start
    } else {
      self.range.end
    }
  }

  pub fn anchor(&self) -> usize {
    if self.reversed {
      self.range.end
    } else {
      self.range.start
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selections {
  primary: Selection,
  others: Vec<Selection>,
  next_id: u64,
}

impl Default for Selections {
  fn default() -> Self {
    Self {
      primary: Selection {
        id: 0,
        range: 0..0,
        reversed: false,
        goal: None,
      },
      others: Vec::new(),
      next_id: 1,
    }
  }
}

impl Selections {
  pub fn primary(&self) -> &Selection {
    &self.primary
  }
  pub fn primary_mut(&mut self) -> &mut Selection {
    &mut self.primary
  }

  pub fn iter(&self) -> impl Iterator<Item = &Selection> {
    let index = self
      .others
      .partition_point(|selection| selection.range.start < self.primary.range.start);
    self
      .others
      .iter()
      .take(index)
      .chain(std::iter::once(&self.primary))
      .chain(self.others.iter().skip(index))
  }

  pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Selection> {
    std::iter::once(&mut self.primary).chain(self.others.iter_mut())
  }

  pub fn len(&self) -> usize {
    self.others.len() + 1
  }
  pub fn is_empty(&self) -> bool {
    false
  }

  pub(crate) fn single(&mut self) {
    self.others.clear();
  }

  pub(crate) fn add(&mut self, range: Range<usize>, reversed: bool) {
    self.extend(std::iter::once(range), reversed);
  }

  pub(crate) fn extend(&mut self, ranges: impl IntoIterator<Item = Range<usize>>, reversed: bool) {
    for range in ranges {
      let selection = Selection {
        id: self.next_id,
        range,
        reversed,
        goal: None,
      };
      self.next_id += 1;
      self
        .others
        .push(std::mem::replace(&mut self.primary, selection));
    }
    self.normalize();
  }

  pub(crate) fn set_primary(&mut self, id: u64) {
    if let Some(selection) = self.others.iter_mut().find(|selection| selection.id == id) {
      std::mem::swap(selection, &mut self.primary);
      self
        .others
        .sort_by_key(|selection| (selection.range.start, selection.range.end));
    }
  }

  pub(crate) fn normalize(&mut self) {
    let mut entries = std::mem::take(&mut self.others);
    entries.push(self.primary.clone());
    entries.sort_by_key(|selection| (selection.range.start, selection.range.end));
    let mut merged: Vec<Selection> = Vec::with_capacity(entries.len());
    for selection in entries {
      if let Some(previous) = merged.last_mut()
        && (selection.range.start < previous.range.end
          || (selection.range.start == previous.range.end
            && (selection.range.is_empty() || previous.range.is_empty())))
      {
        let range = previous.range.start..previous.range.end.max(selection.range.end);
        if selection.id == self.primary.id {
          *previous = selection;
        }
        previous.range = range;
      } else {
        merged.push(selection);
      }
    }
    for selection in merged {
      if selection.id == self.primary.id {
        self.primary = selection;
      } else {
        self.others.push(selection);
      }
    }
  }

  pub(crate) fn clamp(&mut self, length: usize) {
    for selection in self.iter_mut() {
      selection.range.start = selection.range.start.min(length);
      selection.range.end = selection.range.end.min(length);
      selection.reversed &= !selection.range.is_empty();
    }
    self.normalize();
  }
}
