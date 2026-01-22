use once_cell::sync::OnceCell;
use ratatui::prelude::Color;
use std::str::FromStr;

static THEME: OnceCell<Theme> = OnceCell::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreset {
    DefaultDark,
    DefaultLight,
    CatppuccinMocha,
    CatppuccinLatte,
    Dracula,
    Nord,
    GruvboxDark,
    GruvboxLight,
    OneDark,
    SolarizedDark,
    SolarizedLight,
}

impl FromStr for ThemePreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('_', "-").as_str() {
            "default-dark" | "dark" => Ok(Self::DefaultDark),
            "default-light" | "light" => Ok(Self::DefaultLight),
            "catppuccin-mocha" | "mocha" => Ok(Self::CatppuccinMocha),
            "catppuccin-latte" | "latte" => Ok(Self::CatppuccinLatte),
            "dracula" => Ok(Self::Dracula),
            "nord" => Ok(Self::Nord),
            "gruvbox-dark" => Ok(Self::GruvboxDark),
            "gruvbox-light" => Ok(Self::GruvboxLight),
            "one-dark" | "onedark" => Ok(Self::OneDark),
            "solarized-dark" => Ok(Self::SolarizedDark),
            "solarized-light" => Ok(Self::SolarizedLight),
            _ => Err(format!(
                "Unknown theme '{}'. Valid: default-dark, default-light, catppuccin-mocha, catppuccin-latte, dracula, nord, gruvbox-dark, gruvbox-light, one-dark, solarized-dark, solarized-light",
                s
            )),
        }
    }
}

