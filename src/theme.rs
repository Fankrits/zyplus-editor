//! Colours for the native app: the ten palettes the web build offers, each with
//! the light/dark base the system setting resolves to.

use gpui::{rgb, Rgba, WindowAppearance};

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub bg: Rgba,
    pub surface: Rgba,
    pub fg: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub sidebar_bg: Rgba,
    pub hover: Rgba,
    pub accent: Rgba,
    pub accent_soft: Rgba,
    pub selection: Rgba,
    pub heading: Rgba,
    pub code: Rgba,
    pub quote: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub cursor: Rgba,
    pub syntax: Syntax,
}

/// Token colours for code highlighting. Punctuation and plain text use `muted`
/// and `fg` from [`Theme`].
#[derive(Clone, Copy, Debug)]
pub struct Syntax {
    pub keyword: Rgba,
    pub string: Rgba,
    pub number: Rgba,
    pub comment: Rgba,
    pub function: Rgba,
    pub ty: Rgba,
    pub constant: Rgba,
    pub property: Rgba,
    pub tag: Rgba,
    pub attribute: Rgba,
    pub operator: Rgba,
}

impl Syntax {
    /// The syntax colours for a palette id; unknown ids use the light or dark base.
    fn for_palette(name: &str, dark: bool) -> Self {
        match name {
            "latte" => Self {
                keyword: rgb(0x8839ef),
                string: rgb(0x40a02b),
                number: rgb(0xfe640b),
                comment: rgb(0x8c8fa1),
                function: rgb(0x1e66f5),
                ty: rgb(0xdf8e1d),
                constant: rgb(0xfe640b),
                property: rgb(0x209fb5),
                tag: rgb(0xd20f39),
                attribute: rgb(0xdf8e1d),
                operator: rgb(0x179299),
            },
            "frappe" => Self {
                keyword: rgb(0xca9ee6),
                string: rgb(0xa6d189),
                number: rgb(0xef9f76),
                comment: rgb(0x838ba7),
                function: rgb(0x8caaee),
                ty: rgb(0xe5c890),
                constant: rgb(0xef9f76),
                property: rgb(0x85c1dc),
                tag: rgb(0xe78284),
                attribute: rgb(0xe5c890),
                operator: rgb(0x81c8be),
            },
            "macchiato" => Self {
                keyword: rgb(0xc6a0f6),
                string: rgb(0xa6da95),
                number: rgb(0xf5a97f),
                comment: rgb(0x8087a2),
                function: rgb(0x8aadf4),
                ty: rgb(0xeed49f),
                constant: rgb(0xf5a97f),
                property: rgb(0x7dc4e4),
                tag: rgb(0xed8796),
                attribute: rgb(0xeed49f),
                operator: rgb(0x8bd5ca),
            },
            "mocha" => Self {
                keyword: rgb(0xcba6f7),
                string: rgb(0xa6e3a1),
                number: rgb(0xfab387),
                comment: rgb(0x7f849c),
                function: rgb(0x89b4fa),
                ty: rgb(0xf9e2af),
                constant: rgb(0xfab387),
                property: rgb(0x89dceb),
                tag: rgb(0xf38ba8),
                attribute: rgb(0xf9e2af),
                operator: rgb(0x94e2d5),
            },
            "tokyo-night" | "tokyo-storm" => Self {
                keyword: rgb(0xbb9af7),
                string: rgb(0x9ece6a),
                number: rgb(0xff9e64),
                comment: rgb(0x565f89),
                function: rgb(0x7aa2f7),
                ty: rgb(0x2ac3de),
                constant: rgb(0xff9e64),
                property: rgb(0x7dcfff),
                tag: rgb(0xf7768e),
                attribute: rgb(0xe0af68),
                operator: rgb(0x89ddff),
            },
            "tokyo-day" => Self {
                keyword: rgb(0x9854f1),
                string: rgb(0x587539),
                number: rgb(0xb15c00),
                comment: rgb(0x848cb5),
                function: rgb(0x2e7de9),
                ty: rgb(0x007197),
                constant: rgb(0xb15c00),
                property: rgb(0x118c74),
                tag: rgb(0xf52a65),
                attribute: rgb(0x8c6c3e),
                operator: rgb(0x007197),
            },
            _ if dark => Self {
                keyword: rgb(0xc792ea),
                string: rgb(0x8fd3a8),
                number: rgb(0xf5b971),
                comment: rgb(0x7d7d78),
                function: rgb(0x6ba1ff),
                ty: rgb(0x6fd3d0),
                constant: rgb(0xf5b971),
                property: rgb(0x9cc8f0),
                tag: rgb(0xff6b5e),
                attribute: rgb(0xe6c07b),
                operator: rgb(0xb8b8b2),
            },
            _ => Self {
                keyword: rgb(0x9b2fae),
                string: rgb(0x2f7d5b),
                number: rgb(0xb35900),
                comment: rgb(0x8a8a84),
                function: rgb(0x2459c4),
                ty: rgb(0x0b6e7a),
                constant: rgb(0xb35900),
                property: rgb(0x1f5f8b),
                tag: rgb(0xc4342b),
                attribute: rgb(0xb35900),
                operator: rgb(0x4a4a46),
            },
        }
    }
}

