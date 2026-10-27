//! Per-workspace placement: scrolling columns, floating windows, fullscreen.
use crate::{Appearance, Rect, Workspace, scrolling};

/// What the layout needs to know about a window.
#[derive(Clone, Debug)]
pub struct WindowInfo<Id> {
    pub id: Id,
    pub floating: bool,
    pub fullscreen: bool,
    /// User-chosen column width; `None` uses the default.
    pub tiled_width: Option<i32>,
    /// Floating geometry relative to the work area.
    pub floating_rect: Option<Rect>,
    /// Transient parent (dialogs), used to keep the parent's column visible.
    pub parent: Option<Id>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement<Id> {
    pub id: Id,
    /// Window content in output-local logical coordinates.
    pub content: Rect,
    pub border: i32,
    pub fullscreen: bool,
    /// Constrained floating rect the caller should store back.
    pub floating_rect: Option<Rect>,
}

/// Place one floating window; its rect is constrained to the work area.
pub fn place_floating<Id: Clone>(
    window: &WindowInfo<Id>,
    work_area: Rect,
    appearance: &Appearance,
) -> Placement<Id> {
    let rect = window
        .floating_rect
        .unwrap_or_else(|| Rect::centered(work_area.width, work_area.height))
        .constrained(work_area.width, work_area.height);
    let (content, border) = appearance.content(Rect {
        x: work_area.x + rect.x,
        y: work_area.y + rect.y,
        width: rect.width,
        height: rect.height,
    });
    Placement { id: window.id.clone(), content, border, fullscreen: false, floating_rect: Some(rect) }
}

/// Compute placements for the windows of `workspace` on an output whose usable
/// area is `work_area` and whose full area (covered by fullscreen windows) is
/// `output_area`, both in global coordinates. Updates the workspace's scroll offset.
/// Tiled windows come first, then floating, then fullscreen (paint order).
pub fn arrange<Id: Clone + Eq>(
    workspace: &mut Workspace<Id>,
    windows: &[WindowInfo<Id>],
    work_area: Rect,
    output_area: Rect,
    appearance: &Appearance,
) -> Vec<Placement<Id>> {
    let info = |id: &Id| windows.iter().find(|w| &w.id == id);
    let in_workspace: Vec<&WindowInfo<Id>> = workspace.windows.iter().filter_map(info).collect();
    // Fullscreen windows keep their column so the layout underneath is preserved.
    let all_tiled: Vec<&WindowInfo<Id>> = in_workspace.iter().copied().filter(|w| !w.floating).collect();

    // Keep a focused dialog's tiled ancestor visible behind it.
    let mut scroll_focus = workspace.focused.clone();
    let mut focused = None;
    for _ in 0..=windows.len() {
        focused = all_tiled.iter().position(|w| Some(&w.id) == scroll_focus.as_ref());
        if focused.is_some() || scroll_focus.is_none() {
            break;
        }
        scroll_focus = scroll_focus.as_ref().and_then(info).and_then(|w| w.parent.clone());
    }

    let viewport = appearance.viewport(work_area);
    let widths: Vec<_> = all_tiled.iter().map(|w| w.tiled_width).collect();
    let (scroll, columns) = scrolling::variable_columns(
        viewport.width,
        &widths,
        focused,
        workspace.scroll,
        appearance.gaps_inner,
    );
    workspace.scroll = scroll;

    let mut placements = Vec::new();
    for (window, (left, column_width)) in all_tiled.iter().zip(columns) {
        if window.fullscreen {
            continue;
        }
        let (content, border) = appearance.content(Rect {
            x: viewport.x + left,
            y: viewport.y,
            width: column_width,
            height: viewport.height,
        });
        placements.push(Placement {
            id: window.id.clone(),
            content,
            border,
            fullscreen: false,
            floating_rect: None,
        });
    }

    for window in in_workspace.iter().filter(|w| w.floating && !w.fullscreen) {
        placements.push(place_floating(window, work_area, appearance));
    }

    // Only the workspace's focused window may be fullscreen; others keep their normal place.
    for window in in_workspace.iter().filter(|w| w.fullscreen) {
        let shown = workspace.focused.as_ref() == Some(&window.id);
        if shown {
            placements.push(Placement {
                id: window.id.clone(),
                content: output_area,
                border: 0,
                fullscreen: true,
                floating_rect: window.floating_rect,
            });
        }
    }
    placements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: u32) -> WindowInfo<u32> {
        WindowInfo { id, floating: false, fullscreen: false, tiled_width: None, floating_rect: None, parent: None }
    }

