//! Framework-free colors and board themes.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    #[allow(clippy::self_named_constructors)]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_u8(self) -> [u8; 4] {
        [
            (self.r.clamp(0.0, 1.0) * 255.0) as u8,
            (self.g.clamp(0.0, 1.0) * 255.0) as u8,
            (self.b.clamp(0.0, 1.0) * 255.0) as u8,
            (self.a.clamp(0.0, 1.0) * 255.0) as u8,
        ]
    }

    pub fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// Board color theme — also controls the entire GUI palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BoardTheme {
    Classic,
    Emerald,
    Ocean,
    Royal,
    Walnut,
    Midnight,
    Forest,
    Sakura,
}

impl std::fmt::Display for BoardTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Classic => write!(f, "Classic"),
            Self::Emerald => write!(f, "Emerald"),
            Self::Ocean => write!(f, "Ocean"),
            Self::Royal => write!(f, "Royal"),
            Self::Walnut => write!(f, "Walnut"),
            Self::Midnight => write!(f, "Midnight"),
            Self::Forest => write!(f, "Forest"),
            Self::Sakura => write!(f, "Sakura"),
        }
    }
}

/// Full GUI color palette so themes control the entire app.
#[derive(Debug, Clone, Copy)]
pub struct GuiPalette {
    pub bg: Rgba,
    pub panel: Rgba,
    pub sidebar: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub accent: Rgba,
    pub accent_alt: Rgba,
    pub border: Rgba,
}

/// Square colors for the board grid.
#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub light: Rgba,
    pub dark: Rgba,
    pub selected: Rgba,
    pub last_light: Rgba,
    pub last_dark: Rgba,
    pub legal_light: Rgba,
    pub legal_dark: Rgba,
}

impl ThemeColors {
    pub fn coord_color(self, is_light: bool) -> Rgba {
        if is_light {
            Rgba::rgba(self.dark.r, self.dark.g, self.dark.b, 0.8)
        } else {
            Rgba::rgba(self.light.r, self.light.g, self.light.b, 0.8)
        }
    }
}

impl BoardTheme {
    pub const ALL: [BoardTheme; 8] = [
        BoardTheme::Classic,
        BoardTheme::Emerald,
        BoardTheme::Ocean,
        BoardTheme::Royal,
        BoardTheme::Walnut,
        BoardTheme::Midnight,
        BoardTheme::Forest,
        BoardTheme::Sakura,
    ];

