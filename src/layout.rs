use crate::model::Rect;

pub const DEFAULT_ASPECT_RATIO: f64 = 4.0 / 3.0;
pub const LDPLAYER_NATURAL_ASPECT_RATIO: f64 = 9.0 / 16.0;

#[must_use]
pub fn normalized_ldplayer_aspect_ratio(detected: f64) -> f64 {
    if detected.is_finite() && detected > 0.0 {
        detected.max(LDPLAYER_NATURAL_ASPECT_RATIO)
    } else {
        LDPLAYER_NATURAL_ASPECT_RATIO
    }
}

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
    fn ldplayer_ratio_keeps_the_natural_portrait_width_as_its_minimum() {
        assert_eq!(normalized_ldplayer_aspect_ratio(0.50), 9.0 / 16.0);
        assert_eq!(normalized_ldplayer_aspect_ratio(0.65), 0.65);
        assert_eq!(normalized_ldplayer_aspect_ratio(f64::NAN), 9.0 / 16.0);
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
}
