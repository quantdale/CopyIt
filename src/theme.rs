//! Color themes and visual styling for CopyIt.
//!
//! Provides 37 selectable color themes, each with custom egui::Visuals for background,
//! text, panels, buttons, and accent colors. Themes are persisted in config.json
//! and applied every frame via update().

use eframe::egui;
use std::fmt;
use std::str::FromStr;

/// Selectable color themes for CopyIt.
/// Each theme builds custom egui::Visuals by specifying base colors (background, panel, text, accent)
/// and using helper functions (mix, themed_widgets, build_visuals) to compute all derived colors consistently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    Nord,
    Dracula,
    SolarizedDark,
    SolarizedLight,
    GruvboxDark,
    GruvboxLight,
    CatppuccinMocha,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    TokyoNight,
    TokyoNightStorm,
    TokyoNightLight,
    OneDark,
    OneLight,
    Monokai,
    MonokaiPro,
    GithubDark,
    GithubLight,
    AyuDark,
    AyuLight,
    AyuMirage,
    RosePine,
    RosePineMoon,
    RosePineDawn,
    EverforestDark,
    EverforestLight,
    MaterialOcean,
    MaterialPalenight,
    Kanagawa,
    NightOwl,
    Zenburn,
    SynthwaveEighties,
    Cobalt2,
    HorizonDark,
}

impl Theme {
    /// Returns a static slice of all available themes in order. Used to populate the theme selector dropdown.
    pub fn all() -> &'static [Theme] {
        &[
            Theme::Dark,
            Theme::Light,
            Theme::Nord,
            Theme::Dracula,
            Theme::SolarizedDark,
            Theme::SolarizedLight,
            Theme::GruvboxDark,
            Theme::GruvboxLight,
            Theme::CatppuccinMocha,
            Theme::CatppuccinLatte,
            Theme::CatppuccinFrappe,
            Theme::CatppuccinMacchiato,
            Theme::TokyoNight,
            Theme::TokyoNightStorm,
            Theme::TokyoNightLight,
            Theme::OneDark,
            Theme::OneLight,
            Theme::Monokai,
            Theme::MonokaiPro,
            Theme::GithubDark,
            Theme::GithubLight,
            Theme::AyuDark,
            Theme::AyuLight,
            Theme::AyuMirage,
            Theme::RosePine,
            Theme::RosePineMoon,
            Theme::RosePineDawn,
            Theme::EverforestDark,
            Theme::EverforestLight,
            Theme::MaterialOcean,
            Theme::MaterialPalenight,
            Theme::Kanagawa,
            Theme::NightOwl,
            Theme::Zenburn,
            Theme::SynthwaveEighties,
            Theme::Cobalt2,
            Theme::HorizonDark,
        ]
    }

    /// Generates the complete egui::Visuals color scheme for this theme. Applied to egui context each frame.
    pub fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Dark => dark_theme(),
            Theme::Light => light_theme(),
            Theme::Nord => nord_theme(),
            Theme::Dracula => dracula_theme(),
            Theme::SolarizedDark => solarized_dark_theme(),
            Theme::SolarizedLight => solarized_light_theme(),
            Theme::GruvboxDark => gruvbox_dark_theme(),
            Theme::GruvboxLight => gruvbox_light_theme(),
            Theme::CatppuccinMocha => catppuccin_mocha_theme(),
            Theme::CatppuccinLatte => catppuccin_latte_theme(),
            Theme::CatppuccinFrappe => catppuccin_frappe_theme(),
            Theme::CatppuccinMacchiato => catppuccin_macchiato_theme(),
            Theme::TokyoNight => tokyo_night_theme(),
            Theme::TokyoNightStorm => tokyo_night_storm_theme(),
            Theme::TokyoNightLight => tokyo_night_light_theme(),
            Theme::OneDark => one_dark_theme(),
            Theme::OneLight => one_light_theme(),
            Theme::Monokai => monokai_theme(),
            Theme::MonokaiPro => monokai_pro_theme(),
            Theme::GithubDark => github_dark_theme(),
            Theme::GithubLight => github_light_theme(),
            Theme::AyuDark => ayu_dark_theme(),
            Theme::AyuLight => ayu_light_theme(),
            Theme::AyuMirage => ayu_mirage_theme(),
            Theme::RosePine => rose_pine_theme(),
            Theme::RosePineMoon => rose_pine_moon_theme(),
            Theme::RosePineDawn => rose_pine_dawn_theme(),
            Theme::EverforestDark => everforest_dark_theme(),
            Theme::EverforestLight => everforest_light_theme(),
            Theme::MaterialOcean => material_ocean_theme(),
            Theme::MaterialPalenight => material_palenight_theme(),
            Theme::Kanagawa => kanagawa_theme(),
            Theme::NightOwl => night_owl_theme(),
            Theme::Zenburn => zenburn_theme(),
            Theme::SynthwaveEighties => synthwave_eighties_theme(),
            Theme::Cobalt2 => cobalt2_theme(),
            Theme::HorizonDark => horizon_dark_theme(),
        }
    }
}