    pub fn gui_palette(self) -> GuiPalette {
        match self {
            BoardTheme::Classic | BoardTheme::Walnut => GuiPalette {
                bg: Rgba::rgb(0.102, 0.102, 0.180),
                panel: Rgba::rgb(0.086, 0.129, 0.243),
                sidebar: Rgba::rgb(0.059, 0.204, 0.376),
                text_primary: Rgba::rgb(0.96, 0.96, 0.96),
                text_secondary: Rgba::rgb(0.627, 0.627, 0.690),
                accent: Rgba::rgb(0.914, 0.271, 0.376),
                accent_alt: Rgba::rgb(0.325, 0.749, 0.616),
                border: Rgba::rgb(0.16, 0.18, 0.28),
            },
            BoardTheme::Emerald | BoardTheme::Forest => GuiPalette {
                bg: Rgba::rgb(0.067, 0.118, 0.086),
                panel: Rgba::rgb(0.090, 0.157, 0.114),
                sidebar: Rgba::rgb(0.118, 0.216, 0.153),
                text_primary: Rgba::rgb(0.92, 0.96, 0.93),
                text_secondary: Rgba::rgb(0.55, 0.65, 0.58),
                accent: Rgba::rgb(0.325, 0.749, 0.416),
                accent_alt: Rgba::rgb(0.706, 0.569, 0.235),
                border: Rgba::rgb(0.14, 0.22, 0.16),
            },
            BoardTheme::Ocean => GuiPalette {
                bg: Rgba::rgb(0.059, 0.094, 0.149),
                panel: Rgba::rgb(0.078, 0.129, 0.204),
                sidebar: Rgba::rgb(0.098, 0.176, 0.290),
                text_primary: Rgba::rgb(0.92, 0.95, 0.98),
                text_secondary: Rgba::rgb(0.55, 0.63, 0.72),
                accent: Rgba::rgb(0.357, 0.645, 0.949),
                accent_alt: Rgba::rgb(0.325, 0.749, 0.616),
                border: Rgba::rgb(0.12, 0.18, 0.27),
            },
            BoardTheme::Royal => GuiPalette {
                bg: Rgba::rgb(0.110, 0.075, 0.165),
                panel: Rgba::rgb(0.145, 0.102, 0.220),
                sidebar: Rgba::rgb(0.188, 0.133, 0.290),
                text_primary: Rgba::rgb(0.95, 0.93, 0.97),
                text_secondary: Rgba::rgb(0.62, 0.58, 0.68),
                accent: Rgba::rgb(0.700, 0.400, 0.900),
                accent_alt: Rgba::rgb(0.914, 0.271, 0.576),
                border: Rgba::rgb(0.18, 0.14, 0.26),
            },
            BoardTheme::Midnight => GuiPalette {
                bg: Rgba::rgb(0.047, 0.047, 0.082),
                panel: Rgba::rgb(0.063, 0.067, 0.118),
                sidebar: Rgba::rgb(0.082, 0.090, 0.161),
                text_primary: Rgba::rgb(0.90, 0.92, 0.96),
                text_secondary: Rgba::rgb(0.50, 0.53, 0.62),
                accent: Rgba::rgb(0.400, 0.600, 1.000),
                accent_alt: Rgba::rgb(0.914, 0.271, 0.376),
                border: Rgba::rgb(0.10, 0.11, 0.18),
            },
            BoardTheme::Sakura => GuiPalette {
                bg: Rgba::rgb(0.141, 0.082, 0.106),
                panel: Rgba::rgb(0.188, 0.110, 0.145),
                sidebar: Rgba::rgb(0.243, 0.149, 0.192),
                text_primary: Rgba::rgb(0.98, 0.93, 0.95),
                text_secondary: Rgba::rgb(0.68, 0.56, 0.62),
                accent: Rgba::rgb(0.957, 0.400, 0.600),
                accent_alt: Rgba::rgb(0.706, 0.569, 0.235),
                border: Rgba::rgb(0.22, 0.14, 0.18),
            },
        }
    }

