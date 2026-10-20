//! Dark theme colors from a wallpaper.
//!
//! Material's dark schemes put every accent at tone 80 or brighter, where most hues can hardly carry
//! chroma, and derive all accents from one seed; wallpapers with more than one color then turn into
//! near-white pastels of a single hue. Here the accents are the image's own hue families, the surfaces
//! take the hue of its dark regions, and the terminal keeps fixed hue roles (red stays red) turned
//! toward the image. Everything is computed in HCT, so contrast follows from the tones alone.

use material_colors::{
    color::Argb,
    hct::Hct,
    image::{AsPixels, Image},
    quantize::{Quantizer, QuantizerCelebi},
    utils::math::{difference_degrees, rotate_direction, sanitize_degrees_double},
};
use serde::{Deserialize, Serialize};

/// Clusters below this chroma count as grey: they tint nothing and seed no accent.
const COLORFUL: f64 = 12.0;
/// Accents differ at least this much in hue, so secondary and tertiary are colors of their own.
const ACCENT_DISTANCE: f64 = 35.0;
/// Half the width of a hue family: the quantizer splits one sky into many slightly different clusters.
const FAMILY_HALF_WIDTH: usize = 15;
/// Image regions darker than this tone are what the dark surfaces stand for.
const DARK_REGION: f64 = 40.0;
/// Pure sRGB red (#ff0000) in HCT: errors are the same vivid red on every wallpaper.
const ERROR_HUE: f64 = 27.4;
/// ANSI red, green, yellow, blue, magenta, cyan: their HCT hue and how far an image color may differ
/// and still count as that role.
const ANSI_ROLES: [(f64, f64); 6] = [(25.0, 20.0), (140.0, 30.0), (90.0, 25.0), (265.0, 30.0), (330.0, 30.0), (195.0, 30.0)];

/// What the theme takes from the image; the colors follow from it alone, so it is what gets cached.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Primary, secondary and tertiary as the image has them: hue, chroma, tone.
    accents: [[f64; 3]; 3],
    /// Hue and chroma of the surfaces.
    neutral: [f64; 2],
    /// Hue and chroma of ANSI red, green, yellow, blue, magenta and cyan.
    terminal: [[f64; 2]; 6],
}

impl Analysis {
    /// `image` should be small already (about 112×112): quantizing is the expensive part.
    pub fn of_image(image: &Image) -> Self {
        let result = QuantizerCelebi::quantize(&image.as_pixels(), 128);
        Self::of_histogram(result.color_to_count.iter().map(|(&color, &count)| (color, count)))
    }

    pub fn of_histogram(histogram: impl IntoIterator<Item = (Argb, u32)>) -> Self {
        let clusters: Vec<(Hct, u32)> = histogram.into_iter().map(|(color, count)| (Hct::new(color), count)).collect();
        let total = clusters.iter().map(|&(_, count)| f64::from(count)).sum::<f64>().max(1.0);
        // Every cluster with its share of the image area.
        let clusters: Vec<(Hct, f64)> = clusters.into_iter().map(|(hct, count)| (hct, f64::from(count) / total)).collect();
        let colorful: Vec<(Hct, f64)> = clusters.iter().copied().filter(|(hct, _)| hct.get_chroma() >= COLORFUL).collect();

        let accents = accents(&colorful);
        let neutral = neutral(&clusters);
        let terminal = terminal(&colorful).map(|(hue, chroma)| [hue, chroma]);
        Self {
            accents: accents.map(|hct| [hct.get_hue(), hct.get_chroma(), hct.get_tone()]),
            neutral: [neutral.0, neutral.1],
            terminal,
        }
    }

    /// The primary color as found in the image.
    pub fn source(&self) -> Argb {
        let [hue, chroma, tone] = self.accents[0];
        Hct::from(hue, chroma, tone).into()
    }
}