/// Converts a theme to its human-readable display string (e.g., "Solarized Dark"). Used in UI dropdowns and config serialization.
impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Theme::Dark => write!(f, "Dark"),
            Theme::Light => write!(f, "Light"),
            Theme::Nord => write!(f, "Nord"),
            Theme::Dracula => write!(f, "Dracula"),
            Theme::SolarizedDark => write!(f, "Solarized Dark"),
            Theme::SolarizedLight => write!(f, "Solarized Light"),
            Theme::GruvboxDark => write!(f, "Gruvbox Dark"),
            Theme::GruvboxLight => write!(f, "Gruvbox Light"),
            Theme::CatppuccinMocha => write!(f, "Catppuccin Mocha"),
            Theme::CatppuccinLatte => write!(f, "Catppuccin Latte"),
            Theme::CatppuccinFrappe => write!(f, "Catppuccin Frappe"),
            Theme::CatppuccinMacchiato => write!(f, "Catppuccin Macchiato"),
            Theme::TokyoNight => write!(f, "Tokyo Night"),
            Theme::TokyoNightStorm => write!(f, "Tokyo Night Storm"),
            Theme::TokyoNightLight => write!(f, "Tokyo Night Light"),
            Theme::OneDark => write!(f, "One Dark"),
            Theme::OneLight => write!(f, "One Light"),
            Theme::Monokai => write!(f, "Monokai"),
            Theme::MonokaiPro => write!(f, "Monokai Pro"),
            Theme::GithubDark => write!(f, "GitHub Dark"),
            Theme::GithubLight => write!(f, "GitHub Light"),
            Theme::AyuDark => write!(f, "Ayu Dark"),
            Theme::AyuLight => write!(f, "Ayu Light"),
            Theme::AyuMirage => write!(f, "Ayu Mirage"),
            Theme::RosePine => write!(f, "Rose Pine"),
            Theme::RosePineMoon => write!(f, "Rose Pine Moon"),
            Theme::RosePineDawn => write!(f, "Rose Pine Dawn"),
            Theme::EverforestDark => write!(f, "Everforest Dark"),
            Theme::EverforestLight => write!(f, "Everforest Light"),
            Theme::MaterialOcean => write!(f, "Material Ocean"),
            Theme::MaterialPalenight => write!(f, "Material Palenight"),
            Theme::Kanagawa => write!(f, "Kanagawa"),
            Theme::NightOwl => write!(f, "Night Owl"),
            Theme::Zenburn => write!(f, "Zenburn"),
            Theme::SynthwaveEighties => write!(f, "Synthwave '84"),
            Theme::Cobalt2 => write!(f, "Cobalt2"),
            Theme::HorizonDark => write!(f, "Horizon Dark"),
        }
    }
}

