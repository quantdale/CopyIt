//! Snippet card grid: layout constants, virtualization math, gap geometry,
//! insertion-line drawing, and the drag state machine.
//!
//! Everything a maintainer must touch to change the grid — card size,
//! spacing, column count, which rows are built, the insertion-line
//! placement, the drag threshold, and the drop-target math — lives behind
//! this module's interface. `app.rs` stays thin: it renders cards, feeds
//! pointer events into [`DragMachine`], and resolves the drop against
//! [`DragContext`].

use eframe::egui::{self, pos2, vec2, Pos2, Rect};

/// Width of a card's inner content area.
pub const CARD_INNER_W: f32 = 300.0;
/// Height of a card's inner content area.
pub const CARD_INNER_H: f32 = 168.0;
/// The group frame around each card adds 10 px of inner margin on each side.
pub const CARD_FRAME_MARGIN: f32 = 20.0;
/// Full visible width of a card, frame included.
pub const CARD_W: f32 = CARD_INNER_W + CARD_FRAME_MARGIN;
/// Full visible height of a card, frame included.
pub const CARD_H: f32 = CARD_INNER_H + CARD_FRAME_MARGIN;
/// Gap between neighbouring cards, horizontally and vertically.
pub const CARD_SPACING: f32 = 12.0;
/// Vertical distance from the top of one row of cards to the top of the next.
pub const ROW_PITCH: f32 = CARD_H + CARD_SPACING;
/// Blank space above the first row inside the scroll area.
pub const GRID_TOP_SPACE: f32 = 4.0;
/// Horizontal padding on both sides of the grid.
pub const GRID_MARGIN_X: f32 = 18.0;

/// Pointer movement (px) required before an armed drag becomes a real drag.
const DRAG_THRESHOLD: f32 = 4.0;

/// Number of columns that fit `avail_width` px of content width.
pub fn cols_for(avail_width: f32) -> usize {
    ((avail_width / (CARD_W + CARD_SPACING)).floor() as usize).max(1)
}

/// Position of the card at index `i` of a grid of `cols` columns, computed
/// from the grid's origin rather than from layout. The grid is uniform —
/// `cols` cards of `card_w` x `card_h` per row, separated by `spacing` — so
/// off-screen cards still have exact rects for drag-and-drop hit-testing
/// without being laid out or painted.
pub fn grid_card_rect(
    i: usize,
    cols: usize,
    origin: Pos2,
    card_w: f32,
    card_h: f32,
    spacing: f32,
) -> Rect {
    let cols = cols.max(1);
    let col = i % cols;
    let row = i / cols;
    Rect::from_min_size(
        origin
            + vec2(
                col as f32 * (card_w + spacing),
                row as f32 * (card_h + spacing),
            ),
        vec2(card_w, card_h),
    )
}

/// Inclusive range of grid rows that intersect `clip` (the visible part of the scroll
/// area), with one row of overscan on each side so a row entering the viewport is
/// already laid out and edge rounding can't reveal a gap. Rows outside the range are
/// replaced by blank space of the same height, so scrolling and card positions are
/// unaffected. Non-finite geometry (NaN or infinity in `clip`, `origin_y`, or
/// `row_pitch`) falls back to "every row". Extreme finite inputs whose
/// `clip - origin_y` difference overflows to infinity in the intermediate math
/// saturate to a single row via the `as usize` clamp rather than wrapping.
pub fn visible_rows(clip: Rect, origin_y: f32, row_pitch: f32, rows: usize) -> (usize, usize) {
    let max_row = rows.saturating_sub(1);
    if rows == 0 {
        return (0, 0);
    }
    if !row_pitch.is_finite()
        || row_pitch <= 0.0
        || !origin_y.is_finite()
        || !clip.top().is_finite()
        || !clip.bottom().is_finite()
    {
        return (0, max_row);
    }
    let first = (((clip.top() - origin_y) / row_pitch).floor() - 1.0).max(0.0);
    let last = (((clip.bottom() - origin_y) / row_pitch).ceil() + 1.0).max(0.0);
    // `as usize` saturates, so an absurd clip rect clamps instead of wrapping.
    let first = (first as usize).min(max_row);
    let last = (last as usize).clamp(first, max_row);
    (first, last)
}

