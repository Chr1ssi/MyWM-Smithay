/// Compute fixed-width columns and minimally scroll the focused column into view.
#[cfg(test)]
pub fn columns(
    width: i32,
    count: usize,
    focus: Option<usize>,
    scroll: i32,
) -> (i32, Vec<(i32, i32)>) {
    columns_with_gap(width, count, focus, scroll, 0)
}

#[cfg(test)]
pub fn columns_with_gap(
    width: i32,
    count: usize,
    focus: Option<usize>,
    scroll: i32,
    gap: i32,
) -> (i32, Vec<(i32, i32)>) {
    if count == 0 || width <= 0 {
        return (0, Vec::new());
    }
    let gap = gap.clamp(0, (width - 2).max(0));
    let column_width = if count == 1 {
        width
    } else {
        ((width - gap) / 2).max(1)
    };
    let stride = column_width + gap;
    let max_scroll = (stride * count as i32 - gap - width).max(0);
    let mut scroll = scroll.clamp(0, max_scroll);
    if let Some(focus) = focus.filter(|i| *i < count) {
        let left = focus as i32 * stride;
        if left < scroll {
            scroll = left;
        } else if left + column_width > scroll + width {
            scroll = left + column_width - width;
        }
    }
    (
        scroll,
        (0..count)
            .map(|i| (i as i32 * stride - scroll, column_width))
            .collect(),
    )
}

/// Lay out independently resized columns while keeping the focused one visible.
pub fn variable_columns(
    width: i32,
    widths: &[Option<i32>],
    focus: Option<usize>,
    scroll: i32,
    gap: i32,
) -> (i32, Vec<(i32, i32)>) {
    if widths.is_empty() || width <= 0 {
        return (0, Vec::new());
    }
    let gap = gap.clamp(0, (width - 2).max(0));
    let default_width = if widths.len() == 1 {
        width
    } else {
        ((width - gap) / 2).max(1)
    };
    let widths: Vec<_> = widths
        .iter()
        .map(|preferred| preferred.unwrap_or(default_width).clamp(1, width))
        .collect();
    let content_width = widths.iter().sum::<i32>() + gap * (widths.len() as i32 - 1);
    let max_scroll = (content_width - width).max(0);
    let mut scroll = scroll.clamp(0, max_scroll);
    let starts: Vec<_> = widths
        .iter()
        .scan(0, |left, column_width| {
            let result = *left;
            *left += *column_width + gap;
            Some(result)
        })
        .collect();
    if let Some(focus) = focus.filter(|index| *index < widths.len()) {
        let left = starts[focus];
        if left < scroll {
            scroll = left;
        } else if left + widths[focus] > scroll + width {
            scroll = left + widths[focus] - width;
        }
        scroll = scroll.clamp(0, max_scroll);
    }
    (
        scroll,
        starts
            .into_iter()
            .zip(widths)
            .map(|(left, column_width)| (left - scroll, column_width))
            .collect(),
    )
}

pub fn index_after_remove(index: usize, removed: usize) -> Option<usize> {
    if index == removed {
        None
    } else if index > removed {
        Some(index - 1)
    } else {
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_fills_monitor_two_split_it() {
        assert_eq!(columns(1920, 1, Some(0), 960), (0, vec![(0, 1920)]));
        assert_eq!(
            columns(1920, 2, Some(1), 0),
            (0, vec![(0, 960), (960, 960)])
        );
        assert_eq!(columns(1920, 0, None, 960), (0, vec![]));
    }

    #[test]
    fn focus_scrolls_only_when_needed() {
        let (scroll, positions) = columns(1920, 3, Some(2), 0);
        assert_eq!(scroll, 960);
        assert_eq!(positions, vec![(-960, 960), (0, 960), (960, 960)]);
        assert_eq!(columns(1920, 3, Some(1), scroll).0, 960);
        assert_eq!(columns(1920, 3, Some(0), scroll).0, 0);
    }

    #[test]
    fn removal_and_resize_clamp_viewport() {
        assert_eq!(columns(1920, 2, Some(1), 1920).0, 0);
        let (scroll, positions) = columns(1280, 4, Some(3), 1920);
        assert_eq!(scroll, 1280);
        assert_eq!(positions[3], (640, 640));
        let (_, positions) = columns(1919, 3, Some(2), 0);
        assert_eq!(positions[2].0 + positions[2].1, 1919);
    }

    #[test]
    fn gaps_separate_frames_without_wasting_the_single_window_area() {
        assert_eq!(
            columns_with_gap(1904, 1, Some(0), 0, 8),
            (0, vec![(0, 1904)])
        );
        assert_eq!(
            columns_with_gap(1904, 2, Some(1), 0, 8),
            (0, vec![(0, 948), (956, 948)])
        );
        let (scroll, positions) = columns_with_gap(1904, 3, Some(2), 0, 8);
        assert_eq!(scroll, 956);
        assert_eq!(positions[2], (956, 948));
        assert_eq!(
            columns_with_gap(1, 2, Some(1), 0, 128),
            (1, vec![(-1, 1), (0, 1)])
        );
    }

    #[test]
    fn removed_indices_cannot_refer_to_another_window() {
        assert_eq!(index_after_remove(2, 2), None);
        assert_eq!(index_after_remove(3, 2), Some(2));
        assert_eq!(index_after_remove(1, 2), Some(1));
    }

    #[test]
    fn variable_columns_preserve_user_widths_and_scroll_focus() {
        let widths = [Some(700), None, Some(900)];
        let (scroll, columns) = variable_columns(1200, &widths, Some(2), 0, 8);
        assert_eq!(
            columns.iter().map(|column| column.1).collect::<Vec<_>>(),
            [700, 596, 900]
        );
        assert_eq!(scroll, 1012);
        assert_eq!(columns[2], (300, 900));
    }
}
