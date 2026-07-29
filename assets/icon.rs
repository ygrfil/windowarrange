pub const SIZE: u32 = 32;

const BORDER: [u8; 4] = [15, 23, 42, 255];
const BACKGROUND: [u8; 4] = [37, 99, 235, 255];
const SPADE: [u8; 4] = [15, 45, 95, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

const SPADE_OUTLINE: [(f32, f32); 18] = [
    (15.5, 2.5),
    (26.5, 13.0),
    (28.0, 17.0),
    (27.0, 20.5),
    (24.5, 23.0),
    (21.0, 24.0),
    (18.0, 22.0),
    (18.5, 25.5),
    (22.5, 28.5),
    (8.5, 28.5),
    (12.5, 25.5),
    (13.0, 22.0),
    (10.0, 24.0),
    (6.5, 23.0),
    (4.0, 20.5),
    (3.0, 17.0),
    (4.5, 13.0),
    (15.5, 2.5),
];

#[must_use]
pub fn rgba_pixel(x: u32, y: u32) -> [u8; 4] {
    let border = x < 2 || y < 2 || x >= SIZE - 2 || y >= SIZE - 2;
    if border {
        return BORDER;
    }

    let point = (x as f32 + 0.5, y as f32 + 0.5);
    if !inside_polygon(point, &SPADE_OUTLINE) {
        return BACKGROUND;
    }

    let plus = ((14..=17).contains(&x) && (9..=22).contains(&y))
        || ((9..=22).contains(&x) && (14..=17).contains(&y));
    if plus { WHITE } else { SPADE }
}

fn inside_polygon(point: (f32, f32), polygon: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for current in 0..polygon.len() {
        let (current_x, current_y) = polygon[current];
        let (previous_x, previous_y) = polygon[previous];
        let crosses = (current_y > point.1) != (previous_y > point.1)
            && point.0
                < (previous_x - current_x) * (point.1 - current_y) / (previous_y - current_y)
                    + current_x;
        if crosses {
            inside = !inside;
        }
        previous = current;
    }
    inside
}