/// Computes a representative point for a given gap in the grid, used for distance-based
/// gap selection. The grid is arranged in rows of `cols` cards each.
/// - If g is a vertical gap (between cards in the same row), return the midpoint between them.
/// - If g is a horizontal gap (between rows), return a point centered on the column nearest
///   the pointer's x-coordinate. This ensures the insertion line aligns with the pointer's
///   intended column even when dragging over empty space between rows.
pub fn gap_point(
    g: usize,
    rects: &[Rect],
    cols: usize,
    spacing: f32,
    _card_w: f32,
    pointer: Pos2,
) -> Pos2 {
    let cols = cols.max(1);
    if rects.is_empty() {
        return pos2(0.0, 0.0);
    }
    let n = rects.len();

    // Same-row vertical gap: return the point midway between the two adjacent cards.
    if g > 0 && g < n && !g.is_multiple_of(cols) {
        let x = (rects[g - 1].right() + rects[g].left()) * 0.5;
        let y = rects[g].center().y;
        return pos2(x, y);
    }

    // Row-boundary horizontal gap: y-coordinate centered in the gap; x-coordinate follows pointer.
    let y = if g == 0 {
        rects[0].top() - spacing * 0.5
    } else if g == n {
        rects[n - 1].bottom() + spacing * 0.5
    } else {
        (rects[g - 1].bottom() + rects[g].top()) * 0.5
    };

    // Find the card column whose x-center is closest to the pointer's x position.
    // `total_cmp` keeps this from panicking if a coordinate is ever NaN.
    let nearest_x = rects
        .iter()
        .map(|r| r.center().x)
        .min_by(|a, b| (a - pointer.x).abs().total_cmp(&(b - pointer.x).abs()))
        .unwrap_or(rects[0].center().x);

    pos2(nearest_x, y)
}

/// Finds the closest insertion gap (0 to n, inclusive) based on the pointer position.
/// A "gap" is a logical position between cards in the filtered grid: gap 0 is before the first card,
/// gap n is after the last card, and gaps in between are the spaces between adjacent cards (both
/// vertical within a row and horizontal between rows). The function computes a representative
/// point for each gap (via [`gap_point`]) and returns the index of the gap whose point is closest
/// to the current pointer position. This guides the insertion line and drop target.
/// Returns 0 if the card list is empty (edge case: no cards to reorder against).
pub fn nearest_gap(pointer: Pos2, rects: &[Rect], cols: usize, spacing: f32, card_w: f32) -> usize {
    let cols = cols.max(1);
    let n = rects.len();
    if n == 0 {
        return 0;
    }

    let mut best = 0;
    let mut best_dist = f32::INFINITY;

    for g in 0..=n {
        let p = gap_point(g, rects, cols, spacing, card_w, pointer);
        let d = pointer.distance(p);
        if d < best_dist {
            best_dist = d;
            best = g;
        }
    }

    best
}

