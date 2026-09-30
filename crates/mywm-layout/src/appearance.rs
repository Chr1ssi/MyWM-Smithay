use crate::Rect;

/// RGBA color with premultiplication left to the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 4]);

impl Color {
    /// Parse `#RRGGBB`.
    pub fn parse(value: &str) -> Result<Self, String> {
        let hex = value
            .strip_prefix('#')
            .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| "colors must use #RRGGBB".to_string())?;
        let channel = |i: usize| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap() as f32 / 255.0;
        Ok(Self([channel(0), channel(1), channel(2), 1.0]))
    }
}

#[derive(Clone, Debug)]
pub struct Appearance {
    pub gaps_inner: i32,
    pub gaps_outer: i32,
    pub border_width: i32,
    pub active_border: Color,
    pub inactive_border: Color,
    pub background: Color,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            gaps_inner: 8,
            gaps_outer: 8,
            border_width: 2,
            active_border: Color::parse("#89b4fa").unwrap(),
            inactive_border: Color::parse("#45475a").unwrap(),
            background: Color::parse("#1e1e2e").unwrap(),
        }
    }
}

impl Appearance {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=128).contains(&self.gaps_inner) || !(0..=128).contains(&self.gaps_outer) {
            return Err("appearance gaps must be between 0 and 128".into());
        }
        if !(0..=32).contains(&self.border_width) {
            return Err("appearance.border_width must be between 0 and 32".into());
        }
        Ok(())
    }

    /// Area for tiled columns: the work area minus the outer gap.
    pub fn viewport(&self, area: Rect) -> Rect {
        let gap = self
            .gaps_outer
            .min((area.width - 1).max(0) / 2)
            .min((area.height - 1).max(0) / 2);
        Rect {
            x: area.x + gap,
            y: area.y + gap,
            width: area.width - 2 * gap,
            height: area.height - 2 * gap,
        }
    }

    /// Split a frame into window content and border width (border is drawn inside the frame).
    pub fn content(&self, frame: Rect) -> (Rect, i32) {
        let border = self
            .border_width
            .min((frame.width - 1).max(0) / 2)
            .min((frame.height - 1).max(0) / 2);
        (
            Rect {
                x: frame.x + border,
                y: frame.y + border,
                width: frame.width - 2 * border,
                height: frame.height - 2 * border,
            },
            border,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borders_and_gaps_fit_inside_work_area() {
        let style = Appearance::default();
        let area = Rect { x: 100, y: 240, width: 1920, height: 1000 };
        let (content, border) = style.content(style.viewport(area));
        assert_eq!(content, Rect { x: 110, y: 250, width: 1900, height: 980 });
        assert_eq!(border, 2);
        let tiny = Rect { width: 1, height: 1, ..area };
        assert_eq!(style.content(style.viewport(tiny)), (tiny, 0));
    }

    #[test]
    fn validates_and_parses() {
        assert_eq!(Color::parse("#ff8000").unwrap().0[0], 1.0);
        assert!(Color::parse("blue").is_err());
        assert!(Color::parse("#12345g").is_err());
        assert!(Appearance { gaps_outer: 129, ..Default::default() }.validate().is_err());
        assert!(Appearance { border_width: 33, ..Default::default() }.validate().is_err());
        assert!(Appearance::default().validate().is_ok());
    }
}
