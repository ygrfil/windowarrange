use clubgg_table_arranger::{
    layout::{PokerColumnSpec, calculate_layout, calculate_mixed_layout, right_side_free_rect},
    model::Rect,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn layouts_are_equal_bounded_and_maximal(
        left in -4000_i32..4000,
        top in -2500_i32..2500,
        width in 320_i32..8000,
        height in 240_i32..4500,
        count in 1_usize..=8,
        ratio in 0.9_f64..2.2,
    ) {
        let work = Rect::new(left, top, width, height);
        let layout = calculate_layout(work, count, ratio);

        prop_assert_eq!(layout.rectangles.len(), count);
        prop_assert_eq!(layout.rectangles[0].left, left);
        prop_assert_eq!(layout.rectangles[0].top, top);
        prop_assert!(layout.table_width > 0);
        prop_assert!(layout.table_height > 0);

        for rect in &layout.rectangles {
            prop_assert_eq!(rect.width, layout.table_width);
            prop_assert_eq!(rect.height, layout.table_height);
            prop_assert!(rect.left >= work.left);
            prop_assert!(rect.top >= work.top);
            prop_assert!(rect.right() <= work.right());
            prop_assert!(rect.bottom() <= work.bottom());
        }

        for (index, left_rect) in layout.rectangles.iter().enumerate() {
            for right_rect in layout.rectangles.iter().skip(index + 1) {
                let separated = left_rect.right() <= right_rect.left
                    || right_rect.right() <= left_rect.left
                    || left_rect.bottom() <= right_rect.top
                    || right_rect.bottom() <= left_rect.top;
                prop_assert!(separated);
            }
        }

        let chosen_area = i64::from(layout.table_width) * i64::from(layout.table_height);
        let layout_count = count.max(4);
        let maximal_area = (1..=layout_count)
            .map(|columns| {
                let rows = layout_count.div_ceil(columns);
                let cell_width = width / i32::try_from(columns).unwrap();
                let cell_height = height / i32::try_from(rows).unwrap();
                let cell_ratio = f64::from(cell_width) / f64::from(cell_height);
                let (table_width, table_height) = if cell_ratio > ratio {
                    ((f64::from(cell_height) * ratio).floor() as i32, cell_height)
                } else {
                    (cell_width, (f64::from(cell_width) / ratio).floor() as i32)
                };
                i64::from(table_width.max(1)) * i64::from(table_height.max(1))
            })
            .max()
            .unwrap();
        prop_assert_eq!(chosen_area, maximal_area);
    }
}

proptest! {
    #[test]
    fn mixed_columns_preserve_order_aspects_and_bounds(
        left in -4000_i32..4000,
        top in -2500_i32..2500,
        width in 640_i32..8000,
        height in 480_i32..4500,
        club_columns in 0_usize..=4,
        ld_columns in 0_usize..=4,
        ld_ratio in 0.4_f64..1.8,
    ) {
        prop_assume!(club_columns + ld_columns > 0);
        let work = Rect::new(left, top, width, height);
        let mut specs = vec![PokerColumnSpec::ClubGg; club_columns];
        specs.extend((0..ld_columns).map(|_| PokerColumnSpec::LdPlayer {
            aspect_ratio: ld_ratio,
        }));
        let layout = calculate_mixed_layout(work, &specs);

        prop_assert_eq!(layout.columns.len(), specs.len());
        prop_assert!(layout.height > 0 && layout.height <= work.height);
        prop_assert_eq!(layout.columns[0].bounds.left, work.left);
        prop_assert!(layout.columns.last().unwrap().bounds.right() <= work.right());
        for pair in layout.columns.windows(2) {
            prop_assert_eq!(pair[0].bounds.right(), pair[1].bounds.left);
        }
        for (spec, column) in specs.iter().zip(&layout.columns) {
            prop_assert_eq!(column.bounds.top, work.top);
            prop_assert_eq!(column.bounds.height, layout.height);
            match spec {
                PokerColumnSpec::ClubGg => {
                    let upper = column.top.unwrap();
                    let lower = column.bottom.unwrap();
                    prop_assert_eq!(upper.bottom(), lower.top);
                    prop_assert_eq!(lower.bottom(), column.bounds.bottom());
                    prop_assert!(
                        (upper.width as f64 / upper.height as f64 - 4.0 / 3.0).abs()
                            <= 1.0 / upper.height as f64
                    );
                }
                PokerColumnSpec::LdPlayer { aspect_ratio } => {
                    prop_assert!(column.top.is_none() && column.bottom.is_none());
                    prop_assert!(
                        (column.bounds.width as f64 / column.bounds.height as f64 - aspect_ratio)
                            .abs()
                            <= 1.0 / column.bounds.height as f64
                    );
                }
            }
        }
    }
}

proptest! {
    #[test]
    fn free_space_is_a_right_side_strip_and_avoids_active_tables(
        left in -4000_i32..4000,
        top in -2500_i32..2500,
        width in 320_i32..8000,
        height in 240_i32..4500,
        count in 0_usize..=8,
        ratio in 0.9_f64..2.2,
    ) {
        let work = Rect::new(left, top, width, height);
        let tables = calculate_layout(work, count, ratio).rectangles;
        let free = right_side_free_rect(work, &tables);

        prop_assert!(free.left >= work.left);
        prop_assert_eq!(free.top, work.top);
        prop_assert!(free.right() <= work.right());
        prop_assert_eq!(free.bottom(), work.bottom());
        for table in tables {
            prop_assert!(table.right() <= free.left);
        }
    }
}

#[test]
fn expected_grid_shapes_for_current_monitor() {
    let work = Rect::new(0, 0, 2752, 1104);
    let expected = [
        (2, 2),
        (2, 2),
        (2, 2),
        (2, 2),
        (3, 2),
        (3, 2),
        (4, 2),
        (4, 2),
    ];
    for (count, expected_grid) in (1..=8).zip(expected) {
        let layout = calculate_layout(work, count, 4.0 / 3.0);
        assert_eq!((layout.columns, layout.rows), expected_grid);
    }
}
