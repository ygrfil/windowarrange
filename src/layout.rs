use crate::model::Rect;

pub const DEFAULT_ASPECT_RATIO: f64 = 4.0 / 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PokerColumnSpec {
    ClubGg,
    LdPlayer { aspect_ratio: f64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PokerColumnLayout {
    pub bounds: Rect,
    pub top: Option<Rect>,
    pub bottom: Option<Rect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MixedPokerLayout {
    pub height: i32,
    pub columns: Vec<PokerColumnLayout>,
}

#[must_use]
pub fn calculate_mixed_layout(work_area: Rect, columns: &[PokerColumnSpec]) -> MixedPokerLayout {
    if columns.is_empty() || work_area.width <= 0 || work_area.height <= 0 {
        return MixedPokerLayout {
            height: 0,
            columns: Vec::new(),
        };
    }

    let width_per_height: f64 = columns
        .iter()
        .map(|column| match column {
            PokerColumnSpec::ClubGg => DEFAULT_ASPECT_RATIO / 2.0,
            PokerColumnSpec::LdPlayer { aspect_ratio } => valid_ratio(*aspect_ratio),
        })
        .sum();
    let height_for_width = (f64::from(work_area.width) / width_per_height).floor() as i32;
    let mut height = work_area.height.min(height_for_width).max(2);
    if columns
        .iter()
        .any(|column| matches!(column, PokerColumnSpec::ClubGg))
    {
        height -= height.rem_euclid(2);
    }
    height = height.max(2);

    let mut left = work_area.left;
    let mut layouts = Vec::with_capacity(columns.len());
    for column in columns {
        let (width, top, bottom) = match column {
            PokerColumnSpec::ClubGg => {
                let cell_height = height / 2;
                let width = (f64::from(cell_height) * DEFAULT_ASPECT_RATIO).floor() as i32;
                (
                    width.max(1),
                    Some(Rect::new(left, work_area.top, width.max(1), cell_height)),
                    Some(Rect::new(
                        left,
                        work_area.top.saturating_add(cell_height),
                        width.max(1),
                        cell_height,
                    )),
                )
            }
            PokerColumnSpec::LdPlayer { aspect_ratio } => {
                let width = (f64::from(height) * valid_ratio(*aspect_ratio)).floor() as i32;
                (width.max(1), None, None)
            }
        };
        let bounds = Rect::new(left, work_area.top, width, height);
        layouts.push(PokerColumnLayout {
            bounds,
            top,
            bottom,
        });
        left = left.saturating_add(width);
    }

    MixedPokerLayout {
        height,
        columns: layouts,
    }
}

fn valid_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() && (0.25..=4.0).contains(&ratio) {
        ratio
    } else {
        DEFAULT_ASPECT_RATIO
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub rows: usize,
    pub table_width: i32,
    pub table_height: i32,
    pub rectangles: Vec<Rect>,
}

impl Layout {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            columns: 0,
            rows: 0,
            table_width: 0,
            table_height: 0,
            rectangles: Vec::new(),
        }
    }
}

#[must_use]
pub fn calculate_layout(work_area: Rect, count: usize, aspect_ratio: f64) -> Layout {
    if count == 0 || work_area.width <= 0 || work_area.height <= 0 {
        return Layout::empty();
    }

    let ratio = if aspect_ratio.is_finite() && (0.25..=4.0).contains(&aspect_ratio) {
        aspect_ratio
    } else {
        DEFAULT_ASPECT_RATIO
    };
    let layout_count = count.max(4);

    let mut best: Option<(i64, usize, usize, i32, i32)> = None;
    for columns in 1..=layout_count {
        let rows = layout_count.div_ceil(columns);
        let cell_width = work_area.width / i32::try_from(columns).unwrap_or(i32::MAX);
        let cell_height = work_area.height / i32::try_from(rows).unwrap_or(i32::MAX);
        if cell_width <= 0 || cell_height <= 0 {
            continue;
        }

        let cell_ratio = f64::from(cell_width) / f64::from(cell_height);
        let (width, height) = if cell_ratio > ratio {
            ((f64::from(cell_height) * ratio).floor() as i32, cell_height)
        } else {
            (cell_width, (f64::from(cell_width) / ratio).floor() as i32)
        };
        let width = width.max(1);
        let height = height.max(1);
        let area = i64::from(width) * i64::from(height);
        let slots = columns * rows;

        let replace = best.is_none_or(|(best_area, best_columns, best_rows, _, _)| {
            area > best_area
                || (area == best_area
                    && (slots < best_columns * best_rows
                        || (slots == best_columns * best_rows && rows < best_rows)))
        });
        if replace {
            best = Some((area, columns, rows, width, height));
        }
    }

    let Some((_, columns, rows, table_width, table_height)) = best else {
        return Layout::empty();
    };

    let rectangles = (0..count)
        .map(|index| {
            let (column, row) = grid_position(index, count, columns, rows);
            Rect::new(
                work_area.left
                    + i32::try_from(column)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(table_width),
                work_area.top
                    + i32::try_from(row)
                        .unwrap_or(i32::MAX)
                        .saturating_mul(table_height),
                table_width,
                table_height,
            )
        })
        .collect();

    Layout {
        columns,
        rows,
        table_width,
        table_height,
        rectangles,
    }
}

fn grid_position(index: usize, count: usize, columns: usize, rows: usize) -> (usize, usize) {
    if count <= 4 || columns < 2 || rows < 2 {
        return (index % columns, index / columns);
    }

    const FIRST_FOUR: [(usize, usize); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];
    if index < FIRST_FOUR.len() {
        return FIRST_FOUR[index];
    }

    let remaining = index - FIRST_FOUR.len();
    let right_side_slots = columns.saturating_sub(2) * rows;
    if remaining < right_side_slots {
        return (2 + remaining / rows, remaining % rows);
    }

    let below_first_grid = remaining - right_side_slots;
    (below_first_grid % 2, 2 + below_first_grid / 2)
}

#[must_use]
pub fn right_side_free_rect(work_area: Rect, occupied: &[Rect]) -> Rect {
    if work_area.width <= 0 || work_area.height <= 0 {
        return Rect::default();
    }

    let rightmost_occupied = occupied
        .iter()
        .filter_map(|rect| intersection(*rect, work_area))
        .map(|rect| rect.right())
        .max()
        .unwrap_or(work_area.left)
        .clamp(work_area.left, work_area.right());

    Rect::new(
        rightmost_occupied,
        work_area.top,
        work_area.right().saturating_sub(rightmost_occupied),
        work_area.height,
    )
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let clipped_left = left.left.max(right.left);
    let clipped_top = left.top.max(right.top);
    let clipped_right = left.right().min(right.right());
    let clipped_bottom = left.bottom().min(right.bottom());
    (clipped_right > clipped_left && clipped_bottom > clipped_top).then(|| {
        Rect::new(
            clipped_left,
            clipped_top,
            clipped_right - clipped_left,
            clipped_bottom - clipped_top,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_columns_keep_clubgg_paired_and_ldplayer_full_height() {
        let work = Rect::new(0, 0, 2752, 1104);
        let layout = calculate_mixed_layout(
            work,
            &[
                PokerColumnSpec::ClubGg,
                PokerColumnSpec::ClubGg,
                PokerColumnSpec::LdPlayer {
                    aspect_ratio: 9.0 / 16.0,
                },
            ],
        );

        assert_eq!(layout.height, work.height);
        assert_eq!(layout.columns.len(), 3);
        for column in &layout.columns[..2] {
            let top = column.top.unwrap();
            let bottom = column.bottom.unwrap();
            assert_eq!(top.height, work.height / 2);
            assert_eq!(bottom.top, top.bottom());
            assert_eq!(top.width as f64 / top.height as f64, DEFAULT_ASPECT_RATIO);
        }
        let ldplayer = &layout.columns[2];
        assert!(ldplayer.top.is_none() && ldplayer.bottom.is_none());
        assert_eq!(ldplayer.bounds.height, work.height);
        assert_eq!(ldplayer.bounds.left, layout.columns[1].bounds.right());
    }

    #[test]
    fn crowded_mixed_columns_shrink_together_without_overlap() {
        let work = Rect::new(0, 0, 1920, 1040);
        let layout = calculate_mixed_layout(
            work,
            &[
                PokerColumnSpec::ClubGg,
                PokerColumnSpec::ClubGg,
                PokerColumnSpec::ClubGg,
                PokerColumnSpec::LdPlayer {
                    aspect_ratio: 9.0 / 16.0,
                },
                PokerColumnSpec::LdPlayer {
                    aspect_ratio: 9.0 / 16.0,
                },
            ],
        );

        assert!(layout.height < work.height);
        assert_eq!(layout.height % 2, 0);
        assert!(layout.columns.last().unwrap().bounds.right() <= work.right());
        for pair in layout.columns.windows(2) {
            assert_eq!(pair[0].bounds.right(), pair[1].bounds.left);
        }
    }

    #[test]
    fn five_tables_use_equal_three_by_two_layout() {
        let layout = calculate_layout(Rect::new(0, 0, 2752, 1104), 5, 4.0 / 3.0);
        assert_eq!((layout.columns, layout.rows), (3, 2));
        assert_eq!(layout.rectangles.len(), 5);
        assert!(layout.rectangles.iter().all(|rect| {
            rect.width == layout.table_width && rect.height == layout.table_height
        }));
    }

    #[test]
    fn tables_above_four_extend_the_two_by_two_grid_in_vertical_pairs() {
        let work_area = Rect::new(0, 0, 2752, 1104);
        let expected = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (2, 0),
            (2, 1),
            (3, 0),
            (3, 1),
        ];

        for count in 5..=8 {
            let layout = calculate_layout(work_area, count, 4.0 / 3.0);
            let positions: Vec<_> = layout
                .rectangles
                .iter()
                .map(|rect| {
                    (
                        usize::try_from((rect.left - work_area.left) / layout.table_width).unwrap(),
                        usize::try_from((rect.top - work_area.top) / layout.table_height).unwrap(),
                    )
                })
                .collect();
            assert_eq!(positions, expected[..count]);
        }
    }

    #[test]
    fn invalid_ratio_falls_back() {
        let a = calculate_layout(Rect::new(0, 0, 1920, 1040), 4, f64::NAN);
        let b = calculate_layout(Rect::new(0, 0, 1920, 1040), 4, DEFAULT_ASPECT_RATIO);
        assert_eq!(a, b);
    }

    #[test]
    fn one_to_four_tables_share_the_four_table_cell_size() {
        let work_area = Rect::new(0, 0, 2752, 1104);
        let four = calculate_layout(work_area, 4, 4.0 / 3.0);
        assert_eq!((four.columns, four.rows), (2, 2));

        for count in 1..=3 {
            let layout = calculate_layout(work_area, count, 4.0 / 3.0);
            assert_eq!((layout.columns, layout.rows), (four.columns, four.rows));
            assert_eq!(
                (layout.table_width, layout.table_height),
                (four.table_width, four.table_height)
            );
            assert_eq!(layout.rectangles, four.rectangles[..count]);
        }
    }

    #[test]
    fn free_rectangle_uses_only_the_right_side_of_tables() {
        let work_area = Rect::new(0, 0, 1200, 800);
        let occupied = [
            Rect::new(0, 0, 400, 300),
            Rect::new(400, 0, 400, 300),
            Rect::new(0, 300, 400, 300),
        ];

        assert_eq!(
            right_side_free_rect(work_area, &occupied),
            Rect::new(800, 0, 400, 800)
        );
    }

    #[test]
    fn entire_work_area_is_free_without_tables() {
        let work_area = Rect::new(-1920, 0, 1920, 1040);
        assert_eq!(right_side_free_rect(work_area, &[]), work_area);
    }

    #[test]
    fn two_tables_never_select_the_larger_bottom_band() {
        let work_area = Rect::new(0, 0, 2752, 1104);
        let tables = calculate_layout(work_area, 2, 4.0 / 3.0).rectangles;
        let free = right_side_free_rect(work_area, &tables);

        assert_eq!(free.top, work_area.top);
        assert_eq!(free.height, work_area.height);
        assert_eq!(
            free.left,
            tables
                .iter()
                .map(|rect| rect.right())
                .max()
                .unwrap_or_default()
        );
    }
}
