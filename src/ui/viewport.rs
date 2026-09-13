//! Keep the list's visible range independent from its selected row. Scrolling
//! backward first moves the selection through the current viewport.
use std::cell::Cell;

#[derive(Default)]
pub(super) struct Viewport(Cell<usize>);

impl Viewport {
    pub(super) fn start(&self, selected: usize, length: usize, rows: usize, below: usize) -> usize {
        let rows = rows.max(1);
        let selected = selected.min(length.saturating_sub(1));
        let start = self
            .0
            .get()
            .min(selected)
            .max(
                selected
                    .saturating_add(1 + below.min(rows - 1))
                    .saturating_sub(rows),
            )
            .min(length.saturating_sub(rows));
        self.0.set(start);
        start
    }
}
