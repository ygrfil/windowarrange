use clubgg_table_arranger::{
    layout::{PokerColumnSpec, calculate_mixed_layout, right_side_free_rect},
    model::Rect,
};
use proptest::prelude::*;

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
        club_columns in 0_usize..=4,
        ld_columns in 0_usize..=4,
        ld_ratio in 0.4_f64..1.8,
    ) {
        let work = Rect::new(left, top, width, height);
        let mut specs = vec![PokerColumnSpec::ClubGg; club_columns];
        specs.extend((0..ld_columns).map(|_| PokerColumnSpec::LdPlayer {
            aspect_ratio: ld_ratio,
        }));
        let tables: Vec<_> = calculate_mixed_layout(work, &specs)
            .columns
            .into_iter()
            .map(|column| column.bounds)
            .collect();
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