/// Parses a theme name from config.json (case-insensitive, handles underscores/spaces). Returns Err(()) on unknown theme.
impl FromStr for Theme {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dark" => Ok(Theme::Dark),
            "light" => Ok(Theme::Light),
            "nord" => Ok(Theme::Nord),
            "dracula" => Ok(Theme::Dracula),
            "solarized dark" | "solarized_dark" => Ok(Theme::SolarizedDark),
            "solarized light" | "solarized_light" => Ok(Theme::SolarizedLight),
            "gruvbox dark" | "gruvbox_dark" => Ok(Theme::GruvboxDark),
            "gruvbox light" | "gruvbox_light" => Ok(Theme::GruvboxLight),
            "catppuccin mocha" | "catppuccin_mocha" => Ok(Theme::CatppuccinMocha),
            "catppuccin latte" | "catppuccin_latte" => Ok(Theme::CatppuccinLatte),
            "catppuccin frappe" | "catppuccin_frappe" => Ok(Theme::CatppuccinFrappe),
            "catppuccin macchiato" | "catppuccin_macchiato" => Ok(Theme::CatppuccinMacchiato),
            "tokyo night" | "tokyo_night" => Ok(Theme::TokyoNight),
            "tokyo night storm" | "tokyo_night_storm" => Ok(Theme::TokyoNightStorm),
            "tokyo night light" | "tokyo_night_light" => Ok(Theme::TokyoNightLight),
            "one dark" | "one_dark" => Ok(Theme::OneDark),
            "one light" | "one_light" => Ok(Theme::OneLight),
            "monokai" => Ok(Theme::Monokai),
            "monokai pro" | "monokai_pro" => Ok(Theme::MonokaiPro),
            "github dark" | "github_dark" => Ok(Theme::GithubDark),
            "github light" | "github_light" => Ok(Theme::GithubLight),
            "ayu dark" | "ayu_dark" => Ok(Theme::AyuDark),
            "ayu light" | "ayu_light" => Ok(Theme::AyuLight),
            "ayu mirage" | "ayu_mirage" => Ok(Theme::AyuMirage),
            "rose pine" | "rose_pine" => Ok(Theme::RosePine),
            "rose pine moon" | "rose_pine_moon" => Ok(Theme::RosePineMoon),
            "rose pine dawn" | "rose_pine_dawn" => Ok(Theme::RosePineDawn),
            "everforest dark" | "everforest_dark" => Ok(Theme::EverforestDark),
            "everforest light" | "everforest_light" => Ok(Theme::EverforestLight),
            "material ocean" | "material_ocean" => Ok(Theme::MaterialOcean),
            "material palenight" | "material_palenight" => Ok(Theme::MaterialPalenight),
            "kanagawa" => Ok(Theme::Kanagawa),
            "night owl" | "night_owl" => Ok(Theme::NightOwl),
            "zenburn" => Ok(Theme::Zenburn),
            "synthwave '84" | "synthwave 84" | "synthwave_84" | "synthwave_eighties" => {
                Ok(Theme::SynthwaveEighties)
            }
            "cobalt2" | "cobalt 2" => Ok(Theme::Cobalt2),
            "horizon dark" | "horizon_dark" => Ok(Theme::HorizonDark),
            _ => Err(()),
        }
    }
}

/// Linearly interpolates between two colors. t=0 returns a, t=1 returns b.
/// Used to compute derived colors (hover, active states) from base colors.
fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let r = (a.r() as f32 * (1.0 - t) + b.r() as f32 * t) as u8;
    let g = (a.g() as f32 * (1.0 - t) + b.g() as f32 * t) as u8;
    let bl = (a.b() as f32 * (1.0 - t) + b.b() as f32 * t) as u8;
    egui::Color32::from_rgb(r, g, bl)
}

/// Builds the Widgets style (buttons, text edits, checkboxes) from base colors.
/// Computes hover and active states by mixing the surface and accent colors,
/// and ensures interactive elements have consistent visual feedback.
fn themed_widgets(
    base: &egui::Visuals,
    surface: egui::Color32,
    fill: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
) -> egui::style::Widgets {
    let hover = mix(surface, accent, 0.22);
    let active = mix(surface, accent, 0.42);
    let subtle_text = mix(text, surface, 0.35);

    let mut w = base.widgets.clone();
    w.noninteractive.bg_fill = surface;
    w.noninteractive.fg_stroke.color = subtle_text;
    w.noninteractive.bg_stroke.color = mix(surface, text, 0.12);

    w.inactive.bg_fill = fill;
    w.inactive.fg_stroke.color = text;
    w.inactive.bg_stroke.color = mix(surface, accent, 0.35);

    w.hovered.bg_fill = hover;
    w.hovered.fg_stroke.color = text;
    w.hovered.bg_stroke.color = accent;

    w.active.bg_fill = active;
    w.active.fg_stroke.color = text;
    w.active.bg_stroke.color = accent;

    w.open.bg_fill = hover;
    w.open.fg_stroke.color = text;
    w.open.bg_stroke.color = accent;

    w
}