/// Up to three hue families ranked by area × chroma × visibility (near-black regions count less), each
/// as its area-weighted mean color. Missing ones are rotations of the first, as in a grey image.
fn accents(colorful: &[(Hct, f64)]) -> [Hct; 3] {
    let visibility = |tone: f64| ((tone - 5.0) / 30.0).clamp(0.2, 1.0);
    // Per degree of hue: ranking weight, area, and area-weighted chroma, tone and hue vector.
    let mut weight = [0.0; 360];
    let mut area = [0.0; 360];
    let mut chroma = [0.0; 360];
    let mut tone = [0.0; 360];
    let mut hue_sin = [0.0; 360];
    let mut hue_cos = [0.0; 360];
    for &(hct, share) in colorful {
        let degree = hct.get_hue().floor() as usize % 360;
        weight[degree] += share * hct.get_chroma() * visibility(hct.get_tone());
        area[degree] += share;
        chroma[degree] += share * hct.get_chroma();
        tone[degree] += share * hct.get_tone();
        hue_sin[degree] += share * hct.get_hue().to_radians().sin();
        hue_cos[degree] += share * hct.get_hue().to_radians().cos();
    }
    let family = |center: usize| (0..=2 * FAMILY_HALF_WIDTH).map(move |offset| (center + 360 - FAMILY_HALF_WIDTH + offset) % 360);
    let score: Vec<f64> = (0..360).map(|center| family(center).map(|degree| weight[degree]).sum()).collect();

    let mut found: Vec<Hct> = Vec::with_capacity(3);
    let mut taken = [false; 360];
    while found.len() < 3 {
        let Some(best) = (0..360).filter(|&degree| !taken[degree] && score[degree] > 0.0).max_by(|&a, &b| score[a].total_cmp(&score[b]))
        else {
            break;
        };
        let sum = |values: &[f64; 360]| family(best).map(|degree| values[degree]).sum::<f64>();
        let family_area = sum(&area);
        let hue = sanitize_degrees_double(sum(&hue_sin).atan2(sum(&hue_cos)).to_degrees());
        found.push(Hct::from(hue, sum(&chroma) / family_area, sum(&tone) / family_area));
        let distance = ACCENT_DISTANCE as usize;
        for offset in 0..=2 * distance {
            taken[(best + 360 - distance + offset) % 360] = true;
        }
    }
    let first = found.first().copied().unwrap_or_else(|| Hct::from(260.0, 30.0, 60.0));
    std::array::from_fn(|index| {
        found.get(index).copied().unwrap_or_else(|| {
            Hct::from(sanitize_degrees_double(first.get_hue() + 60.0 * index as f64), first.get_chroma(), first.get_tone())
        })
    })
}

/// Hue and chroma of the surfaces: the image's dark regions (what dark surfaces stand for), else the
/// cluster with the largest area.
fn neutral(clusters: &[(Hct, f64)]) -> (f64, f64) {
    let (mut area, mut chroma, mut hue_sin, mut hue_cos) = (0.0, 0.0, 0.0, 0.0);
    for &(hct, share) in clusters {
        if hct.get_tone() >= DARK_REGION || hct.get_chroma() < 4.0 {
            continue;
        }
        let weight = share * hct.get_chroma();
        area += share;
        chroma += share * hct.get_chroma();
        hue_sin += weight * hct.get_hue().to_radians().sin();
        hue_cos += weight * hct.get_hue().to_radians().cos();
    }
    if area > 0.05 {
        return (sanitize_degrees_double(hue_sin.atan2(hue_cos).to_degrees()), (chroma / area * 0.6).clamp(4.0, 14.0));
    }
    clusters
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map_or((260.0, 4.0), |(hct, _)| (hct.get_hue(), (hct.get_chroma() * 0.4).clamp(4.0, 14.0)))
}

