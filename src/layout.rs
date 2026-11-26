//! Where a dropped window goes, as Windows and macOS do it: drag a window
//! to the left or right edge of the screen and it takes that half; to a
//! corner, that quarter; to the top, the whole screen. Drop it near an
//! edge of a window already tiled, and the two share that window's place.
//! Pure geometry, so it is tested without a server.

use protolith::Rect;

/// How close to the screen's edge the pointer must be (it stops at the
/// edge, so on a real screen this is "touching it").
pub const EDGE: i32 = 2;
/// Along an edge, how far from a corner still means the corner.
pub const CORNER: i32 = 48;
/// How far inside a tiled window's edge a drop splits it there.
pub const SPLIT: i32 = 48;

/// A side of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Top.
    Top,
    /// Bottom.
    Bottom,
}

/// A place on the screen a dragged window can be dropped into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The whole screen.
    Maximize,
    /// Half the screen.
    Half(Side),
    /// A quarter: (left or right, top or bottom).
    Quarter(Side, Side),
}

/// The zone the pointer at `p` touches, on a screen `screen`.
pub fn edge_zone(p: (i32, i32), screen: Rect) -> Option<Zone> {
    let (x, y) = (p.0 - screen.x, p.1 - screen.y);
    let (w, h) = (screen.width as i32, screen.height as i32);
    let (left, right, top) = (x < EDGE, x >= w - EDGE, y < EDGE);
    let vertical = if y < CORNER {
        Some(Side::Top)
    } else if y >= h - CORNER {
        Some(Side::Bottom)
    } else {
        None
    };
    let horizontal = if x < CORNER {
        Some(Side::Left)
    } else if x >= w - CORNER {
        Some(Side::Right)
    } else {
        None
    };
    if left || right {
        let side = if left { Side::Left } else { Side::Right };
        return Some(match vertical {
            Some(v) => Zone::Quarter(side, v),
            None => Zone::Half(side),
        });
    }
    if top {
        return Some(match horizontal {
            Some(s) => Zone::Quarter(s, Side::Top),
            None => Zone::Maximize,
        });
    }
    None
}

/// The rectangle of a zone in the work area `area`.
pub fn zone_rect(zone: Zone, area: Rect) -> Rect {
    match zone {
        Zone::Maximize => area,
        Zone::Half(side) => half(area, side),
        Zone::Quarter(h, v) => half(half(area, h), v),
    }
}

/// One half of `r`.
pub fn half(r: Rect, side: Side) -> Rect {
    let (w1, h1) = (r.width / 2, r.height / 2);
    match side {
        Side::Left => Rect::new(r.x, r.y, w1, r.height),
        Side::Right => Rect::new(r.x + w1 as i32, r.y, r.width - w1, r.height),
        Side::Top => Rect::new(r.x, r.y, r.width, h1),
        Side::Bottom => Rect::new(r.x, r.y + h1 as i32, r.width, r.height - h1),
    }
}

/// The other side.
pub fn opposite(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
    }
}

/// Which edge of a tiled window `r` the pointer at `p` is just inside
/// (within [`SPLIT`], or a quarter of a small window): a drop there gives
/// that half to the window dropped, the other half to `r`'s.
pub fn split_side(p: (i32, i32), r: Rect) -> Option<Side> {
    if !r.contains(p.0, p.1) {
        return None;
    }
    let band_x = SPLIT.min(r.width as i32 / 4);
    let band_y = SPLIT.min(r.height as i32 / 4);
    let d = [
        (p.0 - r.x, band_x, Side::Left),
        (r.x + r.width as i32 - 1 - p.0, band_x, Side::Right),
        (p.1 - r.y, band_y, Side::Top),
        (r.y + r.height as i32 - 1 - p.1, band_y, Side::Bottom),
    ];
    d.iter().filter(|(dist, band, _)| dist < band).min_by_key(|(dist, _, _)| *dist).map(|&(_, _, s)| s)
}

/// `r` scaled from a screen `from` to a screen `to` (the screen changed
/// size: tiled windows keep their share of it).
pub fn rescale(r: Rect, from: Rect, to: Rect) -> Rect {
    let sx = |v: i64| (v * i64::from(to.width) / i64::from(from.width.max(1))) as i32;
    let sy = |v: i64| (v * i64::from(to.height) / i64::from(from.height.max(1))) as i32;
    let (x0, y0) = (sx(i64::from(r.x - from.x)), sy(i64::from(r.y - from.y)));
    let (x1, y1) = (sx(i64::from(r.x - from.x) + i64::from(r.width)), sy(i64::from(r.y - from.y) + i64::from(r.height)));
    Rect::new(to.x + x0, to.y + y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)
}