/// Constructs a complete egui::Visuals from a theme's base colors.
/// Takes dark/light mode flag and five colors (background, panel, extreme, accent, text),
/// then computes all UI element colors (buttons, strokes, selection, hyperlinks) using mix() for consistency.
/// This is the core builder used by all individual theme functions.
fn build_visuals(
    dark: bool,
    bg: egui::Color32,
    panel: egui::Color32,
    extreme: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
) -> egui::Visuals {
    let base = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let mut v = base;
    v.dark_mode = dark;
    v.panel_fill = panel;
    v.window_fill = bg;
    v.extreme_bg_color = extreme;
    v.faint_bg_color = panel;
    v.code_bg_color = extreme;
    v.hyperlink_color = accent;
    v.selection.bg_fill = mix(panel, accent, 0.75);
    v.selection.stroke.color = text;
    v.override_text_color = None;
    v.widgets = themed_widgets(&v, panel, bg, accent, text);
    v
}

// Individual theme builders. Each encapsulates a cohesive color palette
// (e.g., Nord: polar night + frost + snow storm; Dracula: dark purples + bright accents).
// All follow the same pattern: call build_visuals() with dark/light mode and five RGB colors.
// Theme colors are sourced from official palette definitions to ensure visual fidelity.

fn dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x11, 0x18, 0x27), // window
        egui::Color32::from_rgb(0x1f, 0x29, 0x37), // panel
        egui::Color32::from_rgb(0x03, 0x07, 0x12), // extreme
        egui::Color32::from_rgb(0x3b, 0x82, 0xf6), // accent
        egui::Color32::WHITE,
    )
}

fn light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xff, 0xff, 0xff), // window
        egui::Color32::from_rgb(0xf3, 0xf4, 0xf6), // panel
        egui::Color32::from_rgb(0xe5, 0xe7, 0xeb), // extreme
        egui::Color32::from_rgb(0x25, 0x63, 0xeb), // accent
        egui::Color32::from_rgb(0x11, 0x17, 0x27), // text
    )
}

fn nord_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x2e, 0x34, 0x40), // polar night 0
        egui::Color32::from_rgb(0x3b, 0x42, 0x52), // polar night 1
        egui::Color32::from_rgb(0x24, 0x29, 0x33), // extreme
        egui::Color32::from_rgb(0x88, 0xc0, 0xd0), // frost
        egui::Color32::from_rgb(0xd8, 0xde, 0xe9), // snow storm
    )
}

fn dracula_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x28, 0x2a, 0x36), // background
        egui::Color32::from_rgb(0x44, 0x47, 0x5a), // current line
        egui::Color32::from_rgb(0x21, 0x23, 0x2d), // extreme
        egui::Color32::from_rgb(0xbd, 0x93, 0xf9), // purple
        egui::Color32::from_rgb(0xf8, 0xf8, 0xf2), // foreground
    )
}

fn solarized_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x00, 0x2b, 0x36), // base03
        egui::Color32::from_rgb(0x07, 0x36, 0x42), // base02
        egui::Color32::from_rgb(0x00, 0x21, 0x2b), // extreme
        egui::Color32::from_rgb(0x26, 0x8b, 0xd2), // blue
        egui::Color32::from_rgb(0x83, 0x94, 0x96), // base0
    )
}

fn solarized_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xfd, 0xf6, 0xe3), // base3
        egui::Color32::from_rgb(0xee, 0xe8, 0xd5), // base2
        egui::Color32::from_rgb(0xe3, 0xdb, 0xc0), // extreme
        egui::Color32::from_rgb(0x26, 0x8b, 0xd2), // blue
        egui::Color32::from_rgb(0x58, 0x6e, 0x75), // base01
    )
}

fn gruvbox_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x28, 0x28, 0x28), // bg
        egui::Color32::from_rgb(0x3c, 0x38, 0x36), // bg1
        egui::Color32::from_rgb(0x1d, 0x20, 0x21), // extreme
        egui::Color32::from_rgb(0xd7, 0x99, 0x21), // yellow
        egui::Color32::from_rgb(0xeb, 0xdb, 0xb2), // fg
    )
}

fn gruvbox_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xfb, 0xf1, 0xc7), // bg0
        egui::Color32::from_rgb(0xeb, 0xdb, 0xb2), // bg1
        egui::Color32::from_rgb(0xd5, 0xc4, 0xa1), // bg2 extreme
        egui::Color32::from_rgb(0xd6, 0x5d, 0x0e), // orange
        egui::Color32::from_rgb(0x3c, 0x38, 0x36), // fg1
    )
}