/// Renders a dashed insertion line indicating where the dragged card would be dropped.
/// For vertical gaps (between cards in the same row), draws a short vertical dashed line centered
/// between the two cards. For horizontal gaps (between rows), draws a horizontal dashed line
/// centered on the column nearest the pointer's x-position. Both cases include clearance checks
/// to ensure the line doesn't visually overlap with adjacent cards (which would be confusing).
/// The line only draws if its computed length is non-zero and it won't intersect any cards.
pub fn draw_insertion_line(
    ctx: &egui::Context,
    gap: usize,
    rects: &[Rect],
    cols: usize,
    spacing: f32,
    card_w: f32,
    card_h: f32,
) {
    let cols = cols.max(1);
    let n = rects.len();
    if n == 0 {
        return;
    }
    let pointer = ctx.input(|i| i.pointer.interact_pos().unwrap_or_default());

    let color = egui::Color32::from_rgb(0x60, 0xb0, 0xff);
    let stroke = egui::Stroke::new(2.0_f32, color);
    let clearance = 4.0_f32; // Minimum distance the line must maintain from card edges

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("drag_ghost"),
    ));

    if gap > 0 && gap < n && !gap.is_multiple_of(cols) {
        // Vertical dashed line between two cards on the same row.
        let a = &rects[gap - 1];
        let b = &rects[gap];
        let x = (a.right() + b.left()) * 0.5;
        let y_center = a.center().y;
        // Make the line span ~35% of the card height, but never extend past the card boundaries.
        let half_len = (card_h * 0.35).min(a.height() * 0.5 - clearance);
        let from = pos2(x, y_center - half_len);
        let to = pos2(x, y_center + half_len);
        let line_rect = line_segment_rect(from, to, stroke.width);
        if half_len > 0.0 && !line_rect.intersects(*a) && !line_rect.intersects(*b) {
            draw_dashed_line(&painter, from, to, 6.0, 4.0, stroke);
        }
    } else {
        // Horizontal dashed line at a row boundary (top, between rows, or bottom).
        let (y, gap_top, gap_bottom, above, below) = if gap == 0 {
            (
                rects[0].top() - spacing * 0.5,
                rects[0].top() - spacing,
                rects[0].top(),
                None,
                Some(&rects[0]),
            )
        } else if gap == n {
            (
                rects[n - 1].bottom() + spacing * 0.5,
                rects[n - 1].bottom(),
                rects[n - 1].bottom() + spacing,
                Some(&rects[n - 1]),
                None,
            )
        } else {
            (
                (rects[gap - 1].bottom() + rects[gap].top()) * 0.5,
                rects[gap - 1].bottom(),
                rects[gap].top(),
                Some(&rects[gap - 1]),
                Some(&rects[gap]),
            )
        };

        // Center the horizontal line on the column of the nearest card to guide the drop location.
        let nearest = rects
            .iter()
            .min_by(|a, b| {
                (a.center().x - pointer.x)
                    .abs()
                    .total_cmp(&(b.center().x - pointer.x).abs())
            })
            .unwrap_or(&rects[0]);
        let x_center = nearest.center().x;
        let half_len = (card_w * 0.35).min(nearest.width() * 0.5 - clearance);
        let from = pos2(x_center - half_len, y);
        let to = pos2(x_center + half_len, y);
        let line_rect = line_segment_rect(from, to, stroke.width);
        // Only draw if the line has positive length and maintains clearance from adjacent cards.
        let mut clear = half_len > 0.0 && y > gap_top + clearance && y < gap_bottom - clearance;
        if let Some(a) = above {
            clear &= !line_rect.intersects(*a);
        }
        if let Some(b) = below {
            clear &= !line_rect.intersects(*b);
        }
        if clear {
            draw_dashed_line(&painter, from, to, 6.0, 4.0, stroke);
        }
    }
}

/// Screen-space context a drop is decided against: the grid's bounds, the
/// screen-space rect of every filtered card, and the column count.
pub struct DragContext<'a> {
    pub grid_area: Rect,
    pub card_rects: &'a [Rect],
    pub cols: usize,
}

/// Tracks an in-progress drag operation. Initialized when the user clicks and holds on a card,
/// but remains in a "pre-drag" state until the pointer moves >[`DRAG_THRESHOLD`] px (to avoid
/// accidental drags from single clicks). Once dragging becomes true, visual feedback (insertion
/// line, ghost box) appears to guide the user to a drop location.
///
/// Only the stable `snippet_id` is retained — never the index the card had at drag
/// start. The caller resolves the index by id at drop time, so a library that changed
/// mid-drag reorders the right card or nothing at all instead of moving the wrong one
/// (or panicking in `Vec::remove`).
pub struct DragMachine {
    snippet_id: u64,
    start_pos: Pos2,
    dragging: bool,
}

impl DragMachine {
    /// Arms a drag for `snippet_id`, capturing the pointer position at initiation.
    pub fn begin(snippet_id: u64, start_pos: Pos2) -> Self {
        Self {
            snippet_id,
            start_pos,
            dragging: false,
        }
    }

    /// The stable id of the snippet being dragged.
    pub fn snippet_id(&self) -> u64 {
        self.snippet_id
    }

    /// True once the pointer has moved far enough past the start to count as a drag.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Feed the current pointer position in: promotes the armed drag to a real one
    /// once it crosses the threshold.
    pub fn pointer(&mut self, pos: Pos2) {
        if !self.dragging && self.start_pos.distance(pos) > DRAG_THRESHOLD {
            self.dragging = true;
        }
    }

    /// The primary button was released at `pointer`. Consumes the machine:
    /// `Some((snippet_id, gap))` when a drag is in progress and the pointer is
    /// inside the grid, `None` when the release cancels (click without drag, or
    /// dropped outside the grid).
    pub fn release(self, pointer: Pos2, ctx: &DragContext<'_>) -> Option<(u64, usize)> {
        if self.dragging && ctx.grid_area.contains(pointer) {
            let gap = nearest_gap(pointer, ctx.card_rects, ctx.cols, CARD_SPACING, CARD_W);
            Some((self.snippet_id, gap))
        } else {
            None
        }
    }
}