/// Hue and chroma of each ANSI color: a nearby image color gives both; otherwise the role turns toward
/// the closest image color and takes the image's mean chroma.
fn terminal(colorful: &[(Hct, f64)]) -> [(f64, f64); 6] {
    let area = colorful.iter().map(|&(_, share)| share).sum::<f64>();
    let mean_chroma = if area > 0.0 {
        colorful.iter().map(|&(hct, share)| hct.get_chroma() * share).sum::<f64>() / area
    } else {
        20.0
    };
    let candidates: Vec<Hct> = colorful.iter().map(|&(hct, _)| hct).filter(|hct| hct.get_chroma() >= 16.0).collect();
    ANSI_ROLES.map(|(role, window)| {
        let distance = |hct: &&Hct| difference_degrees(hct.get_hue(), role);
        if let Some(near) = candidates.iter().filter(|hct| distance(hct) <= window).max_by(|a, b| a.get_chroma().total_cmp(&b.get_chroma())) {
            return (near.get_hue(), (near.get_chroma() * 1.2).clamp(36.0, 80.0));
        }
        let hue = candidates.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).map_or(role, |closest| {
            let turn = (difference_degrees(role, closest.get_hue()) * 0.5).min(25.0);
            sanitize_degrees_double(role + turn * rotate_direction(role, closest.get_hue()))
        });
        (hue, (mean_chroma * 1.3).clamp(32.0, 64.0))
    })
}

/// The theme's color roles (Material/Noctalia names, so templates and consumers stay unchanged) and the
/// terminal palette.
pub fn roles(analysis: &Analysis) -> Vec<(&'static str, Argb)> {
    let color = |hue: f64, chroma: f64, tone: f64| -> Argb { Hct::from(hue, chroma, tone).into() };
    // Accents keep the image color's own tone and chroma, pulled into a readable and not greyed-out range.
    let accent = |[hue, chroma, tone]: [f64; 3]| (hue, (chroma * 1.25).clamp(40.0, 90.0), tone.clamp(64.0, 80.0));
    let [neutral_hue, tint] = analysis.neutral;
    let neutral = |chroma: f64, tone: f64| color(neutral_hue, chroma, tone);

    let mut roles = Vec::with_capacity(72);
    for (index, name) in ["primary", "secondary", "tertiary"].into_iter().enumerate() {
        let (hue, chroma, tone) = accent(analysis.accents[index]);
        let names = ACCENT_ROLES[index];
        roles.extend([
            (name, color(hue, chroma, tone)),
            (names[0], color(hue, 14.0, 12.0)),
            (names[1], color(hue, chroma * 0.7, 30.0)),
            (names[2], color(hue, 20.0, 90.0)),
            (names[3], color(hue, chroma * 0.6, 88.0)),
            (names[4], color(hue, chroma * 0.7, 78.0)),
            (names[5], color(hue, 14.0, 10.0)),
            (names[6], color(hue, chroma * 0.6, 30.0)),
        ]);
        if index == 0 {
            roles.push(("inverse_primary", color(hue, chroma, 40.0)));
            roles.push(("surface_tint", color(hue, chroma, tone)));
        }
    }
    let (surface, on_surface) = (neutral(tint, 7.0), neutral(4.0, 94.0));
    let on_surface_variant = neutral(tint + 4.0, 78.0);
    let outline = neutral(tint + 6.0, 60.0);
    roles.extend([
        ("error", color(ERROR_HUE, 120.0, 55.0)),
        ("on_error", color(ERROR_HUE, 14.0, 4.0)),
        ("error_container", color(ERROR_HUE, 80.0, 30.0)),
        ("on_error_container", color(ERROR_HUE, 30.0, 90.0)),
        ("surface_dim", neutral(tint, 6.0)),
        ("surface", surface),
        ("surface_bright", neutral(tint, 24.0)),
        ("surface_container_lowest", neutral(tint, 4.0)),
        ("surface_container_low", neutral(tint, 10.0)),
        ("surface_container", neutral(tint, 13.0)),
        ("surface_container_high", neutral(tint, 17.0)),
        ("surface_container_highest", neutral(tint, 22.0)),
        ("surface_variant", neutral(tint + 4.0, 30.0)),
        ("on_surface", on_surface),
        ("on_surface_variant", on_surface_variant),
        ("outline", outline),
        ("outline_variant", neutral(tint + 8.0, 45.0)),
        ("inverse_surface", neutral(tint, 90.0)),
        ("inverse_on_surface", neutral(tint, 20.0)),
        ("background", surface),
        ("on_background", on_surface),
        ("shadow", Argb::new(255, 0, 0, 0)),
        ("scrim", Argb::new(255, 0, 0, 0)),
        ("terminal_background", surface),
        ("terminal_foreground", on_surface),
        ("terminal_cursor", on_surface),
        ("terminal_cursor_text", surface),
        ("terminal_selection_bg", neutral(tint, 22.0)),
        ("terminal_selection_fg", on_surface),
        ("terminal_normal_black", neutral(tint + 2.0, 17.0)),
        ("terminal_normal_white", on_surface_variant),
        ("terminal_bright_black", outline),
        ("terminal_bright_white", on_surface),
    ]);
    for (index, [hue, chroma]) in analysis.terminal.into_iter().enumerate() {
        let [normal, bright] = ANSI_NAMES[index];
        roles.push((normal, color(hue, chroma, 68.0)));
        roles.push((bright, color(hue, chroma * 0.85, 80.0)));
    }
    roles
}