    fn plain() -> Appearance {
        Appearance { gaps_inner: 0, gaps_outer: 0, border_width: 0, ..Default::default() }
    }

    const AREA: Rect = Rect { x: 0, y: 0, width: 1920, height: 1080 };

    fn workspace(ids: &[u32], focus: u32) -> Workspace<u32> {
        Workspace { windows: ids.to_vec(), focused: Some(focus), ..Default::default() }
    }

    #[test]
    fn two_columns_split_and_third_scrolls_focus_into_view() {
        let mut ws = workspace(&[1, 2, 3], 3);
        let windows = [info(1), info(2), info(3)];
        let p = arrange(&mut ws, &windows, AREA, AREA, &plain());
        assert_eq!(ws.scroll, 960);
        assert_eq!(p[2].content, Rect { x: 960, y: 0, width: 960, height: 1080 });
        assert_eq!(p[0].content.x, -960);
    }

    #[test]
    fn floating_windows_are_centered_and_constrained() {
        let mut ws = workspace(&[1], 1);
        let mut w = info(1);
        w.floating = true;
        w.floating_rect = Some(Rect { x: 5000, y: 5000, width: 400, height: 300 });
        let p = arrange(&mut ws, &[w], AREA, AREA, &plain());
        assert_eq!(p[0].content, Rect { x: 1520, y: 780, width: 400, height: 300 });
        assert_eq!(p[0].floating_rect, Some(p[0].content));
    }

    #[test]
    fn fullscreen_covers_work_area_and_frees_its_column() {
        let mut ws = workspace(&[1, 2], 2);
        let mut fs = info(2);
        fs.fullscreen = true;
        let p = arrange(&mut ws, &[info(1), fs], AREA, AREA, &plain());
        assert_eq!(p.last().unwrap().content, AREA);
        assert!(p.last().unwrap().fullscreen);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn dialog_keeps_parent_column_visible() {
        let mut ws = workspace(&[1, 2, 3, 4], 4);
        let mut dialog = info(4);
        dialog.floating = true;
        dialog.parent = Some(1);
        let windows = [info(1), info(2), info(3), dialog];
        arrange(&mut ws, &windows, AREA, AREA, &plain());
        assert_eq!(ws.scroll, 0);
    }

    #[test]
    fn panels_shrink_tiles_but_not_fullscreen() {
        let usable = Rect { x: 0, y: 30, width: 1920, height: 1050 };
        let mut ws = workspace(&[1, 2], 2);
        let mut fs = info(2);
        fs.fullscreen = true;
        let p = arrange(&mut ws, &[info(1), fs], usable, AREA, &plain());
        assert_eq!(p[0].content.y, 30);
        assert_eq!(p[0].content.height, 1050);
        assert_eq!(p.last().unwrap().content, AREA, "fullscreen covers the panel too");
        let mut ws = workspace(&[1], 1);
        let mut floating = info(1);
        floating.floating = true;
        floating.floating_rect = Some(Rect { x: 0, y: 0, width: 100, height: 100 });
        let p = arrange(&mut ws, &[floating], usable, AREA, &plain());
        assert_eq!(p[0].content.y, 30, "floating windows stay below the panel");
    }

    #[test]
    fn borders_shrink_content() {
        let mut ws = workspace(&[1], 1);
        let p = arrange(&mut ws, &[info(1)], AREA, AREA, &Appearance::default());
        assert_eq!(p[0].border, 2);
        assert_eq!(p[0].content, Rect { x: 10, y: 10, width: 1900, height: 1060 });
    }
}
