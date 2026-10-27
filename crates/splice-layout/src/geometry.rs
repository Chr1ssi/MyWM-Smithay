/// Axis-aligned rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Edges grabbed during an interactive resize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum DragKind {
    Move,
    Resize(Edges),
}

impl Rect {
    pub fn centered(width: i32, height: i32) -> Self {
        let w = (width / 3 * 2).max(1);
        let h = (height / 3 * 2).max(1);
        Self {
            x: (width - w) / 2,
            y: (height - h) / 2,
            width: w,
            height: h,
        }
    }

    pub fn constrained(self, width: i32, height: i32) -> Self {
        let w = self.width.clamp(1, width.max(1));
        let h = self.height.clamp(1, height.max(1));
        Self {
            x: self.x.clamp(0, (width - w).max(0)),
            y: self.y.clamp(0, (height - h).max(0)),
            width: w,
            height: h,
        }
    }

    /// Deltas are cumulative from the start, never incremental between events.
    pub fn dragged(self, kind: DragKind, dx: i32, dy: i32, width: i32, height: i32) -> Self {
        let mut result = self.constrained(width, height);
        match kind {
            DragKind::Move => {
                result.x = result.x.saturating_add(dx);
                result.y = result.y.saturating_add(dy);
            }
            DragKind::Resize(edges) => {
                let right = result.x + result.width;
                let bottom = result.y + result.height;
                if edges.left {
                    result.x = result.x.saturating_add(dx).clamp(0, (right - 100).max(0));
                    result.width = right - result.x;
                } else if edges.right {
                    let max = (width - result.x).max(1);
                    result.width = result.width.saturating_add(dx).clamp(100.min(max), max);
                }
                if edges.top {
                    result.y = result.y.saturating_add(dy).clamp(0, (bottom - 60).max(0));
                    result.height = bottom - result.y;
                } else if edges.bottom {
                    let max = (height - result.y).max(1);
                    result.height = result.height.saturating_add(dy).clamp(60.min(max), max);
                }
            }
        }
        result.constrained(width, height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Rect = Rect { x: 100, y: 100, width: 400, height: 300 };
    const TOP_LEFT: Edges = Edges { left: true, top: true, right: false, bottom: false };
    const BOTTOM_RIGHT: Edges = Edges { right: true, bottom: true, left: false, top: false };

    #[test]
    fn move_is_bounded_and_cumulative() {
        assert_eq!(RECT.dragged(DragKind::Move, 50, 20, 1000, 800).x, 150);
        assert_eq!(RECT.dragged(DragKind::Move, 70, 30, 1000, 800).x, 170);
        let moved = RECT.dragged(DragKind::Move, i32::MAX, i32::MIN, 1000, 800);
        assert_eq!((moved.x, moved.y), (600, 0));
    }

    #[test]
    fn resize_anchors_opposite_edges_and_limits_size() {
        let resized = RECT.dragged(DragKind::Resize(TOP_LEFT), 50, 20, 1000, 800);
        assert_eq!(resized, Rect { x: 150, y: 120, width: 350, height: 280 });
        let small = RECT.dragged(DragKind::Resize(BOTTOM_RIGHT), -999, -999, 1000, 800);
        assert_eq!((small.width, small.height), (100, 60));
        let large = RECT.dragged(DragKind::Resize(BOTTOM_RIGHT), 999, 999, 1000, 800);
        assert_eq!((large.width, large.height), (900, 700));
    }

    #[test]
    fn hotplug_and_tiny_outputs_remain_valid() {
        let rect = Rect { x: 1500, y: 900, width: 900, height: 600 };
        assert_eq!(rect.constrained(800, 500), Rect { x: 0, y: 0, width: 800, height: 500 });
        let tiny = Rect::centered(1, 1).dragged(DragKind::Resize(TOP_LEFT), 99, 99, 1, 1);
        assert_eq!(tiny, Rect { x: 0, y: 0, width: 1, height: 1 });
    }
}
