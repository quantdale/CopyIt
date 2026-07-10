use eframe::egui;
use std::fmt;
use std::str::FromStr;

/// Selectable color themes for CopyIt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    Nord,
    Dracula,
    SolarizedDark,
    GruvboxDark,
    CatppuccinMocha,
}

impl Theme {
    pub fn all() -> &'static [Theme] {
        &[
            Theme::Dark,
            Theme::Light,
            Theme::Nord,
            Theme::Dracula,
            Theme::SolarizedDark,
            Theme::GruvboxDark,
            Theme::CatppuccinMocha,
        ]
    }

    pub fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Dark => dark_theme(),
            Theme::Light => light_theme(),
            Theme::Nord => nord_theme(),
            Theme::Dracula => dracula_theme(),
            Theme::SolarizedDark => solarized_dark_theme(),
            Theme::GruvboxDark => gruvbox_dark_theme(),
            Theme::CatppuccinMocha => catppuccin_mocha_theme(),
        }
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Theme::Dark => write!(f, "Dark"),
            Theme::Light => write!(f, "Light"),
            Theme::Nord => write!(f, "Nord"),
            Theme::Dracula => write!(f, "Dracula"),
            Theme::SolarizedDark => write!(f, "Solarized Dark"),
            Theme::GruvboxDark => write!(f, "Gruvbox Dark"),
            Theme::CatppuccinMocha => write!(f, "Catppuccin Mocha"),
        }
    }
}

impl FromStr for Theme {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dark" => Ok(Theme::Dark),
            "light" => Ok(Theme::Light),
            "nord" => Ok(Theme::Nord),
            "dracula" => Ok(Theme::Dracula),
            "solarized dark" | "solarized_dark" => Ok(Theme::SolarizedDark),
            "gruvbox dark" | "gruvbox_dark" => Ok(Theme::GruvboxDark),
            "catppuccin mocha" | "catppuccin_mocha" => Ok(Theme::CatppuccinMocha),
            _ => Err(()),
        }
    }
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let r = (a.r() as f32 * (1.0 - t) + b.r() as f32 * t) as u8;
    let g = (a.g() as f32 * (1.0 - t) + b.g() as f32 * t) as u8;
    let bl = (a.b() as f32 * (1.0 - t) + b.b() as f32 * t) as u8;
    egui::Color32::from_rgb(r, g, bl)
}

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