/// Computes a tight bounding rect of a line segment, including its stroke thickness on all sides.
/// Used to check for visual overlap between the insertion line and adjacent cards during drag-and-drop.
fn line_segment_rect(from: Pos2, to: Pos2, stroke_width: f32) -> Rect {
    let half = stroke_width * 0.5;
    Rect::from_min_max(
        (from.min(to)) - vec2(half, half),
        (from.max(to)) + vec2(half, half),
    )
}

/// Draws a dashed line by rendering alternating solid segments (dashes) and transparent gaps.
/// This creates a visual "dashed" effect without needing special stroke rendering. The line
/// is drawn along the direction from `from` to `to`, and `dash_len` / `gap_len` control
/// the length of each dash and the space between them (both in screen pixels).
fn draw_dashed_line(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    dash_len: f32,
    gap_len: f32,
    stroke: egui::Stroke,
) {
    let vec = to - from;
    let total = vec.length();
    if total <= 0.0 {
        return;
    }
    let dir = vec / total;
    let mut pos = 0.0;
    let mut drawing_dash = true;
    while pos < total {
        let seg_len = if drawing_dash {
            dash_len.min(total - pos)
        } else {
            gap_len.min(total - pos)
        };
        if drawing_dash {
            // Only draw the solid segment; skip gaps by not painting them.
            let a = from + dir * pos;
            let b = from + dir * (pos + seg_len);
            painter.line_segment([a, b], stroke);
        }
        pos += seg_len;
        drawing_dash = !drawing_dash; // Alternate between drawing and skipping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure rects for a `count`-card grid starting at the origin — no egui context
    /// needed, which is exactly why the geometry lives in this module.
    fn card_rects(count: usize, cols: usize) -> Vec<Rect> {
        (0..count)
            .map(|i| grid_card_rect(i, cols, Pos2::ZERO, CARD_W, CARD_H, CARD_SPACING))
            .collect()
    }

    fn pointer_in_vertical_gap(rects: &[Rect], g: usize) -> Pos2 {
        let a = rects[g - 1];
        let b = rects[g];
        pos2((a.right() + b.left()) * 0.5, a.center().y)
    }

    fn pointer_in_horizontal_gap(rects: &[Rect], g: usize) -> Pos2 {
        let a = rects[g - 1];
        let b = rects[g];
        pos2(a.center().x, (a.bottom() + b.top()) * 0.5)
    }

    fn grid_area() -> Rect {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 700.0))
    }

    #[test]
    fn cols_for_fits_available_width() {
        assert_eq!(cols_for(1000.0), 3);
        assert_eq!(cols_for(CARD_W * 2.0 + CARD_SPACING * 2.0), 2);
        assert_eq!(cols_for(CARD_W * 2.0 + CARD_SPACING * 2.0 - 1.0), 1);
        assert_eq!(cols_for(1.0), 1);
    }

    #[test]
    fn card_rects_have_card_size_and_spacing() {
        let rects = card_rects(7, 2);
        for r in &rects {
            assert!((r.width() - CARD_W).abs() < 0.1);
            assert!((r.height() - CARD_H).abs() < 0.1);
        }
        for g in 1..rects.len() {
            if g % 2 != 0 {
                let a = rects[g - 1];
                let b = rects[g];
                let gap = b.left() - a.right();
                let midpoint = (a.right() + b.left()) * 0.5;
                assert!(
                    (gap - CARD_SPACING).abs() < 0.1,
                    "vertical gap {} = {}",
                    g,
                    gap
                );
                assert!((midpoint - (a.right() + gap * 0.5)).abs() < 0.1);
            } else {
                let a = rects[g - 1];
                let b = rects[g];
                let gap = b.top() - a.bottom();
                let midpoint = (a.bottom() + b.top()) * 0.5;
                assert!(
                    (gap - CARD_SPACING).abs() < 0.1,
                    "horizontal gap {} = {}",
                    g,
                    gap
                );
                assert!((midpoint - (a.bottom() + gap * 0.5)).abs() < 0.1);
            }
        }
    }

    #[test]
    fn gap_points_are_centered_and_clear() {
        let rects = card_rects(7, 2);
        let cols = 2;
        let clearance = 4.0_f32;
        let stroke_width = 2.0_f32;

        // Boundary gaps (before the first card and after the last) are horizontal
        // gaps: y sits centered in the spacing above/below the edge card, and x
        // follows the nearest column to the pointer.
        let p0 = gap_point(0, &rects, cols, CARD_SPACING, CARD_W, rects[0].center());
        assert!(
            (p0.y - (rects[0].top() - CARD_SPACING * 0.5)).abs() < 0.1,
            "gap 0 y = {}, expected {}",
            p0.y,
            rects[0].top() - CARD_SPACING * 0.5
        );
        assert!((p0.x - rects[0].center().x).abs() < 0.1);
        let pn = gap_point(
            rects.len(),
            &rects,
            cols,
            CARD_SPACING,
            CARD_W,
            rects[rects.len() - 1].center(),
        );
        assert!(
            (pn.y - (rects[rects.len() - 1].bottom() + CARD_SPACING * 0.5)).abs() < 0.1,
            "gap n y = {}, expected {}",
            pn.y,
            rects[rects.len() - 1].bottom() + CARD_SPACING * 0.5
        );
        assert!((pn.x - rects[rects.len() - 1].center().x).abs() < 0.1);

        for g in 1..rects.len() {
            let pointer = if g % cols != 0 {
                pointer_in_vertical_gap(&rects, g)
            } else {
                pointer_in_horizontal_gap(&rects, g)
            };
            let p = gap_point(g, &rects, cols, CARD_SPACING, CARD_W, pointer);

            if g % cols != 0 {
                let a = rects[g - 1];
                let b = rects[g];
                let gap = b.left() - a.right();
                let expected_x = a.right() + gap * 0.5;
                assert!(
                    (p.x - expected_x).abs() < 0.1,
                    "vertical gap {} x = {}, expected {}",
                    g,
                    p.x,
                    expected_x
                );
                assert!((p.y - a.center().y).abs() < 0.1);
                let line_rect = Rect::from_min_max(
                    pos2(p.x - stroke_width * 0.5, p.y - 60.0),
                    pos2(p.x + stroke_width * 0.5, p.y + 60.0),
                );
                assert!(
                    !line_rect.intersects(a),
                    "vertical line intersects left card"
                );
                assert!(
                    !line_rect.intersects(b),
                    "vertical line intersects right card"
                );
                assert!(p.x > a.right() + clearance - stroke_width * 0.5);
                assert!(p.x < b.left() - clearance + stroke_width * 0.5);
            } else {
                let a = rects[g - 1];
                let b = rects[g];
                let gap = b.top() - a.bottom();
                let expected_y = a.bottom() + gap * 0.5;
                assert!(
                    (p.y - expected_y).abs() < 0.1,
                    "horizontal gap {} y = {}, expected {}",
                    g,
                    p.y,
                    expected_y
                );
                let line_rect = Rect::from_min_max(
                    pos2(p.x - 112.0, p.y - stroke_width * 0.5),
                    pos2(p.x + 112.0, p.y + stroke_width * 0.5),
                );
                assert!(
                    !line_rect.intersects(a),
                    "horizontal line intersects above card"
                );
                assert!(
                    !line_rect.intersects(b),
                    "horizontal line intersects below card"
                );
                assert!(p.y > a.bottom() + clearance - stroke_width * 0.5);
                assert!(p.y < b.top() - clearance + stroke_width * 0.5);
            }
        }
    }

    #[test]
    fn nearest_gap_picks_the_closest_gap() {
        let rects = card_rects(4, 2);
        let p = pointer_in_vertical_gap(&rects, 1);
        assert_eq!(nearest_gap(p, &rects, 2, CARD_SPACING, CARD_W), 1);
        let p = pointer_in_horizontal_gap(&rects, 2);
        assert_eq!(nearest_gap(p, &rects, 2, CARD_SPACING, CARD_W), 2);
        let p = pos2(rects[3].right() + 100.0, rects[3].bottom() + 6.0);
        assert_eq!(nearest_gap(p, &rects, 2, CARD_SPACING, CARD_W), 4);
        let p = pos2(rects[0].center().x, rects[0].top() - 100.0);
        assert_eq!(nearest_gap(p, &rects, 2, CARD_SPACING, CARD_W), 0);
        assert_eq!(nearest_gap(pos2(0.0, 0.0), &[], 2, CARD_SPACING, CARD_W), 0);
    }

    #[test]
    fn zero_cols_does_not_panic() {
        // `is_multiple_of(cols)` panics on a zero rhs, so cols: 0 must be clamped
        // before it is reached. These calls must not panic and return something sane.
        let rects = card_rects(4, 2);
        let p = pointer_in_vertical_gap(&rects, 1);
        let gp = gap_point(1, &rects, 0, CARD_SPACING, CARD_W, p);
        assert!(gp.is_finite());
        let ng = nearest_gap(p, &rects, 0, CARD_SPACING, CARD_W);
        assert!((0..=rects.len()).contains(&ng));

        // Empty card list with cols: 0 must also be safe.
        assert_eq!(
            gap_point(0, &[], 0, CARD_SPACING, CARD_W, pos2(0.0, 0.0)),
            pos2(0.0, 0.0)
        );
        assert_eq!(nearest_gap(pos2(0.0, 0.0), &[], 0, CARD_SPACING, CARD_W), 0);
    }

    #[test]
    fn visible_rows_covers_the_viewport_and_stays_in_bounds() {
        let row_pitch = 200.0_f32;
        let origin_y = 100.0_f32;
        let rows = 50;

        let clip = Rect::from_min_max(pos2(0.0, 100.0), pos2(1000.0, 700.0));
        let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
        assert_eq!(first, 0);
        assert!(last >= 3, "viewport spans 3 rows, got last = {last}");
        assert!(last < rows);

        let clip = Rect::from_min_max(pos2(0.0, 2100.0), pos2(1000.0, 2700.0));
        let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
        assert!((8..=9).contains(&first), "first = {first}");
        assert!(last >= 13, "last = {last}");
        assert!(first > 0, "rows above the viewport must be skipped");
        assert!(last < rows - 1, "rows below the viewport must be skipped");

        for row in 0..rows {
            let y = origin_y + row as f32 * row_pitch;
            let clip = Rect::from_min_max(pos2(0.0, y), pos2(1000.0, y + 10.0));
            let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
            assert!(
                first <= row && row <= last,
                "row {row} not rendered for its own scroll position ({first}..={last})"
            );
            assert!(last < rows);
        }

        // Degenerate inputs fall back to rendering everything rather than nothing.
        assert_eq!(visible_rows(clip, origin_y, 0.0, rows), (0, rows - 1));
        assert_eq!(visible_rows(clip, f32::NAN, row_pitch, rows), (0, rows - 1));
        assert_eq!(visible_rows(clip, origin_y, row_pitch, 1), (0, 0));
        assert_eq!(visible_rows(clip, origin_y, row_pitch, 0), (0, 0));

        // An infinite clip is non-finite geometry, so it falls back to every row.
        let inf_clip = Rect::from_min_max(pos2(0.0, f32::INFINITY), pos2(1000.0, f32::INFINITY));
        assert_eq!(
            visible_rows(inf_clip, origin_y, row_pitch, rows),
            (0, rows - 1)
        );
    }

    #[test]
    fn drag_cancels_before_threshold() {
        let mut drag = DragMachine::begin(1, pos2(10.0, 10.0));
        drag.pointer(pos2(12.0, 10.0));
        assert!(!drag.is_dragging());
        let rects = card_rects(2, 2);
        let ctx = DragContext {
            grid_area: grid_area(),
            card_rects: &rects,
            cols: 2,
        };
        assert_eq!(drag.release(pos2(12.0, 10.0), &ctx), None);
    }

    #[test]
    fn drag_crosses_threshold_and_drops_inside_grid() {
        let mut drag = DragMachine::begin(1, pos2(10.0, 10.0));
        drag.pointer(pos2(20.0, 20.0));
        assert!(drag.is_dragging());
        let rects = card_rects(4, 2);
        let ctx = DragContext {
            grid_area: grid_area(),
            card_rects: &rects,
            cols: 2,
        };
        let p = pointer_in_vertical_gap(&rects, 1);
        let (id, gap) = drag.release(p, &ctx).expect("drop inside grid");
        assert_eq!(id, 1);
        assert_eq!(gap, 1);
    }

    #[test]
    fn drag_dropping_outside_grid_cancels() {
        let mut drag = DragMachine::begin(1, pos2(10.0, 10.0));
        drag.pointer(pos2(20.0, 20.0));
        let rects = card_rects(4, 2);
        let ctx = DragContext {
            grid_area: grid_area(),
            card_rects: &rects,
            cols: 2,
        };
        assert_eq!(drag.release(pos2(2000.0, 2000.0), &ctx), None);
    }

    #[test]
    fn drag_keeps_tracking_while_dragging() {
        let mut drag = DragMachine::begin(1, pos2(10.0, 10.0));
        drag.pointer(pos2(20.0, 20.0));
        drag.pointer(pos2(25.0, 25.0));
        assert!(drag.is_dragging());
    }
}