/// One card of the appearance picker. `swatch` is `[background, surface, accent]`
/// from the `THEMES` table in `src/lib/theme.ts`, as `0xRRGGBB`; the picker paints
/// the miniature window from it.
pub struct ThemeOption {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub swatch: [u32; 3],
}

/// Picker order and grouping, same as the web `THEMES` table.
pub const THEMES: [ThemeOption; 10] = [
    ThemeOption {
        id: "system",
        label: "System",
        group: "Base",
        swatch: [0xffffff, 0x000000, 0x006fee],
    },
    ThemeOption {
        id: "light",
        label: "Light",
        group: "Base",
        swatch: [0xffffff, 0xf1f1f4, 0x006fee],
    },
    ThemeOption {
        id: "dark",
        label: "Dark",
        group: "Base",
        swatch: [0x18181b, 0x3f3f46, 0x63b3ff],
    },
    ThemeOption {
        id: "latte",
        label: "Latte",
        group: "Catppuccin",
        swatch: [0xeff1f5, 0xccd0da, 0x1e66f5],
    },
    ThemeOption {
        id: "frappe",
        label: "Frappé",
        group: "Catppuccin",
        swatch: [0x303446, 0x414559, 0x8caaee],
    },
    ThemeOption {
        id: "macchiato",
        label: "Macchiato",
        group: "Catppuccin",
        swatch: [0x24273a, 0x363a4f, 0x8aadf4],
    },
    ThemeOption {
        id: "mocha",
        label: "Mocha",
        group: "Catppuccin",
        swatch: [0x1e1e2e, 0x313244, 0x89b4fa],
    },
    ThemeOption {
        id: "tokyo-night",
        label: "Night",
        group: "Tokyo Night",
        swatch: [0x1a1b26, 0x292e42, 0x7aa2f7],
    },
    ThemeOption {
        id: "tokyo-storm",
        label: "Storm",
        group: "Tokyo Night",
        swatch: [0x24283b, 0x2f334d, 0x7aa2f7],
    },
    ThemeOption {
        id: "tokyo-day",
        label: "Day",
        group: "Tokyo Night",
        swatch: [0xe1e2e7, 0xc4c8da, 0x2e7de9],
    },
];

