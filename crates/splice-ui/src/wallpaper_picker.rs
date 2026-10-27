//! The wallpaper picker (splice-shell's `wallpaper.qml`): over the dimmed monitor a title, a carousel of the
//! images (the selected one wide in the middle, its neighbours as narrow strips on both sides, all leaning to the
//! right), the selected one's name, a filter field and a footer with the position and the keys, or what went
//! wrong. The tiles are not part of the scene: the compositor draws them leaning (`TileLook`, `LEAN`), so they
//! can slide without painting anything again.
use crate::{Align, Color, Measure, Node, Rect, Scene, TextStyle, Theme, edit::Input, i18n::Language};

/// The dimming over the whole monitor.
pub const BACKDROP: Color = Color::rgba(0, 0, 0, 0xb3);
/// Tiles this far from the selected one are shown (and their pictures wanted).
pub const VISIBLE: usize = 6;
/// How long a tile takes to move to its new place.
pub const SLIDE_MS: f32 = 180.0;

/// Between the column's parts.
const SPACING: f32 = 22.0;
/// The column is this much narrower than the monitor.
const MARGIN: f32 = 48.0;
const FIELD_WIDTH: f32 = 420.0;
const FIELD_HEIGHT: f32 = 40.0;
const FIELD_PADDING: f32 = 16.0;
const TITLE: TextStyle = TextStyle::new(16.0).bold(true);
const NAME: TextStyle = TextStyle::new(22.0);
const FIELD_TEXT: TextStyle = TextStyle::new(15.0);
const FOOTER: TextStyle = TextStyle::new(14.0);
/// A QML `Text` without a size of its own: the application font, 9 pt.
const PLAIN: TextStyle = TextStyle::new(12.0);
/// Every tile leans (`Matrix4x4(1, -0.06, 0, 14, …)` on its own coordinates): a point `y` below the tile's top
/// is drawn `LEAN · y + LEAN_SHIFT` to the right of where it is.
pub const LEAN: f32 = -0.06;
pub const LEAN_SHIFT: f32 = 14.0;
/// Neighbours are this much lower at the top and higher at the bottom than the selected tile.
const INSET: f32 = 20.0;
/// Neighbours are wider than their step by this much, so they overlap.
const OVERLAP: f32 = 24.0;
const ELLIPSIS: &str = "…";

/// Why the picker cannot do what it was asked.
#[derive(Clone, Debug, PartialEq)]
pub enum PickerError {
    /// The wallpaper directory (named) cannot be read.
    Directory(String),
    /// The saved choice is not one the picker wrote.
    State,
    /// The chosen image does not decode.
    Image,
    Save,
}

/// A tile in the carousel: which image, and where it is now (it slides to its place).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileView {
    /// Its index in the results.
    pub index: usize,
    /// From the carousel's left edge.
    pub x: f32,
    pub width: f32,
}

/// How a tile looks before it leans: the surface color behind the picture, the picture filling `image` (cut to
/// its proportions), a dimming over all of it and the border inside `rect` on top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileLook {
    /// In the scene's coordinates.
    pub rect: Rect,
    pub image: Rect,
    pub surface: Color,
    pub dim: Color,
    pub border: f32,
    pub border_color: Color,
}

/// What the scene shows (the tiles come separately).
#[derive(Clone, Debug, PartialEq)]
pub struct PickerModel {
    /// The monitor's size.
    pub output: (f32, f32),
    pub selected: usize,
    /// How many images match the filter.
    pub count: usize,
    /// The selected image's file name.
    pub name: String,
    pub query: Input,
    /// The directory is still being read.
    pub loading: bool,
    pub error: Option<PickerError>,
    pub language: Language,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTarget {
    /// The filter field.
    Search,
    /// A tile, by its index in the results.
    Tile(usize),
}

/// The carousel's measures on a monitor `output` large.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Carousel {
    pub width: f32,
    pub height: f32,
    /// The selected tile's width.
    pub expanded: f32,
    /// How far apart the neighbours are.
    pub step: f32,
}

impl Carousel {
    pub fn new(output: (f32, f32)) -> Self {
        let width = output.0 - MARGIN;
        Self { width, height: (output.1 * 0.58).min(475.0), expanded: (width * 0.64).min(768.0), step: (width * 0.07).min(78.0) }
    }

