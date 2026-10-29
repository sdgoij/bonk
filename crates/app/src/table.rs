//! A small table with resizable, sortable headers.
//!
//! egui has no table widget that both resizes and expands to the window, so this
//! one keeps column weights rather than pixels. A weight is a share of the
//! available width, which means a window resize rearranges the columns without
//! any stored size going stale. Dragging a separator moves weight between its
//! two neighbours and leaves the total unchanged.

// Column geometry is floating point; amounts are not, and never meet this code.
#![allow(clippy::float_arithmetic)]

use egui::{Align, Color32, CursorIcon, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::theme;

const HEADER_HEIGHT: f32 = 32.0;
const ROW_HEIGHT: f32 = 32.0;
const CELL_PADDING: f32 = 10.0;
const MIN_COLUMN: f32 = 54.0;

/// One cell of a row.
pub struct Cell {
    text: String,
    align: Align,
    strong: bool,
    muted: bool,
}

impl Cell {
    /// Left-aligned text.
    pub fn left(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            align: Align::Min,
            strong: false,
            muted: false,
        }
    }

    /// Right-aligned text, for amounts.
    pub fn right(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            align: Align::Max,
            strong: false,
            muted: false,
        }
    }

    /// Draws the cell in the muted colour.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    /// Draws the cell in a stronger weight.
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }
}

/// The table's persistent state: column shares and the chosen sort.
pub struct Table {
    weights: Vec<f32>,
    /// Column index and whether the order is descending.
    pub sort: Option<(usize, bool)>,
}

impl Table {
    /// A table whose columns start at the given shares.
    pub fn new(weights: &[f32]) -> Self {
        Self {
            weights: weights.to_vec(),
            sort: None,
        }
    }

    /// Draws the header, handles resizing and sorting, and returns whether the
    /// sort changed this frame.
    pub fn header(&mut self, ui: &mut Ui, id: &str, titles: &[&str], width: f32) -> bool {
        let widths = self.pixel_widths(width);
        let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::hover());
        let tokens = theme::palette(ui.ctx());
        ui.painter().rect_filled(rect, 6.0, tokens.surface_alt);
        let before = self.sort;

        let mut x = rect.left();
        for (index, title) in titles.iter().enumerate() {
            let cell_width = widths[index];
            let label = Rect::from_min_max(
                pos2(x + CELL_PADDING, rect.top()),
                pos2(x + cell_width - CELL_PADDING, rect.bottom()),
            );
            theme::cell_label(ui, label, title, Align::Min, true, tokens.text);

            // The separator owns the last few points of the column, so a click
            // there resizes instead of sorting.
            let hit = Rect::from_min_max(pos2(x, rect.top()), pos2(x + cell_width - 4.0, rect.bottom()));
            let response = ui.interact(hit, ui.scope_id().with((id, "head", index)), Sense::click());
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked() {
                self.sort = Some(match self.sort {
                    Some((column, descending)) if column == index => (index, !descending),
                    _ => (index, false),
                });
            }
            if let Some((column, descending)) = self.sort
                && column == index
            {
                theme::sort_caret(
                    ui.painter(),
                    pos2(x + cell_width - 18.0, rect.center().y),
                    !descending,
                    tokens.accent,
                );
            }

            if index + 1 < titles.len() {
                let separator = Rect::from_min_max(
                    pos2(x + cell_width - 3.0, rect.top() + 5.0),
                    pos2(x + cell_width + 3.0, rect.bottom() - 5.0),
                );
                let drag = ui.interact(separator, ui.scope_id().with((id, "drag", index)), Sense::drag());
                if drag.hovered() || drag.dragged() {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    ui.painter()
                        .rect_filled(separator.shrink2(vec2(2.0, 0.0)), 1.0, tokens.accent);
                }
                if drag.dragged() {
                    self.resize(index, drag.drag_delta().x, width);
                }
            }
            x += cell_width;
        }

        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, tokens.border),
        );
        self.sort != before
    }

    /// Draws one row and returns its click response.
    pub fn row(&self, ui: &mut Ui, width: f32, cells: &[Cell], selected: bool, striped: bool) -> Response {
        let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let tokens = theme::palette(ui.ctx());
        let fill = if selected {
            tokens.selection
        } else if response.hovered() {
            tokens.hover
        } else if striped {
            tokens.stripe
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 4.0, fill);
        }

        let widths = self.pixel_widths(width);
        let mut x = rect.left();
        for (index, cell) in cells.iter().enumerate() {
            let area = Rect::from_min_max(
                pos2(x + CELL_PADDING, rect.top()),
                pos2(x + widths[index] - CELL_PADDING, rect.bottom()),
            );
            let colour = if cell.muted { tokens.muted } else { tokens.text };
            theme::cell_label(ui, area, &cell.text, cell.align, cell.strong, colour);
            x += widths[index];
        }
        response
    }

    /// The pixel width of each column at the given total width.
    pub fn pixel_widths(&self, width: f32) -> Vec<f32> {
        let total: f32 = self.weights.iter().sum();
        if total <= 0.0 {
            return vec![width; self.weights.len()];
        }
        let mut widths: Vec<f32> = self.weights.iter().map(|weight| width * weight / total).collect();
        // Floating point rounding leaves a fraction unclaimed; give it to the
        // last column so the table ends where the window does.
        let used: f32 = widths.iter().sum();
        if let Some(last) = widths.last_mut() {
            *last += width - used;
        }
        widths
    }

    /// Moves width between column `index` and the one after it.
    fn resize(&mut self, index: usize, delta: f32, width: f32) {
        let total: f32 = self.weights.iter().sum();
        if total <= 0.0 || width <= 0.0 || index + 1 >= self.weights.len() {
            return;
        }
        let minimum = MIN_COLUMN / width * total;
        let pair = self.weights[index] + self.weights[index + 1];
        if pair <= minimum * 2.0 {
            return;
        }
        let moved = delta / width * total;
        let left = (self.weights[index] + moved).clamp(minimum, pair - minimum);
        self.weights[index] = left;
        self.weights[index + 1] = pair - left;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fill_the_width_exactly() {
        let table = Table::new(&[1.0, 2.0, 3.0]);
        let widths = table.pixel_widths(600.0);
        assert_eq!(widths.len(), 3);
        let sum: f32 = widths.iter().sum();
        assert!((sum - 600.0).abs() < 0.01, "{widths:?} sums to {sum}");
    }

    #[test]
    fn resizing_moves_weight_without_changing_the_total() {
        let mut table = Table::new(&[1.0, 1.0, 1.0]);
        table.resize(0, 20.0, 600.0);
        let sum: f32 = table.weights.iter().sum();
        assert!((sum - 3.0).abs() < 0.001, "{:?}", table.weights);
        assert!(table.weights[0] > 1.0);
        assert!(table.weights[1] < 1.0);
        assert!((table.weights[2] - 1.0).abs() < 0.001);
    }

    #[test]
    fn a_column_stops_at_its_minimum() {
        let mut table = Table::new(&[1.0, 1.0]);
        table.resize(0, 1_000.0, 600.0);
        let widths = table.pixel_widths(600.0);
        assert!(widths[0] >= MIN_COLUMN - 0.01, "{widths:?}");
        assert!(widths[1] >= MIN_COLUMN - 0.01, "{widths:?}");
    }
}