impl Theme {
    /// The palette for a stored theme id; unknown ids fall back to the system.
    pub fn for_name(name: &str, window: WindowAppearance) -> Self {
        let system_dark = matches!(
            window,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        match name {
            "system" | "" => {
                if system_dark {
                    Self::dark()
                } else {
                    Self::light()
                }
            }
            "light" => Self::light(),
            "dark" => Self::dark(),
            "latte" => Self::palette(
                "latte",
                false,
                rgb(0xeff1f5),
                rgb(0xe6e9ef),
                rgb(0x4c4f69),
                rgb(0x6c6f85),
                rgb(0xbcc0cc),
                rgb(0xccd0da),
                rgb(0x1e66f5),
                rgb(0xd20f39),
                rgb(0x40a02b),
            ),
            "frappe" => Self::palette(
                "frappe",
                true,
                rgb(0x303446),
                rgb(0x292c3c),
                rgb(0xc6d0f5),
                rgb(0xa5adce),
                rgb(0x414559),
                rgb(0x414559),
                rgb(0x8caaee),
                rgb(0xe78284),
                rgb(0xa6d189),
            ),
            "macchiato" => Self::palette(
                "macchiato",
                true,
                rgb(0x24273a),
                rgb(0x1e2030),
                rgb(0xcad3f5),
                rgb(0xa5adcb),
                rgb(0x363a4f),
                rgb(0x363a4f),
                rgb(0x8aadf4),
                rgb(0xed8796),
                rgb(0xa6da95),
            ),
            "mocha" => Self::palette(
                "mocha",
                true,
                rgb(0x1e1e2e),
                rgb(0x181825),
                rgb(0xcdd6f4),
                rgb(0xa6adc8),
                rgb(0x313244),
                rgb(0x313244),
                rgb(0x89b4fa),
                rgb(0xf38ba8),
                rgb(0xa6e3a1),
            ),
            "tokyo-night" => Self::palette(
                "tokyo-night",
                true,
                rgb(0x1a1b26),
                rgb(0x16161e),
                rgb(0xc0caf5),
                rgb(0x787c99),
                rgb(0x292e42),
                rgb(0x292e42),
                rgb(0x7aa2f7),
                rgb(0xf7768e),
                rgb(0x9ece6a),
            ),
            "tokyo-storm" => Self::palette(
                "tokyo-storm",
                true,
                rgb(0x24283b),
                rgb(0x1f2335),
                rgb(0xc0caf5),
                rgb(0x8b91b4),
                rgb(0x2f334d),
                rgb(0x2f334d),
                rgb(0x7aa2f7),
                rgb(0xf7768e),
                rgb(0x9ece6a),
            ),
            "tokyo-day" => Self::palette(
                "tokyo-day",
                false,
                rgb(0xe1e2e7),
                rgb(0xd5d6db),
                rgb(0x3760bf),
                rgb(0x6172b0),
                rgb(0xb6bac9),
                rgb(0xc4c8da),
                rgb(0x2e7de9),
                rgb(0xf52a65),
                rgb(0x587539),
            ),
            _ => {
                if system_dark {
                    Self::dark()
                } else {
                    Self::light()
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn palette(
        name: &str,
        dark: bool,
        bg: Rgba,
        surface_secondary: Rgba,
        fg: Rgba,
        muted: Rgba,
        border: Rgba,
        hover: Rgba,
        accent: Rgba,
        danger: Rgba,
        code: Rgba,
    ) -> Self {
        Self {
            bg,
            surface: bg,
            fg,
            muted,
            border,
            sidebar_bg: surface_secondary,
            hover,
            accent,
            accent_soft: tint(accent, 0.16),
            selection: tint(accent, 0.22),
            heading: fg,
            code,
            quote: muted,
            danger,
            warning: if dark { rgb(0xe0af68) } else { rgb(0xdf8e1d) },
            cursor: fg,
            syntax: Syntax::for_palette(name, dark),
        }
    }

    /// The base light palette, used by "System" on a light desktop and "Light".
    ///
    /// The values are what the web build's HeroUI default theme resolves to
    /// (`node_modules/@heroui/styles/dist/themes/default/variables.css`, with
    /// `App.css` making the light canvas white). The HeroUI tokens are oklch;
    /// they were converted to sRGB with CSS gamut mapping. `accent` is
    /// `oklch(0.6204 0.195 253.83)`, which renders as #0485f7 (the swatch table
    /// uses #006fee; that is a rounder approximation, not the token).
    /// `accent_soft` and `selection` are the accent at HeroUI's 15% alpha.
    pub fn light() -> Self {
        let accent = rgb(0x0485f7);
        Self {
            bg: rgb(0xffffff),
            surface: rgb(0xffffff),
            fg: rgb(0x18181b),
            muted: rgb(0x71717a),
            border: rgb(0xdedee0),
            sidebar_bg: rgb(0xefeff0),
            hover: rgb(0xeaeaea),
            accent,
            accent_soft: tint(accent, 0.15),
            selection: tint(accent, 0.15),
            heading: rgb(0x0f0f0e),
            code: rgb(0x2f7d5b),
            quote: rgb(0x77776f),
            danger: rgb(0xff383c),
            warning: rgb(0xf5a524),
            cursor: rgb(0x18181b),
            syntax: Syntax::for_palette("light", false),
        }
    }

    /// The base dark palette: HeroUI's `.dark` tokens. Its canvas is almost black
    /// (`oklch(12% ...)`), and each step up (surface, sidebar) is lighter.
    /// `accent` is the same token as in light, and `accent_soft` is at 12% alpha.
    pub fn dark() -> Self {
        let accent = rgb(0x0485f7);
        Self {
            bg: rgb(0x060607),
            surface: rgb(0x18181b),
            fg: rgb(0xfcfcfc),
            muted: rgb(0x9f9fa9),
            border: rgb(0x28282c),
            sidebar_bg: rgb(0x232325),
            hover: rgb(0x27272a),
            accent,
            accent_soft: tint(accent, 0.12),
            selection: tint(accent, 0.12),
            heading: rgb(0xffffff),
            code: rgb(0x8fd3a8),
            quote: rgb(0x9a9a94),
            danger: rgb(0xdb3b3e),
            warning: rgb(0xf7b750),
            cursor: rgb(0xfcfcfc),
            syntax: Syntax::for_palette("dark", true),
        }
    }
}

fn tint(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(color: Rgba) -> u32 {
        let channel = |c: f32| (c * 255.0).round() as u32;
        (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
    }

    #[test]
    fn base_palettes_match_the_web_tokens() {
        let light = Theme::light();
        assert_eq!(hex(light.bg), 0xffffff);
        assert_eq!(hex(light.surface), 0xffffff);
        assert_eq!(hex(light.sidebar_bg), 0xefeff0);
        assert_eq!(hex(light.fg), 0x18181b);
        assert_eq!(hex(light.muted), 0x71717a);
        assert_eq!(hex(light.border), 0xdedee0);
        assert_eq!(hex(light.hover), 0xeaeaea);
        assert_eq!(hex(light.accent), 0x0485f7);
        assert_eq!(hex(light.danger), 0xff383c);
        assert!((light.accent_soft.a - 0.15).abs() < 1e-3);

        let dark = Theme::dark();
        assert_eq!(hex(dark.bg), 0x060607);
        assert_eq!(hex(dark.surface), 0x18181b);
        assert_eq!(hex(dark.sidebar_bg), 0x232325);
        assert_eq!(hex(dark.fg), 0xfcfcfc);
        assert_eq!(hex(dark.muted), 0x9f9fa9);
        assert_eq!(hex(dark.border), 0x28282c);
        assert_eq!(hex(dark.hover), 0x27272a);
        assert_eq!(hex(dark.accent), 0x0485f7);
        assert_eq!(hex(dark.danger), 0xdb3b3e);
        assert!((dark.accent_soft.a - 0.12).abs() < 1e-3);
    }

    #[test]
    fn system_follows_the_window_appearance() {
        let light = Theme::for_name("system", WindowAppearance::Light);
        assert_eq!(hex(light.bg), 0xffffff);
        let dark = Theme::for_name("system", WindowAppearance::VibrantDark);
        assert_eq!(hex(dark.bg), 0x060607);
        // An unknown stored id falls back to the same resolution.
        let fallback = Theme::for_name("no-such-theme", WindowAppearance::Dark);
        assert_eq!(hex(fallback.bg), 0x060607);
    }

    #[test]
    fn picker_lists_the_web_themes_in_order_with_their_swatches() {
        let ids: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            [
                "system",
                "light",
                "dark",
                "latte",
                "frappe",
                "macchiato",
                "mocha",
                "tokyo-night",
                "tokyo-storm",
                "tokyo-day",
            ]
        );
        let groups: Vec<&str> = THEMES.iter().map(|t| t.group).collect();
        assert_eq!(groups[0], "Base");
        assert_eq!(groups[3], "Catppuccin");
        assert_eq!(groups[7], "Tokyo Night");
        // Spot-check against src/lib/theme.ts: [background, surface, accent].
        assert_eq!(THEMES[2].swatch, [0x18181b, 0x3f3f46, 0x63b3ff]);
        assert_eq!(THEMES[9].swatch, [0xe1e2e7, 0xc4c8da, 0x2e7de9]);
    }
}