    /// Where a tile `distance` places from the selected one belongs: its left edge and width.
    pub fn place(&self, distance: isize) -> (f32, f32) {
        let start = (self.width - self.expanded) / 2.0;
        match distance {
            0 => (start, self.expanded),
            d if d < 0 => (start + d as f32 * self.step, self.step + OVERLAP),
            d => (start + self.expanded + (d - 1) as f32 * self.step, self.step + OVERLAP),
        }
    }

    /// The size a picture needs to fill the selected tile (neighbours show less of the same picture).
    pub fn picture_size(&self) -> (f32, f32) {
        (self.expanded, self.height)
    }
}

/// The parts of the column, from its top.
struct Column {
    title: f32,
    carousel: f32,
    name: f32,
    field: f32,
    footer: f32,
    height: f32,
}

fn column(measure: &mut impl Measure, carousel: &Carousel) -> Column {
    let title = 0.0;
    let carousel_y = title + measure.line_height(TITLE) + SPACING;
    let name = carousel_y + carousel.height + SPACING;
    let field = name + measure.line_height(NAME) + SPACING;
    let footer = field + FIELD_HEIGHT + SPACING;
    let height = footer + measure.line_height(FOOTER);
    Column { title, carousel: carousel_y, name, field, footer, height }
}

/// The scene's size (the column's) and where it sits on the monitor: centered, on whole pixels.
pub fn frame(measure: &mut impl Measure, output: (f32, f32)) -> Rect {
    let carousel = Carousel::new(output);
    let height = column(measure, &carousel).height;
    Rect::new(MARGIN / 2.0, ((output.1 - height) / 2.0).floor(), carousel.width, height)
}

/// Where the carousel lies in the scene (it cuts off the tiles at its sides).
pub fn carousel_area(measure: &mut impl Measure, output: (f32, f32)) -> Rect {
    let carousel = Carousel::new(output);
    Rect::new(0.0, column(measure, &carousel).carousel, carousel.width, carousel.height)
}

/// Where `tile` lies in the scene (the carousel at `area`), before it leans.
fn tile_rect(area: &Rect, tile: &TileView, selected: bool) -> Rect {
    let inset = if selected { 0.0 } else { INSET };
    Rect::new(area.x + tile.x, area.y + inset, tile.width, area.h - 2.0 * inset)
}

/// How `tile` looks in the carousel at `area`.
pub fn tile_look(area: &Rect, tile: &TileView, selected: bool, theme: &Theme) -> TileLook {
    let rect = tile_rect(area, tile, selected);
    let (margin, border, border_color) = if selected { (3.0, 3.0, theme.accent) } else { (1.0, 1.0, theme.border) };
    let dim = if selected { Color::TRANSPARENT } else { Color::rgba(0, 0, 0, 0x66) };
    TileLook { rect, image: rect.inset(margin), surface: theme.surface, dim, border, border_color }
}

/// Where the point (`x`, `y`) of `rect` is drawn once it leans.
pub fn leaned(rect: &Rect, x: f32, y: f32) -> (f32, f32) {
    (x + LEAN * (y - rect.y) + LEAN_SHIFT, y)
}

/// The tiles in painting order: the farthest first, the selected one last (`z`), and of two as far, the later one
/// on top (the `Repeater`'s order).
pub fn stacked(tiles: &[TileView], selected: usize) -> Vec<&TileView> {
    let mut tiles: Vec<&TileView> = tiles.iter().collect();
    tiles.sort_by_key(|tile| (tile.index == selected, std::cmp::Reverse(tile.index.abs_diff(selected)), tile.index));
    tiles
}

fn field_rect(carousel: &Carousel, top: f32) -> Rect {
    let width = FIELD_WIDTH.min(carousel.width);
    Rect::new(((carousel.width - width) / 2.0).round(), top, width, FIELD_HEIGHT)
}

/// Where the field's text starts and how far it is scrolled to the left (to keep the cursor in view).
fn field_text(measure: &mut impl Measure, query: &Input, field_width: f32) -> f32 {
    let cursor_x = measure.width(&query.text[..query.cursor], FIELD_TEXT);
    (cursor_x - (field_width - 2.0 * FIELD_PADDING) + 2.0).max(0.0)
}