fn catppuccin_mocha_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x1e, 0x1e, 0x2e), // base
        egui::Color32::from_rgb(0x31, 0x32, 0x44), // surface0
        egui::Color32::from_rgb(0x18, 0x18, 0x25), // mantle
        egui::Color32::from_rgb(0xb4, 0xbe, 0xfe), // lavender
        egui::Color32::from_rgb(0xcd, 0xd6, 0xf4), // text
    )
}

fn catppuccin_latte_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xef, 0xf1, 0xf5), // base
        egui::Color32::from_rgb(0xe6, 0xe9, 0xef), // mantle
        egui::Color32::from_rgb(0xdc, 0xe0, 0xe8), // crust extreme
        egui::Color32::from_rgb(0x72, 0x87, 0xfd), // lavender
        egui::Color32::from_rgb(0x4c, 0x4f, 0x69), // text
    )
}

fn catppuccin_frappe_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x30, 0x34, 0x46), // base
        egui::Color32::from_rgb(0x41, 0x45, 0x59), // surface0
        egui::Color32::from_rgb(0x29, 0x2c, 0x3c), // mantle extreme
        egui::Color32::from_rgb(0xba, 0xbb, 0xf1), // lavender
        egui::Color32::from_rgb(0xc6, 0xd0, 0xf5), // text
    )
}

fn catppuccin_macchiato_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x24, 0x27, 0x3a), // base
        egui::Color32::from_rgb(0x36, 0x3a, 0x4f), // surface0
        egui::Color32::from_rgb(0x1e, 0x20, 0x30), // mantle extreme
        egui::Color32::from_rgb(0xb7, 0xbd, 0xf8), // lavender
        egui::Color32::from_rgb(0xca, 0xd3, 0xf5), // text
    )
}

fn tokyo_night_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x1a, 0x1b, 0x26), // bg
        egui::Color32::from_rgb(0x29, 0x2e, 0x42), // bg_highlight
        egui::Color32::from_rgb(0x16, 0x16, 0x1e), // bg_dark extreme
        egui::Color32::from_rgb(0x7a, 0xa2, 0xf7), // blue
        egui::Color32::from_rgb(0xc0, 0xca, 0xf5), // fg
    )
}

fn tokyo_night_storm_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x24, 0x28, 0x3b), // bg
        egui::Color32::from_rgb(0x36, 0x3b, 0x54), // bg_highlight
        egui::Color32::from_rgb(0x1f, 0x23, 0x35), // extreme
        egui::Color32::from_rgb(0x7a, 0xa2, 0xf7), // blue
        egui::Color32::from_rgb(0xc0, 0xca, 0xf5), // fg
    )
}

fn tokyo_night_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xe1, 0xe2, 0xe7), // bg
        egui::Color32::from_rgb(0xd0, 0xd3, 0xdd), // panel
        egui::Color32::from_rgb(0xc3, 0xc6, 0xd4), // extreme
        egui::Color32::from_rgb(0x29, 0x59, 0xaa), // blue
        egui::Color32::from_rgb(0x34, 0x3b, 0x58), // fg
    )
}

fn one_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x28, 0x2c, 0x34), // bg
        egui::Color32::from_rgb(0x2c, 0x31, 0x3c), // panel
        egui::Color32::from_rgb(0x21, 0x25, 0x2b), // extreme
        egui::Color32::from_rgb(0x61, 0xaf, 0xef), // blue
        egui::Color32::from_rgb(0xab, 0xb2, 0xbf), // fg
    )
}

fn one_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xfa, 0xfa, 0xfa), // bg
        egui::Color32::from_rgb(0xea, 0xea, 0xeb), // panel
        egui::Color32::from_rgb(0xe3, 0xe3, 0xe4), // extreme
        egui::Color32::from_rgb(0x40, 0x78, 0xf2), // blue
        egui::Color32::from_rgb(0x38, 0x3a, 0x42), // fg
    )
}

fn monokai_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x27, 0x28, 0x22), // bg
        egui::Color32::from_rgb(0x3e, 0x3d, 0x32), // panel
        egui::Color32::from_rgb(0x1e, 0x1f, 0x1c), // extreme
        egui::Color32::from_rgb(0xf9, 0x26, 0x72), // pink
        egui::Color32::from_rgb(0xf8, 0xf8, 0xf2), // fg
    )
}

