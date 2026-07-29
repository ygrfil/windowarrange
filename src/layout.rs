use crate::model::Rect;

pub const DEFAULT_ASPECT_RATIO: f64 = 4.0 / 3.0;

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
            let column = index % columns;
            let row = index / columns;
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
    fn five_tables_use_equal_three_by_two_layout() {
        let layout = calculate_layout(Rect::new(0, 0, 2752, 1104), 5, 4.0 / 3.0);
        assert_eq!((layout.columns, layout.rows), (3, 2));
        assert_eq!(layout.rectangles.len(), 5);
        assert!(layout.rectangles.iter().all(|rect| {
            rect.width == layout.table_width && rect.height == layout.table_height
        }));
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