/// The cursor position nearest to `x` in the filter field (`x` from the field's left edge).
pub fn search_cursor_at(measure: &mut impl Measure, query: &Input, output: (f32, f32), x: f32) -> usize {
    let width = field_rect(&Carousel::new(output), 0.0).w;
    let scrolled = field_text(measure, query, width);
    query.nearest(measure, FIELD_TEXT, x - FIELD_PADDING + scrolled)
}

/// `text` cut in the middle with "…" so that it fits `width` (`elide: Text.ElideMiddle`).
fn elide_middle(measure: &mut impl Measure, text: &str, style: TextStyle, width: f32) -> String {
    if measure.width(text, style) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| {
        let (head, tail) = (keep.div_ceil(2), keep / 2);
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        format!("{start}{ELLIPSIS}{end}")
    };
    // The most characters that still fit with the ellipsis.
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if measure.width(&cut(middle), style) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    cut(low)
}

/// The column, with its origin at (0, 0) (`frame` says where it goes).
pub fn scene(model: &PickerModel, theme: &Theme, measure: &mut impl Measure) -> Scene<PickerTarget> {
    let tr = |text| model.language.tr(text);
    let carousel = Carousel::new(model.output);
    let parts = column(measure, &carousel);
    let mut scene = Scene::new(carousel.width, parts.height);
    let width = carousel.width;

    let title_h = measure.line_height(TITLE);
    // Centered on whole pixels (`anchors.horizontalCenter`).
    let title_w = measure.width("WALLPAPER", TITLE).ceil();
    let title = Rect::new(((width - title_w) / 2.0).round(), parts.title, title_w, title_h);
    scene.push(Node::Label { rect: title, text: "WALLPAPER".into(), style: TITLE, color: theme.text, align: Align::Left });

    let area = Rect::new(0.0, parts.carousel, width, carousel.height);
    if model.count == 0 {
        let text = if model.loading { tr("Bilder werden geladen …") } else { tr("Keine passenden Bilder gefunden") };
        scene.push(Node::Label { rect: area, text: text.into(), style: PLAIN, color: theme.text, align: Align::Center });
    }

    let name = elide_middle(measure, &model.name, NAME, width);
    let name_rect = Rect::new(0.0, parts.name, width, measure.line_height(NAME));
    scene.push(Node::Label { rect: name_rect, text: name, style: NAME, color: theme.text, align: Align::Center });

    let field = field_rect(&carousel, parts.field);
    scene.push(Node::Rect { rect: field, radius: FIELD_HEIGHT / 2.0, fill: theme.glass(), border: Some((1.0, theme.glass_border())) });
    let text_area = Rect::new(field.x + FIELD_PADDING, field.y, field.w - 2.0 * FIELD_PADDING, field.h);
    let scrolled = field_text(measure, &model.query, field.w);
    scene.push(Node::Clip(Some(text_area)));
    if model.query.text.is_empty() {
        scene.push(Node::Label { rect: text_area, text: tr("Bilder filtern …").into(), style: FIELD_TEXT, color: theme.muted, align: Align::Left });
    } else {
        // Wide enough not to be cut off with "…": the clip cuts it.
        let line = Rect::new(text_area.x - scrolled, field.y, measure.width(&model.query.text, FIELD_TEXT).ceil() + 2.0, field.h);
        scene.push(Node::Label { rect: line, text: model.query.text.clone(), style: FIELD_TEXT, color: theme.text, align: Align::Left });
    }
    let cursor_x = measure.width(&model.query.text[..model.query.cursor], FIELD_TEXT);
    let line_h = measure.line_height(FIELD_TEXT);
    let cursor = Rect::new((text_area.x + cursor_x - scrolled).round(), field.y + ((field.h - line_h) / 2.0).round(), 2.0, line_h);
    scene.push(Node::Rect { rect: cursor, radius: 0.0, fill: theme.text, border: None });
    scene.push(Node::Clip(None));
    scene.target(field, PickerTarget::Search);

    let footer = match &model.error {
        Some(PickerError::Directory(directory)) => model.language.trf("Bildordner konnte nicht gelesen werden: %1", &[directory]),
        Some(PickerError::State) => tr("Gespeicherte Auswahl konnte nicht gelesen werden.").into(),
        Some(PickerError::Image) => tr("Dieses Bild kann nicht geladen werden.").into(),
        Some(PickerError::Save) => tr("Auswahl konnte nicht gespeichert werden.").into(),
        None => {
            let position = if model.count > 0 { model.selected + 1 } else { 0 };
            model.language.trf("%1 / %2  ·  ← → Auswahl  ·  ↵ Anwenden  ·  Esc", &[&position.to_string(), &model.count.to_string()])
        }
    };
    let footer_rect = Rect::new(0.0, parts.footer, width, measure.line_height(FOOTER));
    scene.push(Node::Label { rect: footer_rect, text: footer, style: FOOTER, color: theme.text, align: Align::Center });
    scene
}