fn monokai_pro_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x2d, 0x2a, 0x2e), // bg
        egui::Color32::from_rgb(0x40, 0x3e, 0x41), // panel
        egui::Color32::from_rgb(0x22, 0x1f, 0x22), // extreme
        egui::Color32::from_rgb(0xff, 0xd8, 0x66), // yellow
        egui::Color32::from_rgb(0xfc, 0xfc, 0xfa), // fg
    )
}

fn github_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x0d, 0x11, 0x17), // bg
        egui::Color32::from_rgb(0x16, 0x1b, 0x22), // panel
        egui::Color32::from_rgb(0x01, 0x04, 0x09), // extreme
        egui::Color32::from_rgb(0x58, 0xa6, 0xff), // blue
        egui::Color32::from_rgb(0xc9, 0xd1, 0xd9), // fg
    )
}

fn github_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xff, 0xff, 0xff), // bg
        egui::Color32::from_rgb(0xf6, 0xf8, 0xfa), // panel
        egui::Color32::from_rgb(0xea, 0xee, 0xf2), // extreme
        egui::Color32::from_rgb(0x09, 0x69, 0xda), // blue
        egui::Color32::from_rgb(0x24, 0x29, 0x2f), // fg
    )
}

fn ayu_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x0a, 0x0e, 0x14), // bg
        egui::Color32::from_rgb(0x13, 0x17, 0x21), // panel
        egui::Color32::from_rgb(0x06, 0x0a, 0x10), // extreme
        egui::Color32::from_rgb(0xff, 0x8f, 0x40), // orange
        egui::Color32::from_rgb(0xb3, 0xb1, 0xad), // fg
    )
}

fn ayu_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xfa, 0xfa, 0xfa), // bg
        egui::Color32::from_rgb(0xf0, 0xf0, 0xf0), // panel
        egui::Color32::from_rgb(0xe7, 0xe8, 0xe9), // extreme
        egui::Color32::from_rgb(0xfa, 0x8d, 0x3e), // orange
        egui::Color32::from_rgb(0x5c, 0x61, 0x66), // fg
    )
}

fn ayu_mirage_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x1f, 0x24, 0x30), // bg
        egui::Color32::from_rgb(0x23, 0x28, 0x34), // panel
        egui::Color32::from_rgb(0x17, 0x1b, 0x24), // extreme
        egui::Color32::from_rgb(0xff, 0xcc, 0x66), // gold
        egui::Color32::from_rgb(0xcb, 0xcc, 0xc6), // fg
    )
}

fn rose_pine_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x19, 0x17, 0x24), // base
        egui::Color32::from_rgb(0x1f, 0x1d, 0x2e), // surface
        egui::Color32::from_rgb(0x14, 0x12, 0x20), // extreme
        egui::Color32::from_rgb(0xc4, 0xa7, 0xe7), // iris
        egui::Color32::from_rgb(0xe0, 0xde, 0xf4), // text
    )
}

fn rose_pine_moon_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x23, 0x21, 0x36), // base
        egui::Color32::from_rgb(0x2a, 0x27, 0x3f), // surface
        egui::Color32::from_rgb(0x1c, 0x1a, 0x2b), // extreme
        egui::Color32::from_rgb(0xc4, 0xa7, 0xe7), // iris
        egui::Color32::from_rgb(0xe0, 0xde, 0xf4), // text
    )
}

fn rose_pine_dawn_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xfa, 0xf4, 0xed), // base
        egui::Color32::from_rgb(0xf2, 0xe9, 0xe1), // overlay
        egui::Color32::from_rgb(0xe5, 0xda, 0xcb), // extreme
        egui::Color32::from_rgb(0x90, 0x7a, 0xa9), // iris
        egui::Color32::from_rgb(0x57, 0x52, 0x79), // text
    )
}

fn everforest_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x2d, 0x35, 0x3b), // bg0
        egui::Color32::from_rgb(0x3d, 0x48, 0x4d), // bg2
        egui::Color32::from_rgb(0x23, 0x2a, 0x2e), // extreme
        egui::Color32::from_rgb(0xa7, 0xc0, 0x80), // green
        egui::Color32::from_rgb(0xd3, 0xc6, 0xaa), // fg
    )
}

fn everforest_light_theme() -> egui::Visuals {
    build_visuals(
        false,
        egui::Color32::from_rgb(0xff, 0xfb, 0xef), // bg0
        egui::Color32::from_rgb(0xf2, 0xef, 0xdf), // bg1
        egui::Color32::from_rgb(0xe6, 0xe2, 0xcc), // extreme
        egui::Color32::from_rgb(0x8d, 0xa1, 0x01), // green
        egui::Color32::from_rgb(0x5c, 0x6a, 0x72), // fg
    )
}