/// Where a window `size` big goes when it leaves a tile under the pointer
/// at `p`: under the pointer at the same fraction of its width as the
/// pointer was across the tile (`frac`, 0..=1000), `grab_y` down its title.
pub fn unsnap(size: (u32, u32), p: (i32, i32), frac: u32, grab_y: i32) -> Rect {
    let x = p.0 - (i64::from(size.0) * i64::from(frac.min(1000)) / 1000) as i32;
    Rect::new(x, p.1 - grab_y, size.0, size.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1024, 768);

    #[test]
    fn the_edges_and_corners_of_the_screen() {
        assert_eq!(edge_zone((0, 400), SCREEN), Some(Zone::Half(Side::Left)));
        assert_eq!(edge_zone((1023, 400), SCREEN), Some(Zone::Half(Side::Right)));
        assert_eq!(edge_zone((500, 0), SCREEN), Some(Zone::Maximize));
        assert_eq!(edge_zone((0, 10), SCREEN), Some(Zone::Quarter(Side::Left, Side::Top)));
        assert_eq!(edge_zone((10, 0), SCREEN), Some(Zone::Quarter(Side::Left, Side::Top)), "the top edge near a corner");
        assert_eq!(edge_zone((1023, 760), SCREEN), Some(Zone::Quarter(Side::Right, Side::Bottom)));
        assert_eq!(edge_zone((1000, 0), SCREEN), Some(Zone::Quarter(Side::Right, Side::Top)));
        assert_eq!(edge_zone((500, 767), SCREEN), None, "the bottom edge is no zone");
        assert_eq!(edge_zone((5, 400), SCREEN), None, "near is not touching");
        // A screen not at the origin (the second of two).
        let right = Rect::new(1024, 0, 800, 600);
        assert_eq!(edge_zone((1024, 300), right), Some(Zone::Half(Side::Left)));
    }

    #[test]
    fn zones_divide_the_work_area_without_gaps() {
        assert_eq!(zone_rect(Zone::Half(Side::Left), SCREEN), Rect::new(0, 0, 512, 768));
        assert_eq!(zone_rect(Zone::Half(Side::Right), SCREEN), Rect::new(512, 0, 512, 768));
        assert_eq!(zone_rect(Zone::Quarter(Side::Right, Side::Bottom), SCREEN), Rect::new(512, 384, 512, 384));
        assert_eq!(zone_rect(Zone::Maximize, SCREEN), SCREEN);
        // Odd sizes: the halves still add up.
        let odd = Rect::new(0, 0, 1001, 501);
        let (l, r) = (half(odd, Side::Left), half(odd, Side::Right));
        assert_eq!((l.width + r.width, r.x), (1001, 500));
        let (t, b) = (half(odd, Side::Top), half(odd, Side::Bottom));
        assert_eq!((t.height + b.height, b.y), (501, 250));
    }

    #[test]
    fn dropping_near_a_tiled_windows_edge_splits_it() {
        let tile = Rect::new(512, 0, 512, 768);
        assert_eq!(split_side((520, 400), tile), Some(Side::Left));
        assert_eq!(split_side((1020, 400), tile), Some(Side::Right));
        assert_eq!(split_side((700, 5), tile), Some(Side::Top));
        assert_eq!(split_side((700, 760), tile), Some(Side::Bottom));
        assert_eq!(split_side((700, 400), tile), None, "the middle: no split");
        assert_eq!(split_side((100, 400), tile), None, "outside");
        assert_eq!(split_side((513, 2), tile), Some(Side::Left), "the nearest edge wins");
        // A small tile: bands are a quarter of it.
        let small = Rect::new(0, 0, 80, 80);
        assert_eq!(split_side((25, 40), small), None);
        assert_eq!(split_side((15, 40), small), Some(Side::Left));
    }

    #[test]
    fn a_new_screen_size_keeps_each_tiles_share() {
        let to = Rect::new(0, 0, 800, 600);
        assert_eq!(rescale(Rect::new(512, 0, 512, 768), SCREEN, to), Rect::new(400, 0, 400, 600));
        assert_eq!(rescale(Rect::new(0, 384, 512, 384), SCREEN, to), Rect::new(0, 300, 400, 300));
    }

    #[test]
    fn leaving_a_tile_keeps_the_grab_under_the_pointer() {
        // Grabbed a third of the way across a tile; the floating size is 300 wide.
        assert_eq!(unsnap((300, 200), (400, 10), 333, 8), Rect::new(301, 2, 300, 200));
        assert_eq!(opposite(Side::Top), Side::Bottom);
    }
}
