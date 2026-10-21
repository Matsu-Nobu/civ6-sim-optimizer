//! Hex geometry of odd-r offset coordinates (yields.neighbour_xy / hex_distance; world.Map for the wrapping map).
use crate::model::XY;

/// The width of the loaded map, whose west and east edges meet (0: not set, no wrap).
static MAP_WIDTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub fn set_map_width(w: i32) {
    MAP_WIDTH.store(w, std::sync::atomic::Ordering::Relaxed);
}

/// The six neighbours in the game's order, across the map's west and east edges.
pub fn neighbour_xy(x: i32, y: i32) -> [XY; 6] {
    let n = if y & 1 == 1 {
        [(x + 1, y), (x - 1, y), (x, y - 1), (x + 1, y - 1), (x, y + 1), (x + 1, y + 1)]
    } else {
        [(x + 1, y), (x - 1, y), (x - 1, y - 1), (x, y - 1), (x - 1, y + 1), (x, y + 1)]
    };
    let w = MAP_WIDTH.load(std::sync::atomic::Ordering::Relaxed);
    if w > 0 { n.map(|(a, b)| (a.rem_euclid(w), b)) } else { n }
}

fn cube(x: i32, y: i32) -> (i32, i32, i32) {
    let q = x - (y - (y & 1)).div_euclid(2);
    (q, y, -q - y)
}

/// Distance, across the map's west and east edges.
pub fn hex_distance(a: XY, b: XY) -> i32 {
    let one = |bx: i32| {
        let (q1, r1, s1) = cube(a.0, a.1);
        let (q2, r2, s2) = cube(bx, b.1);
        (q1 - q2).abs().max((r1 - r2).abs()).max((s1 - s2).abs())
    };
    let w = MAP_WIDTH.load(std::sync::atomic::Ordering::Relaxed);
    if w > 0 { one(b.0).min(one(b.0 - w)).min(one(b.0 + w)) } else { one(b.0) }
}