impl ThemeMode {
    pub fn detect() -> Self {
        match dark_light::detect() {
            dark_light::Mode::Dark => ThemeMode::Dark,
            dark_light::Mode::Light => ThemeMode::Light,
            dark_light::Mode::Default => ThemeMode::Dark,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyntaxColors {
    pub comment: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub function: Color,
    pub function_macro: Color,
    pub r#type: Color,
    pub variable_builtin: Color,
    pub variable_member: Color,
    pub module: Color,
    pub operator: Color,
    pub tag: Color,
    pub attribute: Color,
    pub label: Color,
    pub punctuation: Color,
    pub default_text: Color,
}

#[derive(Debug, Clone)]
pub struct DiffColors {
    pub added_bg: Color,
    pub added_gutter_bg: Color,
    pub added_gutter_fg: Color,
    pub deleted_bg: Color,
    pub deleted_gutter_bg: Color,
    pub deleted_gutter_fg: Color,
    pub context_bg: Color,
    pub empty_placeholder_fg: Color,
    /// Word-level highlight for added text (darker green)
    pub added_word_bg: Color,
    /// Word-level highlight for deleted text (darker red)
    pub deleted_word_bg: Color,
}

#[derive(Debug, Clone)]
pub struct UiColors {
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub line_number: Color,
    pub bg: Color,
    pub footer_branch_bg: Color,
    pub footer_branch_fg: Color,
    pub status_added: Color,
    pub status_modified: Color,
    pub status_deleted: Color,
    pub stats_added: Color,
    pub stats_removed: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub highlight: Color,
    pub viewed: Color,
    pub watching: Color,
    pub search_match_bg: Color,
    pub search_match_fg: Color,
    pub search_current_bg: Color,
    pub search_current_fg: Color,
}

#[derive(Debug, Clone)]
pub struct Theme {
    #[allow(dead_code)]
    pub mode: ThemeMode,
    pub syntax: SyntaxColors,
    pub diff: DiffColors,
    pub ui: UiColors,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(106, 115, 125),
                keyword: Color::Rgb(255, 123, 114),
                string: Color::Rgb(165, 214, 255),
                number: Color::Rgb(121, 192, 255),
                function: Color::Rgb(210, 168, 255),
                function_macro: Color::Rgb(86, 182, 194),
                r#type: Color::Rgb(255, 203, 107),
                variable_builtin: Color::Rgb(255, 123, 114),
                variable_member: Color::Rgb(121, 192, 255),
                module: Color::Rgb(230, 192, 123),
                operator: Color::Rgb(255, 123, 114),
                tag: Color::Rgb(126, 231, 135),
                attribute: Color::Rgb(121, 192, 255),
                label: Color::Rgb(255, 160, 122),
                punctuation: Color::Rgb(200, 200, 200),
                default_text: Color::Rgb(230, 230, 230),
            },
            diff: DiffColors {
                added_bg: Color::Rgb(35, 50, 40),
                added_gutter_bg: Color::Rgb(40, 80, 50),
                added_gutter_fg: Color::Rgb(140, 200, 160),
                deleted_bg: Color::Rgb(50, 35, 35),
                deleted_gutter_bg: Color::Rgb(80, 40, 40),
                deleted_gutter_fg: Color::Rgb(200, 140, 140),
                context_bg: Color::Rgb(40, 40, 50),
                empty_placeholder_fg: Color::Rgb(55, 60, 70),
                added_word_bg: Color::Rgb(40, 85, 55),
                deleted_word_bg: Color::Rgb(100, 50, 50),
            },
            ui: UiColors {
                border_focused: Color::Cyan,
                border_unfocused: Color::DarkGray,
                text_primary: Color::Rgb(230, 230, 230),
                text_secondary: Color::Rgb(200, 200, 200),
                text_muted: Color::Rgb(140, 140, 160),
                line_number: Color::DarkGray,
                bg: Color::Reset,
                footer_branch_bg: Color::Rgb(50, 50, 70),
                footer_branch_fg: Color::Rgb(180, 180, 220),
                status_added: Color::Green,
                status_modified: Color::Yellow,
                status_deleted: Color::Red,
                stats_added: Color::Rgb(80, 200, 120),
                stats_removed: Color::Rgb(240, 80, 80),
                selection_bg: Color::Cyan,
                selection_fg: Color::Black,
                highlight: Color::Yellow,
                viewed: Color::Green,
                watching: Color::Yellow,
                search_match_bg: Color::Rgb(100, 80, 20),
                search_match_fg: Color::Rgb(255, 220, 120),
                search_current_bg: Color::Rgb(255, 165, 0),
                search_current_fg: Color::Black,
            },
        }
    }

    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            syntax: SyntaxColors {
                comment: Color::Rgb(106, 115, 125),
                keyword: Color::Rgb(207, 34, 46),
                string: Color::Rgb(10, 48, 105),
                number: Color::Rgb(5, 80, 174),
                function: Color::Rgb(130, 80, 223),
                function_macro: Color::Rgb(17, 99, 41),
                r#type: Color::Rgb(149, 56, 0),
                variable_builtin: Color::Rgb(207, 34, 46),
                variable_member: Color::Rgb(5, 80, 174),
                module: Color::Rgb(149, 56, 0),
                operator: Color::Rgb(207, 34, 46),
                tag: Color::Rgb(17, 99, 41),
                attribute: Color::Rgb(5, 80, 174),
                label: Color::Rgb(191, 87, 0),
                punctuation: Color::Rgb(87, 96, 106),
                default_text: Color::Rgb(36, 41, 47),
            },
            diff: DiffColors {
                added_bg: Color::Rgb(230, 255, 237),
                added_gutter_bg: Color::Rgb(180, 240, 200),
                added_gutter_fg: Color::Rgb(36, 100, 60),
                deleted_bg: Color::Rgb(255, 245, 243),
                deleted_gutter_bg: Color::Rgb(255, 210, 205),
                deleted_gutter_fg: Color::Rgb(140, 60, 60),
                context_bg: Color::Rgb(246, 248, 250),
                empty_placeholder_fg: Color::Rgb(200, 205, 212),
                added_word_bg: Color::Rgb(171, 242, 188),
                deleted_word_bg: Color::Rgb(255, 184, 174),
            },
            ui: UiColors {
                border_focused: Color::Rgb(9, 105, 218),
                border_unfocused: Color::Rgb(208, 215, 222),
                text_primary: Color::Rgb(36, 41, 47),
                text_secondary: Color::Rgb(87, 96, 106),
                text_muted: Color::Rgb(140, 149, 159),
                line_number: Color::Rgb(140, 149, 159),
                bg: Color::Reset,
                footer_branch_bg: Color::Rgb(221, 244, 255),
                footer_branch_fg: Color::Rgb(9, 105, 218),
                status_added: Color::Rgb(26, 127, 55),
                status_modified: Color::Rgb(154, 103, 0),
                status_deleted: Color::Rgb(207, 34, 46),
                stats_added: Color::Rgb(26, 127, 55),
                stats_removed: Color::Rgb(207, 34, 46),
                selection_bg: Color::Rgb(9, 105, 218),
                selection_fg: Color::White,
                highlight: Color::Rgb(154, 103, 0),
                viewed: Color::Rgb(26, 127, 55),
                watching: Color::Rgb(154, 103, 0),
                search_match_bg: Color::Rgb(255, 235, 150),
                search_match_fg: Color::Black,
                search_current_bg: Color::Rgb(255, 140, 0),
                search_current_fg: Color::Black,
            },
        }
    }

    pub fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    pub fn from_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::DefaultDark => Self::dark(),
            ThemePreset::DefaultLight => Self::light(),
            ThemePreset::CatppuccinMocha => Self::catppuccin_mocha(),
            ThemePreset::CatppuccinLatte => Self::catppuccin_latte(),
            ThemePreset::Dracula => Self::dracula(),
            ThemePreset::Nord => Self::nord(),
            ThemePreset::GruvboxDark => Self::gruvbox_dark(),
            ThemePreset::GruvboxLight => Self::gruvbox_light(),
            ThemePreset::OneDark => Self::one_dark(),
            ThemePreset::SolarizedDark => Self::solarized_dark(),
            ThemePreset::SolarizedLight => Self::solarized_light(),
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(108, 112, 134),          // overlay0
                keyword: Color::Rgb(203, 166, 247),          // mauve
                string: Color::Rgb(166, 227, 161),           // green
                number: Color::Rgb(250, 179, 135),           // peach
                function: Color::Rgb(137, 180, 250),         // blue
                function_macro: Color::Rgb(148, 226, 213),   // teal
                r#type: Color::Rgb(249, 226, 175),           // yellow
                variable_builtin: Color::Rgb(243, 139, 168), // red
                variable_member: Color::Rgb(116, 199, 236),  // sapphire
                module: Color::Rgb(250, 179, 135),           // peach
                operator: Color::Rgb(137, 220, 235),         // sky
                tag: Color::Rgb(166, 227, 161),              // green
                attribute: Color::Rgb(137, 180, 250),        // blue
                label: Color::Rgb(245, 194, 231),            // pink
                punctuation: Color::Rgb(166, 173, 200),      // subtext0
                default_text: Color::Rgb(205, 214, 244),     // text
            },
            diff: DiffColors {
                added_bg: Color::Rgb(30, 40, 35),
                added_gutter_bg: Color::Rgb(40, 70, 50),
                added_gutter_fg: Color::Rgb(166, 227, 161), // green
                deleted_bg: Color::Rgb(45, 30, 35),
                deleted_gutter_bg: Color::Rgb(70, 40, 50),
                deleted_gutter_fg: Color::Rgb(243, 139, 168), // red
                context_bg: Color::Rgb(30, 30, 46),           // base
                empty_placeholder_fg: Color::Rgb(69, 71, 90), // surface1
                added_word_bg: Color::Rgb(50, 90, 60),
                deleted_word_bg: Color::Rgb(100, 50, 60),
            },
            ui: UiColors {
                border_focused: Color::Rgb(137, 180, 250),   // blue
                border_unfocused: Color::Rgb(69, 71, 90),    // surface1
                text_primary: Color::Rgb(205, 214, 244),     // text
                text_secondary: Color::Rgb(166, 173, 200),   // subtext0
                text_muted: Color::Rgb(108, 112, 134),       // overlay0
                line_number: Color::Rgb(88, 91, 112),        // overlay0
                bg: Color::Rgb(24, 24, 37),                  // mantle
                footer_branch_bg: Color::Rgb(49, 50, 68),    // surface0
                footer_branch_fg: Color::Rgb(137, 180, 250), // blue
                status_added: Color::Rgb(166, 227, 161),     // green
                status_modified: Color::Rgb(249, 226, 175),  // yellow
                status_deleted: Color::Rgb(243, 139, 168),   // red
                stats_added: Color::Rgb(166, 227, 161),
                stats_removed: Color::Rgb(243, 139, 168),
                selection_bg: Color::Rgb(137, 180, 250),
                selection_fg: Color::Rgb(30, 30, 46),
                highlight: Color::Rgb(249, 226, 175),
                viewed: Color::Rgb(166, 227, 161),
                watching: Color::Rgb(249, 226, 175),
                search_match_bg: Color::Rgb(249, 226, 175),
                search_match_fg: Color::Rgb(30, 30, 46),
                search_current_bg: Color::Rgb(250, 179, 135),
                search_current_fg: Color::Rgb(30, 30, 46),
            },
        }
    }

    pub fn catppuccin_latte() -> Self {
        Self {
            mode: ThemeMode::Light,
            syntax: SyntaxColors {
                comment: Color::Rgb(140, 143, 161),        // overlay0
                keyword: Color::Rgb(136, 57, 239),         // mauve
                string: Color::Rgb(64, 160, 43),           // green
                number: Color::Rgb(254, 100, 11),          // peach
                function: Color::Rgb(30, 102, 245),        // blue
                function_macro: Color::Rgb(23, 146, 153),  // teal
                r#type: Color::Rgb(223, 142, 29),          // yellow
                variable_builtin: Color::Rgb(210, 15, 57), // red
                variable_member: Color::Rgb(32, 159, 181), // sapphire
                module: Color::Rgb(254, 100, 11),          // peach
                operator: Color::Rgb(4, 165, 229),         // sky
                tag: Color::Rgb(64, 160, 43),              // green
                attribute: Color::Rgb(30, 102, 245),       // blue
                label: Color::Rgb(234, 118, 203),          // pink
                punctuation: Color::Rgb(92, 95, 119),      // subtext0
                default_text: Color::Rgb(76, 79, 105),     // text
            },
            diff: DiffColors {
                added_bg: Color::Rgb(230, 250, 235),
                added_gutter_bg: Color::Rgb(190, 235, 200),
                added_gutter_fg: Color::Rgb(64, 160, 43),
                deleted_bg: Color::Rgb(255, 235, 235),
                deleted_gutter_bg: Color::Rgb(250, 200, 200),
                deleted_gutter_fg: Color::Rgb(210, 15, 57),
                context_bg: Color::Rgb(239, 241, 245), // base
                empty_placeholder_fg: Color::Rgb(188, 192, 204), // surface1
                added_word_bg: Color::Rgb(160, 230, 180),
                deleted_word_bg: Color::Rgb(255, 180, 180),
            },
            ui: UiColors {
                border_focused: Color::Rgb(30, 102, 245),
                border_unfocused: Color::Rgb(188, 192, 204),
                text_primary: Color::Rgb(76, 79, 105),
                text_secondary: Color::Rgb(92, 95, 119),
                text_muted: Color::Rgb(140, 143, 161),
                line_number: Color::Rgb(140, 143, 161),
                bg: Color::Rgb(230, 233, 239),
                footer_branch_bg: Color::Rgb(204, 208, 218),
                footer_branch_fg: Color::Rgb(30, 102, 245),
                status_added: Color::Rgb(64, 160, 43),
                status_modified: Color::Rgb(223, 142, 29),
                status_deleted: Color::Rgb(210, 15, 57),
                stats_added: Color::Rgb(64, 160, 43),
                stats_removed: Color::Rgb(210, 15, 57),
                selection_bg: Color::Rgb(30, 102, 245),
                selection_fg: Color::White,
                highlight: Color::Rgb(223, 142, 29),
                viewed: Color::Rgb(64, 160, 43),
                watching: Color::Rgb(223, 142, 29),
                search_match_bg: Color::Rgb(223, 142, 29),
                search_match_fg: Color::White,
                search_current_bg: Color::Rgb(254, 100, 11),
                search_current_fg: Color::White,
            },
        }
    }

    pub fn dracula() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(98, 114, 164),           // comment
                keyword: Color::Rgb(255, 121, 198),          // pink
                string: Color::Rgb(241, 250, 140),           // yellow
                number: Color::Rgb(189, 147, 249),           // purple
                function: Color::Rgb(80, 250, 123),          // green
                function_macro: Color::Rgb(139, 233, 253),   // cyan
                r#type: Color::Rgb(139, 233, 253),           // cyan
                variable_builtin: Color::Rgb(255, 184, 108), // orange
                variable_member: Color::Rgb(248, 248, 242),  // foreground
                module: Color::Rgb(255, 184, 108),           // orange
                operator: Color::Rgb(255, 121, 198),         // pink
                tag: Color::Rgb(255, 121, 198),              // pink
                attribute: Color::Rgb(80, 250, 123),         // green
                label: Color::Rgb(255, 184, 108),            // orange
                punctuation: Color::Rgb(248, 248, 242),      // foreground
                default_text: Color::Rgb(248, 248, 242),     // foreground
            },
            diff: DiffColors {
                added_bg: Color::Rgb(35, 50, 40),
                added_gutter_bg: Color::Rgb(40, 80, 50),
                added_gutter_fg: Color::Rgb(80, 250, 123),
                deleted_bg: Color::Rgb(50, 35, 40),
                deleted_gutter_bg: Color::Rgb(80, 40, 50),
                deleted_gutter_fg: Color::Rgb(255, 85, 85),
                context_bg: Color::Rgb(40, 42, 54), // background
                empty_placeholder_fg: Color::Rgb(68, 71, 90), // current line
                added_word_bg: Color::Rgb(50, 100, 60),
                deleted_word_bg: Color::Rgb(100, 50, 60),
            },
            ui: UiColors {
                border_focused: Color::Rgb(189, 147, 249), // purple
                border_unfocused: Color::Rgb(68, 71, 90),
                text_primary: Color::Rgb(248, 248, 242),
                text_secondary: Color::Rgb(189, 147, 249),
                text_muted: Color::Rgb(98, 114, 164),
                line_number: Color::Rgb(98, 114, 164),
                bg: Color::Rgb(33, 34, 44),
                footer_branch_bg: Color::Rgb(68, 71, 90),
                footer_branch_fg: Color::Rgb(189, 147, 249),
                status_added: Color::Rgb(80, 250, 123),
                status_modified: Color::Rgb(255, 184, 108),
                status_deleted: Color::Rgb(255, 85, 85),
                stats_added: Color::Rgb(80, 250, 123),
                stats_removed: Color::Rgb(255, 85, 85),
                selection_bg: Color::Rgb(189, 147, 249),
                selection_fg: Color::Rgb(40, 42, 54),
                highlight: Color::Rgb(241, 250, 140),
                viewed: Color::Rgb(80, 250, 123),
                watching: Color::Rgb(255, 184, 108),
                search_match_bg: Color::Rgb(241, 250, 140),
                search_match_fg: Color::Rgb(40, 42, 54),
                search_current_bg: Color::Rgb(255, 184, 108),
                search_current_fg: Color::Rgb(40, 42, 54),
            },
        }
    }

    pub fn nord() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(76, 86, 106),           // nord3
                keyword: Color::Rgb(129, 161, 193),         // nord9
                string: Color::Rgb(163, 190, 140),          // nord14
                number: Color::Rgb(180, 142, 173),          // nord15
                function: Color::Rgb(136, 192, 208),        // nord8
                function_macro: Color::Rgb(143, 188, 187),  // nord7
                r#type: Color::Rgb(235, 203, 139),          // nord13
                variable_builtin: Color::Rgb(191, 97, 106), // nord11
                variable_member: Color::Rgb(216, 222, 233), // nord4
                module: Color::Rgb(208, 135, 112),          // nord12
                operator: Color::Rgb(129, 161, 193),        // nord9
                tag: Color::Rgb(129, 161, 193),             // nord9
                attribute: Color::Rgb(136, 192, 208),       // nord8
                label: Color::Rgb(208, 135, 112),           // nord12
                punctuation: Color::Rgb(216, 222, 233),     // nord4
                default_text: Color::Rgb(236, 239, 244),    // nord6
            },
            diff: DiffColors {
                added_bg: Color::Rgb(40, 55, 50),
                added_gutter_bg: Color::Rgb(50, 75, 60),
                added_gutter_fg: Color::Rgb(163, 190, 140), // nord14
                deleted_bg: Color::Rgb(55, 45, 50),
                deleted_gutter_bg: Color::Rgb(75, 55, 60),
                deleted_gutter_fg: Color::Rgb(191, 97, 106), // nord11
                context_bg: Color::Rgb(46, 52, 64),          // nord0
                empty_placeholder_fg: Color::Rgb(59, 66, 82), // nord1
                added_word_bg: Color::Rgb(60, 100, 75),
                deleted_word_bg: Color::Rgb(110, 65, 70),
            },
            ui: UiColors {
                border_focused: Color::Rgb(136, 192, 208), // nord8
                border_unfocused: Color::Rgb(59, 66, 82),  // nord1
                text_primary: Color::Rgb(236, 239, 244),   // nord6
                text_secondary: Color::Rgb(216, 222, 233), // nord4
                text_muted: Color::Rgb(76, 86, 106),       // nord3
                line_number: Color::Rgb(76, 86, 106),
                bg: Color::Rgb(59, 66, 82),               // nord1
                footer_branch_bg: Color::Rgb(67, 76, 94), // nord2
                footer_branch_fg: Color::Rgb(136, 192, 208),
                status_added: Color::Rgb(163, 190, 140),
                status_modified: Color::Rgb(235, 203, 139),
                status_deleted: Color::Rgb(191, 97, 106),
                stats_added: Color::Rgb(163, 190, 140),
                stats_removed: Color::Rgb(191, 97, 106),
                selection_bg: Color::Rgb(136, 192, 208),
                selection_fg: Color::Rgb(46, 52, 64),
                highlight: Color::Rgb(235, 203, 139),
                viewed: Color::Rgb(163, 190, 140),
                watching: Color::Rgb(235, 203, 139),
                search_match_bg: Color::Rgb(235, 203, 139),
                search_match_fg: Color::Rgb(46, 52, 64),
                search_current_bg: Color::Rgb(208, 135, 112),
                search_current_fg: Color::Rgb(46, 52, 64),
            },
        }
    }

    pub fn gruvbox_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(146, 131, 116),         // gray
                keyword: Color::Rgb(251, 73, 52),           // red
                string: Color::Rgb(184, 187, 38),           // green
                number: Color::Rgb(211, 134, 155),          // purple
                function: Color::Rgb(184, 187, 38),         // green
                function_macro: Color::Rgb(142, 192, 124),  // aqua
                r#type: Color::Rgb(250, 189, 47),           // yellow
                variable_builtin: Color::Rgb(254, 128, 25), // orange
                variable_member: Color::Rgb(235, 219, 178), // fg
                module: Color::Rgb(254, 128, 25),           // orange
                operator: Color::Rgb(251, 73, 52),          // red
                tag: Color::Rgb(142, 192, 124),             // aqua
                attribute: Color::Rgb(131, 165, 152),       // blue
                label: Color::Rgb(254, 128, 25),            // orange
                punctuation: Color::Rgb(235, 219, 178),     // fg
                default_text: Color::Rgb(235, 219, 178),    // fg
            },
            diff: DiffColors {
                added_bg: Color::Rgb(40, 50, 35),
                added_gutter_bg: Color::Rgb(60, 80, 50),
                added_gutter_fg: Color::Rgb(184, 187, 38),
                deleted_bg: Color::Rgb(55, 35, 35),
                deleted_gutter_bg: Color::Rgb(85, 50, 50),
                deleted_gutter_fg: Color::Rgb(251, 73, 52),
                context_bg: Color::Rgb(40, 40, 40),           // bg
                empty_placeholder_fg: Color::Rgb(60, 56, 54), // bg1
                added_word_bg: Color::Rgb(70, 100, 55),
                deleted_word_bg: Color::Rgb(115, 55, 50),
            },
            ui: UiColors {
                border_focused: Color::Rgb(250, 189, 47), // yellow
                border_unfocused: Color::Rgb(80, 73, 69), // bg2
                text_primary: Color::Rgb(235, 219, 178),
                text_secondary: Color::Rgb(213, 196, 161),
                text_muted: Color::Rgb(146, 131, 116),
                line_number: Color::Rgb(124, 111, 100),
                bg: Color::Rgb(50, 48, 47),
                footer_branch_bg: Color::Rgb(80, 73, 69),
                footer_branch_fg: Color::Rgb(250, 189, 47),
                status_added: Color::Rgb(184, 187, 38),
                status_modified: Color::Rgb(250, 189, 47),
                status_deleted: Color::Rgb(251, 73, 52),
                stats_added: Color::Rgb(184, 187, 38),
                stats_removed: Color::Rgb(251, 73, 52),
                selection_bg: Color::Rgb(250, 189, 47),
                selection_fg: Color::Rgb(40, 40, 40),
                highlight: Color::Rgb(250, 189, 47),
                viewed: Color::Rgb(184, 187, 38),
                watching: Color::Rgb(250, 189, 47),
                search_match_bg: Color::Rgb(250, 189, 47),
                search_match_fg: Color::Rgb(40, 40, 40),
                search_current_bg: Color::Rgb(254, 128, 25),
                search_current_fg: Color::Rgb(40, 40, 40),
            },
        }
    }

    pub fn gruvbox_light() -> Self {
        Self {
            mode: ThemeMode::Light,
            syntax: SyntaxColors {
                comment: Color::Rgb(146, 131, 116),
                keyword: Color::Rgb(204, 36, 29),          // red
                string: Color::Rgb(152, 151, 26),          // green
                number: Color::Rgb(177, 98, 134),          // purple
                function: Color::Rgb(152, 151, 26),        // green
                function_macro: Color::Rgb(104, 157, 106), // aqua
                r#type: Color::Rgb(215, 153, 33),          // yellow
                variable_builtin: Color::Rgb(214, 93, 14), // orange
                variable_member: Color::Rgb(60, 56, 54),   // fg
                module: Color::Rgb(214, 93, 14),           // orange
                operator: Color::Rgb(204, 36, 29),         // red
                tag: Color::Rgb(104, 157, 106),            // aqua
                attribute: Color::Rgb(69, 133, 136),       // blue
                label: Color::Rgb(214, 93, 14),            // orange
                punctuation: Color::Rgb(60, 56, 54),       // fg
                default_text: Color::Rgb(60, 56, 54),      // fg
            },
            diff: DiffColors {
                added_bg: Color::Rgb(235, 250, 230),
                added_gutter_bg: Color::Rgb(200, 230, 190),
                added_gutter_fg: Color::Rgb(152, 151, 26),
                deleted_bg: Color::Rgb(255, 240, 235),
                deleted_gutter_bg: Color::Rgb(250, 210, 200),
                deleted_gutter_fg: Color::Rgb(204, 36, 29),
                context_bg: Color::Rgb(251, 241, 199), // bg
                empty_placeholder_fg: Color::Rgb(213, 196, 161), // bg2
                added_word_bg: Color::Rgb(180, 235, 165),
                deleted_word_bg: Color::Rgb(255, 195, 180),
            },
            ui: UiColors {
                border_focused: Color::Rgb(69, 133, 136), // blue
                border_unfocused: Color::Rgb(213, 196, 161),
                text_primary: Color::Rgb(60, 56, 54),
                text_secondary: Color::Rgb(80, 73, 69),
                text_muted: Color::Rgb(146, 131, 116),
                line_number: Color::Rgb(146, 131, 116),
                bg: Color::Rgb(235, 219, 178),
                footer_branch_bg: Color::Rgb(213, 196, 161),
                footer_branch_fg: Color::Rgb(69, 133, 136),
                status_added: Color::Rgb(152, 151, 26),
                status_modified: Color::Rgb(215, 153, 33),
                status_deleted: Color::Rgb(204, 36, 29),
                stats_added: Color::Rgb(152, 151, 26),
                stats_removed: Color::Rgb(204, 36, 29),
                selection_bg: Color::Rgb(69, 133, 136),
                selection_fg: Color::White,
                highlight: Color::Rgb(215, 153, 33),
                viewed: Color::Rgb(152, 151, 26),
                watching: Color::Rgb(215, 153, 33),
                search_match_bg: Color::Rgb(215, 153, 33),
                search_match_fg: Color::White,
                search_current_bg: Color::Rgb(214, 93, 14),
                search_current_fg: Color::White,
            },
        }
    }

    pub fn one_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(92, 99, 112),            // comment grey
                keyword: Color::Rgb(198, 120, 221),          // purple
                string: Color::Rgb(152, 195, 121),           // green
                number: Color::Rgb(209, 154, 102),           // orange
                function: Color::Rgb(97, 175, 239),          // blue
                function_macro: Color::Rgb(86, 182, 194),    // cyan
                r#type: Color::Rgb(229, 192, 123),           // yellow
                variable_builtin: Color::Rgb(224, 108, 117), // red
                variable_member: Color::Rgb(171, 178, 191),  // fg
                module: Color::Rgb(209, 154, 102),           // orange
                operator: Color::Rgb(198, 120, 221),         // purple
                tag: Color::Rgb(224, 108, 117),              // red
                attribute: Color::Rgb(209, 154, 102),        // orange
                label: Color::Rgb(209, 154, 102),            // orange
                punctuation: Color::Rgb(171, 178, 191),      // fg
                default_text: Color::Rgb(171, 178, 191),     // fg
            },
            diff: DiffColors {
                added_bg: Color::Rgb(35, 50, 40),
                added_gutter_bg: Color::Rgb(50, 80, 55),
                added_gutter_fg: Color::Rgb(152, 195, 121),
                deleted_bg: Color::Rgb(50, 35, 38),
                deleted_gutter_bg: Color::Rgb(80, 50, 55),
                deleted_gutter_fg: Color::Rgb(224, 108, 117),
                context_bg: Color::Rgb(40, 44, 52),           // bg
                empty_placeholder_fg: Color::Rgb(62, 68, 81), // gutter
                added_word_bg: Color::Rgb(55, 100, 65),
                deleted_word_bg: Color::Rgb(110, 55, 60),
            },
            ui: UiColors {
                border_focused: Color::Rgb(97, 175, 239), // blue
                border_unfocused: Color::Rgb(62, 68, 81),
                text_primary: Color::Rgb(171, 178, 191),
                text_secondary: Color::Rgb(152, 159, 172),
                text_muted: Color::Rgb(92, 99, 112),
                line_number: Color::Rgb(76, 82, 99),
                bg: Color::Rgb(33, 37, 43),
                footer_branch_bg: Color::Rgb(62, 68, 81),
                footer_branch_fg: Color::Rgb(97, 175, 239),
                status_added: Color::Rgb(152, 195, 121),
                status_modified: Color::Rgb(229, 192, 123),
                status_deleted: Color::Rgb(224, 108, 117),
                stats_added: Color::Rgb(152, 195, 121),
                stats_removed: Color::Rgb(224, 108, 117),
                selection_bg: Color::Rgb(97, 175, 239),
                selection_fg: Color::Rgb(40, 44, 52),
                highlight: Color::Rgb(229, 192, 123),
                viewed: Color::Rgb(152, 195, 121),
                watching: Color::Rgb(229, 192, 123),
                search_match_bg: Color::Rgb(229, 192, 123),
                search_match_fg: Color::Rgb(40, 44, 52),
                search_current_bg: Color::Rgb(209, 154, 102),
                search_current_fg: Color::Rgb(40, 44, 52),
            },
        }
    }

    pub fn solarized_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            syntax: SyntaxColors {
                comment: Color::Rgb(88, 110, 117),          // base01
                keyword: Color::Rgb(133, 153, 0),           // green
                string: Color::Rgb(42, 161, 152),           // cyan
                number: Color::Rgb(108, 113, 196),          // violet
                function: Color::Rgb(38, 139, 210),         // blue
                function_macro: Color::Rgb(211, 54, 130),   // magenta
                r#type: Color::Rgb(181, 137, 0),            // yellow
                variable_builtin: Color::Rgb(203, 75, 22),  // orange
                variable_member: Color::Rgb(147, 161, 161), // base1
                module: Color::Rgb(203, 75, 22),            // orange
                operator: Color::Rgb(133, 153, 0),          // green
                tag: Color::Rgb(38, 139, 210),              // blue
                attribute: Color::Rgb(42, 161, 152),        // cyan
                label: Color::Rgb(203, 75, 22),             // orange
                punctuation: Color::Rgb(131, 148, 150),     // base0
                default_text: Color::Rgb(131, 148, 150),    // base0
            },
            diff: DiffColors {
                added_bg: Color::Rgb(0, 50, 50),
                added_gutter_bg: Color::Rgb(0, 70, 60),
                added_gutter_fg: Color::Rgb(133, 153, 0),
                deleted_bg: Color::Rgb(50, 30, 30),
                deleted_gutter_bg: Color::Rgb(70, 40, 40),
                deleted_gutter_fg: Color::Rgb(220, 50, 47),
                context_bg: Color::Rgb(0, 43, 54), // base03
                empty_placeholder_fg: Color::Rgb(7, 54, 66), // base02
                added_word_bg: Color::Rgb(20, 85, 75),
                deleted_word_bg: Color::Rgb(100, 50, 45),
            },
            ui: UiColors {
                border_focused: Color::Rgb(38, 139, 210), // blue
                border_unfocused: Color::Rgb(88, 110, 117),
                text_primary: Color::Rgb(131, 148, 150), // base0
                text_secondary: Color::Rgb(147, 161, 161), // base1
                text_muted: Color::Rgb(88, 110, 117),    // base01
                line_number: Color::Rgb(88, 110, 117),
                bg: Color::Rgb(7, 54, 66), // base02
                footer_branch_bg: Color::Rgb(88, 110, 117),
                footer_branch_fg: Color::Rgb(38, 139, 210),
                status_added: Color::Rgb(133, 153, 0),
                status_modified: Color::Rgb(181, 137, 0),
                status_deleted: Color::Rgb(220, 50, 47),
                stats_added: Color::Rgb(133, 153, 0),
                stats_removed: Color::Rgb(220, 50, 47),
                selection_bg: Color::Rgb(38, 139, 210),
                selection_fg: Color::Rgb(0, 43, 54),
                highlight: Color::Rgb(181, 137, 0),
                viewed: Color::Rgb(133, 153, 0),
                watching: Color::Rgb(181, 137, 0),
                search_match_bg: Color::Rgb(181, 137, 0),
                search_match_fg: Color::Rgb(0, 43, 54),
                search_current_bg: Color::Rgb(203, 75, 22),
                search_current_fg: Color::Rgb(0, 43, 54),
            },
        }
    }

    pub fn solarized_light() -> Self {
        Self {
            mode: ThemeMode::Light,
            syntax: SyntaxColors {
                comment: Color::Rgb(147, 161, 161),        // base1
                keyword: Color::Rgb(133, 153, 0),          // green
                string: Color::Rgb(42, 161, 152),          // cyan
                number: Color::Rgb(108, 113, 196),         // violet
                function: Color::Rgb(38, 139, 210),        // blue
                function_macro: Color::Rgb(211, 54, 130),  // magenta
                r#type: Color::Rgb(181, 137, 0),           // yellow
                variable_builtin: Color::Rgb(203, 75, 22), // orange
                variable_member: Color::Rgb(88, 110, 117), // base01
                module: Color::Rgb(203, 75, 22),           // orange
                operator: Color::Rgb(133, 153, 0),         // green
                tag: Color::Rgb(38, 139, 210),             // blue
                attribute: Color::Rgb(42, 161, 152),       // cyan
                label: Color::Rgb(203, 75, 22),            // orange
                punctuation: Color::Rgb(101, 123, 131),    // base00
                default_text: Color::Rgb(101, 123, 131),   // base00
            },
            diff: DiffColors {
                added_bg: Color::Rgb(230, 250, 235),
                added_gutter_bg: Color::Rgb(200, 235, 210),
                added_gutter_fg: Color::Rgb(133, 153, 0),
                deleted_bg: Color::Rgb(255, 240, 238),
                deleted_gutter_bg: Color::Rgb(250, 210, 205),
                deleted_gutter_fg: Color::Rgb(220, 50, 47),
                context_bg: Color::Rgb(253, 246, 227), // base3
                empty_placeholder_fg: Color::Rgb(238, 232, 213), // base2
                added_word_bg: Color::Rgb(175, 235, 190),
                deleted_word_bg: Color::Rgb(255, 190, 185),
            },
            ui: UiColors {
                border_focused: Color::Rgb(38, 139, 210),
                border_unfocused: Color::Rgb(147, 161, 161),
                text_primary: Color::Rgb(101, 123, 131), // base00
                text_secondary: Color::Rgb(88, 110, 117), // base01
                text_muted: Color::Rgb(147, 161, 161),   // base1
                line_number: Color::Rgb(147, 161, 161),
                bg: Color::Rgb(238, 232, 213), // base2
                footer_branch_bg: Color::Rgb(147, 161, 161),
                footer_branch_fg: Color::Rgb(38, 139, 210),
                status_added: Color::Rgb(133, 153, 0),
                status_modified: Color::Rgb(181, 137, 0),
                status_deleted: Color::Rgb(220, 50, 47),
                stats_added: Color::Rgb(133, 153, 0),
                stats_removed: Color::Rgb(220, 50, 47),
                selection_bg: Color::Rgb(38, 139, 210),
                selection_fg: Color::White,
                highlight: Color::Rgb(181, 137, 0),
                viewed: Color::Rgb(133, 153, 0),
                watching: Color::Rgb(181, 137, 0),
                search_match_bg: Color::Rgb(181, 137, 0),
                search_match_fg: Color::White,
                search_current_bg: Color::Rgb(203, 75, 22),
                search_current_fg: Color::White,
            },
        }
    }
}

pub fn init(config_theme: Option<&str>) {
    // Priority: config file > env var > OS auto-detect
    let theme_name = config_theme
        .map(|s| s.to_string())
        .or_else(|| std::env::var("LUMEN_THEME").ok());

    let theme = if let Some(name) = theme_name {
        match name.parse::<ThemePreset>() {
            Ok(preset) => Theme::from_preset(preset),
            Err(err) => {
                eprintln!("\x1b[33mwarning:\x1b[0m {}", err);
                Theme::from_mode(ThemeMode::detect())
            }
        }
    } else {
        Theme::from_mode(ThemeMode::detect())
    };
    let _ = THEME.set(theme);
}

pub fn get() -> &'static Theme {
    THEME.get_or_init(|| Theme::from_mode(ThemeMode::detect()))
}