    pub fn colors(self) -> ThemeColors {
        match self {
            BoardTheme::Classic => ThemeColors {
                light: Rgba::rgb(0.941, 0.851, 0.710),
                dark: Rgba::rgb(0.710, 0.533, 0.388),
                selected: Rgba::rgb(0.510, 0.592, 0.412),
                last_light: Rgba::rgb(0.969, 0.969, 0.514),
                last_dark: Rgba::rgb(0.855, 0.824, 0.459),
                legal_light: Rgba::rgb(0.820, 0.878, 0.600),
                legal_dark: Rgba::rgb(0.680, 0.753, 0.490),
            },
            BoardTheme::Emerald => ThemeColors {
                light: Rgba::rgb(0.933, 0.933, 0.824),
                dark: Rgba::rgb(0.463, 0.588, 0.337),
                selected: Rgba::rgb(0.725, 0.769, 0.286),
                last_light: Rgba::rgb(0.957, 0.969, 0.580),
                last_dark: Rgba::rgb(0.690, 0.780, 0.412),
                legal_light: Rgba::rgb(0.820, 0.878, 0.600),
                legal_dark: Rgba::rgb(0.580, 0.700, 0.420),
            },
            BoardTheme::Ocean => ThemeColors {
                light: Rgba::rgb(0.871, 0.890, 0.902),
                dark: Rgba::rgb(0.357, 0.545, 0.749),
                selected: Rgba::rgb(0.400, 0.580, 0.800),
                last_light: Rgba::rgb(0.690, 0.830, 0.957),
                last_dark: Rgba::rgb(0.380, 0.580, 0.780),
                legal_light: Rgba::rgb(0.750, 0.870, 0.920),
                legal_dark: Rgba::rgb(0.450, 0.620, 0.780),
            },
            BoardTheme::Royal => ThemeColors {
                light: Rgba::rgb(0.910, 0.855, 0.965),
                dark: Rgba::rgb(0.608, 0.447, 0.812),
                selected: Rgba::rgb(0.700, 0.500, 0.850),
                last_light: Rgba::rgb(0.870, 0.780, 0.960),
                last_dark: Rgba::rgb(0.650, 0.480, 0.830),
                legal_light: Rgba::rgb(0.860, 0.820, 0.940),
                legal_dark: Rgba::rgb(0.630, 0.520, 0.800),
            },
            BoardTheme::Walnut => ThemeColors {
                light: Rgba::rgb(0.941, 0.824, 0.706),
                dark: Rgba::rgb(0.627, 0.408, 0.251),
                selected: Rgba::rgb(0.510, 0.443, 0.322),
                last_light: Rgba::rgb(0.957, 0.890, 0.710),
                last_dark: Rgba::rgb(0.690, 0.530, 0.350),
                legal_light: Rgba::rgb(0.880, 0.830, 0.650),
                legal_dark: Rgba::rgb(0.600, 0.500, 0.350),
            },
            BoardTheme::Midnight => ThemeColors {
                light: Rgba::rgb(0.780, 0.800, 0.840),
                dark: Rgba::rgb(0.290, 0.330, 0.440),
                selected: Rgba::rgb(0.400, 0.500, 0.700),
                last_light: Rgba::rgb(0.650, 0.720, 0.880),
                last_dark: Rgba::rgb(0.350, 0.420, 0.580),
                legal_light: Rgba::rgb(0.700, 0.760, 0.860),
                legal_dark: Rgba::rgb(0.380, 0.440, 0.580),
            },
            BoardTheme::Forest => ThemeColors {
                light: Rgba::rgb(0.878, 0.910, 0.859),
                dark: Rgba::rgb(0.337, 0.463, 0.325),
                selected: Rgba::rgb(0.500, 0.650, 0.350),
                last_light: Rgba::rgb(0.820, 0.900, 0.700),
                last_dark: Rgba::rgb(0.420, 0.560, 0.380),
                legal_light: Rgba::rgb(0.800, 0.870, 0.700),
                legal_dark: Rgba::rgb(0.400, 0.530, 0.380),
            },
            BoardTheme::Sakura => ThemeColors {
                light: Rgba::rgb(0.965, 0.882, 0.910),
                dark: Rgba::rgb(0.750, 0.450, 0.550),
                selected: Rgba::rgb(0.850, 0.400, 0.550),
                last_light: Rgba::rgb(0.950, 0.800, 0.860),
                last_dark: Rgba::rgb(0.780, 0.500, 0.600),
                legal_light: Rgba::rgb(0.930, 0.820, 0.870),
                legal_dark: Rgba::rgb(0.720, 0.480, 0.570),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_opaque_palette() {
        for theme in BoardTheme::ALL {
            let pal = theme.gui_palette();
            assert!((pal.bg.a - 1.0).abs() < f32::EPSILON);
            let colors = theme.colors();
            assert!(colors.light.r > colors.dark.r || colors.light.g > colors.dark.g);
        }
    }

    #[test]
    fn rgba_roundtrip_u8() {
        let c = Rgba::rgb(1.0, 0.0, 0.5);
        assert_eq!(c.to_u8()[0], 255);
        assert_eq!(c.to_u8()[1], 0);
    }
}