fn material_ocean_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x0f, 0x11, 0x1a), // bg
        egui::Color32::from_rgb(0x1f, 0x22, 0x33), // panel
        egui::Color32::from_rgb(0x09, 0x0b, 0x10), // extreme
        egui::Color32::from_rgb(0x82, 0xaa, 0xff), // blue
        egui::Color32::from_rgb(0xee, 0xff, 0xff), // fg
    )
}

fn material_palenight_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x29, 0x2d, 0x3e), // bg
        egui::Color32::from_rgb(0x32, 0x37, 0x4d), // panel
        egui::Color32::from_rgb(0x1e, 0x21, 0x32), // extreme
        egui::Color32::from_rgb(0xc7, 0x92, 0xea), // purple
        egui::Color32::from_rgb(0xa6, 0xac, 0xcd), // fg
    )
}

fn kanagawa_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x1f, 0x1f, 0x28), // sumiInk1
        egui::Color32::from_rgb(0x36, 0x36, 0x46), // sumiInk3
        egui::Color32::from_rgb(0x16, 0x16, 0x1d), // sumiInk0 extreme
        egui::Color32::from_rgb(0x7e, 0x9c, 0xd8), // waveBlue
        egui::Color32::from_rgb(0xdc, 0xd7, 0xba), // fujiWhite
    )
}

fn night_owl_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x01, 0x16, 0x27), // bg
        egui::Color32::from_rgb(0x1d, 0x3b, 0x53), // panel/selection
        egui::Color32::from_rgb(0x01, 0x0e, 0x1a), // extreme
        egui::Color32::from_rgb(0x82, 0xaa, 0xff), // blue
        egui::Color32::from_rgb(0xd6, 0xde, 0xeb), // fg
    )
}

fn zenburn_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x3f, 0x3f, 0x3f), // bg
        egui::Color32::from_rgb(0x4f, 0x4f, 0x4f), // panel
        egui::Color32::from_rgb(0x2b, 0x2b, 0x2b), // extreme
        egui::Color32::from_rgb(0xdf, 0xaf, 0x8f), // orange
        egui::Color32::from_rgb(0xdc, 0xdc, 0xcc), // fg
    )
}

fn synthwave_eighties_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x2a, 0x21, 0x39), // bg
        egui::Color32::from_rgb(0x34, 0x29, 0x4f), // panel
        egui::Color32::from_rgb(0x1a, 0x13, 0x30), // extreme
        egui::Color32::from_rgb(0xff, 0x7e, 0xdb), // pink
        egui::Color32::from_rgb(0xf4, 0xee, 0xe4), // fg
    )
}

fn cobalt2_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x19, 0x35, 0x49), // bg
        egui::Color32::from_rgb(0x1f, 0x46, 0x62), // panel
        egui::Color32::from_rgb(0x12, 0x2b, 0x3f), // extreme
        egui::Color32::from_rgb(0xff, 0xc6, 0x00), // yellow
        egui::Color32::WHITE,
    )
}

fn horizon_dark_theme() -> egui::Visuals {
    build_visuals(
        true,
        egui::Color32::from_rgb(0x1c, 0x1e, 0x26), // bg
        egui::Color32::from_rgb(0x23, 0x25, 0x30), // panel
        egui::Color32::from_rgb(0x16, 0x16, 0x1c), // extreme
        egui::Color32::from_rgb(0xe9, 0x56, 0x78), // red/pink
        egui::Color32::from_rgb(0xe0, 0xe0, 0xe0), // fg
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_at_least_twenty_new_themes() {
        assert!(Theme::all().len() >= 27);
    }

    #[test]
    fn all_themes_round_trip_through_display_and_from_str() {
        for theme in Theme::all() {
            let label = theme.to_string();
            let parsed: Theme = label.parse().unwrap_or_else(|_| {
                panic!("could not parse Theme::to_string() output {label:?} back via FromStr")
            });
            assert_eq!(*theme, parsed, "round-trip mismatch for {label:?}");
        }
    }

    #[test]
    fn all_theme_names_are_unique() {
        let mut names: Vec<String> = Theme::all().iter().map(|t| t.to_string()).collect();
        let before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate theme display names found");
    }
}