/// What is at (`x`, `y`) of the scene: a tile (as it leans, the one on top) or the filter field.
pub fn target_at(
    scene: &Scene<PickerTarget>,
    tiles: &[TileView],
    selected: usize,
    area: &Rect,
    x: f32,
    y: f32,
) -> Option<PickerTarget> {
    if area.contains(x, y) {
        for tile in stacked(tiles, selected).into_iter().rev() {
            let rect = tile_rect(area, tile, tile.index == selected);
            // Straightened again: where the point was before the tile leaned.
            let (shift, _) = leaned(&rect, 0.0, y);
            if rect.contains(x - shift, y) {
                return Some(PickerTarget::Tile(tile.index));
            }
        }
    }
    scene.target_at(x, y)
}

/// The images whose names contain `query`, ignoring case (and spaces around it), as indices into `names`.
pub fn results(names: &[String], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    names.iter().enumerate().filter(|(_, name)| name.to_lowercase().contains(&query)).map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scene::tests::Mono, theme::Palette};

    fn theme() -> Theme {
        let c = |v| Color::rgba(v, v, v, 255);
        Theme::new(Palette { background: c(10), surface: c(20), text: c(200), muted: c(150), accent: c(90), border: c(60) }, None)
    }

    fn model(count: usize, selected: usize) -> PickerModel {
        PickerModel {
            output: (1280.0, 800.0),
            selected,
            count,
            name: "a.png".into(),
            query: Input::default(),
            loading: false,
            error: None,
            language: Language::German,
        }
    }

    fn tiles(count: usize, selected: usize) -> Vec<TileView> {
        let carousel = Carousel::new((1280.0, 800.0));
        (0..count)
            .filter(|i| i.abs_diff(selected) <= VISIBLE)
            .map(|index| {
                let (x, width) = carousel.place(index as isize - selected as isize);
                TileView { index, x, width }
            })
            .collect()
    }

    #[test]
    fn the_carousel_spreads_the_neighbours_around_the_wide_one() {
        let carousel = Carousel::new((1280.0, 800.0));
        assert_eq!((carousel.width, carousel.height, carousel.expanded, carousel.step), (1232.0, 464.0, 768.0, 78.0));
        assert_eq!(carousel.place(0), (232.0, 768.0));
        assert_eq!(carousel.place(-1), (154.0, 102.0));
        assert_eq!(carousel.place(1), (1000.0, 102.0));
        assert_eq!(carousel.place(2), (1078.0, 102.0));
        // A small monitor: the measures shrink with it.
        let small = Carousel::new((648.0, 400.0));
        assert_eq!((small.expanded, small.step, small.height), (384.0, 42.0, 232.0));
    }

    #[test]
    fn hits_follow_the_leaning_tiles_and_the_one_on_top() {
        let model = model(5, 2);
        let scene = scene(&model, &theme(), &mut Mono);
        let area = carousel_area(&mut Mono, model.output);
        let top = area.y;
        let tiles = tiles(5, 2);
        let at = |x: f32, y: f32| target_at(&scene, &tiles, 2, &area, x, y);
        // The selected tile spans 232..1000 at its top, shifted by 14 to the right; its neighbour starts 20 lower.
        assert_eq!(at(250.0, top + 1.0), Some(PickerTarget::Tile(2)));
        assert_eq!(at(240.0, top + 1.0), None);
        assert_eq!(at(170.0, top + 25.0), Some(PickerTarget::Tile(1)));
        // At the bottom the selected tile has moved left by 0.06 · 463, and the neighbour ended 20 higher.
        let bottom = top + 463.0;
        assert_eq!(at(232.0 + 14.0 - 0.06 * 463.0 + 1.0, bottom), Some(PickerTarget::Tile(2)));
        assert_eq!(at(232.0 + 14.0 - 0.06 * 463.0 - 2.0, bottom), None);
        // Where neighbours overlap (100 below their top: shifted by 8), the nearer one is on top.
        assert_eq!(at(1000.0 + 8.0 + 90.0, top + 120.0), Some(PickerTarget::Tile(3)));
        assert_eq!(at(1000.0 + 8.0 + 110.0, top + 120.0), Some(PickerTarget::Tile(4)));
        // Above the carousel nothing; the field is the field.
        assert_eq!(at(240.0, top - 2.0), None);
        let field = scene.target_rect(&PickerTarget::Search).expect("field");
        assert_eq!((field.x, field.w), (406.0, 420.0));
        assert_eq!(at(field.x + 5.0, field.y + 20.0), Some(PickerTarget::Search));
    }

    #[test]
    fn the_far_ones_are_painted_first() {
        let tiles = tiles(5, 2);
        let order: Vec<usize> = stacked(&tiles, 2).iter().map(|t| t.index).collect();
        assert_eq!(order, [0, 4, 1, 3, 2]);
    }

    #[test]
    fn the_selected_tile_is_framed_in_the_accent_and_the_others_dimmed() {
        let area = carousel_area(&mut Mono, (1280.0, 800.0));
        let tiles = tiles(5, 2);
        let selected = tile_look(&area, &tiles[2], true, &theme());
        assert_eq!(selected.rect, Rect::new(232.0, area.y, 768.0, 464.0));
        assert_eq!((selected.image, selected.border, selected.dim), (selected.rect.inset(3.0), 3.0, Color::TRANSPARENT));
        let neighbour = tile_look(&area, &tiles[3], false, &theme());
        assert_eq!(neighbour.rect, Rect::new(1000.0, area.y + 20.0, 102.0, 424.0));
        assert_eq!((neighbour.border, neighbour.border_color, neighbour.dim.a), (1.0, theme().border, 0x66));
        assert_eq!(leaned(&neighbour.rect, 1000.0, area.y + 120.0), (1008.0, area.y + 120.0));
    }

    #[test]
    fn names_are_cut_in_the_middle() {
        assert_eq!(elide_middle(&mut Mono, "abcdefghij", NAME, 1000.0), "abcdefghij");
        // 11 per 6 characters: "abc…ij" is 6 × 11 = 66 wide.
        assert_eq!(elide_middle(&mut Mono, "abcdefghij", NAME, 66.0), "abc…ij");
        assert_eq!(elide_middle(&mut Mono, "abcdefghij", NAME, 5.0), "…");
    }

    #[test]
    fn the_filter_ignores_case_and_surrounding_spaces() {
        let names = ["a gradient # 100%.png", "b second.PNG"].map(String::from);
        assert_eq!(results(&names, " GRADIENT "), [0]);
        assert_eq!(results(&names, "png"), [0, 1]);
        assert!(results(&names, "no matches").is_empty());
    }

    #[test]
    fn the_footer_tells_the_position_or_what_went_wrong() {
        let footer = |model: &PickerModel| match scene(model, &theme(), &mut Mono).nodes.last() {
            Some(Node::Label { text, .. }) => text.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(footer(&model(5, 2)), "3 / 5  ·  ← → Auswahl  ·  ↵ Anwenden  ·  Esc");
        assert_eq!(footer(&model(0, 0)), "0 / 0  ·  ← → Auswahl  ·  ↵ Anwenden  ·  Esc");
        let broken = PickerModel { error: Some(PickerError::Directory("/x".into())), ..model(0, 0) };
        assert_eq!(footer(&broken), "Bildordner konnte nicht gelesen werden: /x");
    }
}