/// The roles that come with primary, secondary and tertiary: on, container, on container, fixed, fixed
/// dim, on fixed, on fixed variant.
const ACCENT_ROLES: [[&str; 7]; 3] = [
    [
        "on_primary",
        "primary_container",
        "on_primary_container",
        "primary_fixed",
        "primary_fixed_dim",
        "on_primary_fixed",
        "on_primary_fixed_variant",
    ],
    [
        "on_secondary",
        "secondary_container",
        "on_secondary_container",
        "secondary_fixed",
        "secondary_fixed_dim",
        "on_secondary_fixed",
        "on_secondary_fixed_variant",
    ],
    [
        "on_tertiary",
        "tertiary_container",
        "on_tertiary_container",
        "tertiary_fixed",
        "tertiary_fixed_dim",
        "on_tertiary_fixed",
        "on_tertiary_fixed_variant",
    ],
];

const ANSI_NAMES: [[&str; 2]; 6] = [
    ["terminal_normal_red", "terminal_bright_red"],
    ["terminal_normal_green", "terminal_bright_green"],
    ["terminal_normal_yellow", "terminal_bright_yellow"],
    ["terminal_normal_blue", "terminal_bright_blue"],
    ["terminal_normal_magenta", "terminal_bright_magenta"],
    ["terminal_normal_cyan", "terminal_bright_cyan"],
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn hct(hue: f64, chroma: f64, tone: f64) -> Argb {
        Hct::from(hue, chroma, tone).into()
    }

    fn colors(analysis: &Analysis) -> HashMap<&'static str, Argb> {
        roles(analysis).into_iter().collect()
    }

    fn ratio(a: Argb, b: Argb) -> f64 {
        material_colors::contrast::ratio_of_tones(Hct::new(a).get_tone(), Hct::new(b).get_tone())
    }

    /// Histograms shaped like real wallpapers: a mint sky with a small coral glow, a navy night with a red
    /// spot, a grey photo, and a single pink.
    fn samples() -> Vec<Analysis> {
        vec![
            Analysis::of_histogram([(hct(187.0, 17.0, 63.0), 40), (hct(9.0, 45.0, 54.0), 8), (hct(333.0, 16.0, 30.0), 20), (hct(0.0, 2.0, 50.0), 32)]),
            Analysis::of_histogram([(hct(269.0, 33.0, 15.0), 62), (hct(240.0, 35.0, 22.0), 33), (hct(16.0, 64.0, 49.0), 3)]),
            Analysis::of_histogram([(hct(0.0, 0.0, 30.0), 50), (hct(0.0, 0.0, 80.0), 50)]),
            Analysis::of_histogram([(hct(350.0, 33.0, 55.0), 100)]),
        ]
    }

    #[test]
    fn text_borders_and_accents_stand_out() {
        for analysis in samples() {
            let colors = colors(&analysis);
            let surface = colors["surface"];
            assert!(ratio(colors["outline_variant"], surface) >= 3.0, "{analysis:?}");
            assert!(ratio(colors["on_surface_variant"], surface) >= 7.0, "{analysis:?}");
            assert!(ratio(colors["on_surface"], surface) > ratio(colors["on_surface_variant"], surface), "{analysis:?}");
            for accent in ["primary", "secondary", "tertiary", "error"] {
                assert!(ratio(colors[accent], surface) >= 4.5, "{accent} {analysis:?}");
                assert!(ratio(colors[accent], colors[&*format!("on_{accent}")]) >= 4.5, "on_{accent} {analysis:?}");
                assert!(ratio(colors[&*format!("on_{accent}_container")], colors[&*format!("{accent}_container")]) >= 4.5, "{accent} container {analysis:?}");
            }
            for [normal, bright] in ANSI_NAMES {
                assert!(ratio(colors[normal], colors["terminal_background"]) >= 4.5, "{normal} {analysis:?}");
                assert!(ratio(colors[bright], colors["terminal_background"]) >= 4.5, "{bright} {analysis:?}");
            }
        }
    }

    #[test]
    fn a_large_area_outweighs_a_small_saturated_spot() {
        // Navy covers 95 % of the night picture; its red spot is far more saturated but tiny.
        let night = &samples()[1];
        assert!(difference_degrees(Hct::new(night.source()).get_hue(), 260.0) < 20.0, "{night:?}");
        // The spot still becomes an accent of its own.
        assert!(night.accents[1..].iter().any(|accent| difference_degrees(accent[0], 16.0) < 10.0), "{night:?}");
    }

    #[test]
    fn accents_are_distinct_image_colors() {
        let mint = &samples()[0];
        let hues: Vec<f64> = mint.accents.iter().map(|accent| accent[0]).collect();
        for (index, hue) in hues.iter().enumerate() {
            for other in &hues[index + 1..] {
                assert!(difference_degrees(*hue, *other) >= ACCENT_DISTANCE, "{hues:?}");
            }
        }
        // Mint sky, coral glow and plum shadows each give one.
        for expected in [187.0, 9.0, 333.0] {
            assert!(hues.iter().any(|hue| difference_degrees(*hue, expected) < 5.0), "{expected} {hues:?}");
        }
    }

    #[test]
    fn surfaces_take_the_hue_of_the_dark_regions() {
        // Mint sky, plum shadows: the surfaces are plum, not mint.
        assert!(difference_degrees(samples()[0].neutral[0], 333.0) < 10.0, "{:?}", samples()[0]);
        assert!(difference_degrees(samples()[1].neutral[0], 260.0) < 15.0, "{:?}", samples()[1]);
    }

    #[test]
    fn terminal_colors_keep_their_roles() {
        for analysis in samples() {
            for ([hue, _], (role, _)) in analysis.terminal.iter().zip(ANSI_ROLES) {
                assert!(difference_degrees(*hue, role) <= 30.0, "{role} {analysis:?}");
            }
        }
    }

    #[test]
    fn error_is_the_same_vivid_red_everywhere() {
        let errors: Vec<Argb> = samples().iter().map(|analysis| colors(analysis)["error"]).collect();
        assert!(errors.windows(2).all(|pair| pair[0] == pair[1]), "{errors:?}");
        let error = Hct::new(errors[0]);
        assert!(difference_degrees(error.get_hue(), ERROR_HUE) < 3.0 && error.get_chroma() > 90.0, "{error:?}");
    }

    #[test]
    fn a_grey_image_still_gets_three_accents() {
        let grey = &samples()[2];
        assert_eq!(roles(grey).len(), roles(&samples()[0]).len());
        assert!(grey.accents.iter().all(|accent| accent.iter().all(|value| value.is_finite())), "{grey:?}");
    }

    #[test]
    fn the_cached_analysis_gives_the_same_colors() {
        let analysis = &samples()[0];
        let saved: Analysis = serde_json::from_str(&serde_json::to_string(analysis).unwrap()).unwrap();
        assert_eq!(roles(&saved), roles(analysis));
    }
}
