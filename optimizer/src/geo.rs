//! Hex geometry of odd-r offset coordinates (yields.neighbour_xy / hex_distance; world.Map for the wrapping map).
use crate::model::XY;

/// The six neighbours in the game's order (no wrap).
pub fn neighbour_xy(x: i32, y: i32) -> [XY; 6] {
    if y & 1 == 1 {
        [(x + 1, y), (x - 1, y), (x, y - 1), (x + 1, y - 1), (x, y + 1), (x + 1, y + 1)]
    } else {
        [(x + 1, y), (x - 1, y), (x - 1, y - 1), (x, y - 1), (x - 1, y + 1), (x, y + 1)]
    }
}

fn cube(x: i32, y: i32) -> (i32, i32, i32) {
    let q = x - (y - (y & 1)).div_euclid(2);
    (q, y, -q - y)
}

/// Distance without wrapping.
pub fn hex_distance(a: XY, b: XY) -> i32 {
    let (q1, r1, s1) = cube(a.0, a.1);
    let (q2, r2, s2) = cube(b.0, b.1);
    (q1 - q2).abs().max((r1 - r2).abs()).max((s1 - s2).abs())
}
