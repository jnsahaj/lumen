use once_cell::sync::OnceCell;
use ratatui::prelude::Color;

static THEME: OnceCell<Theme> = OnceCell::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
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
}

#[derive(Debug, Clone)]
pub struct UiColors {
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub line_number: Color,
    pub footer_bg: Color,
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
            },
            ui: UiColors {
                border_focused: Color::Cyan,
                border_unfocused: Color::DarkGray,
                text_primary: Color::Rgb(230, 230, 230),
                text_secondary: Color::Rgb(200, 200, 200),
                text_muted: Color::Rgb(140, 140, 160),
                line_number: Color::DarkGray,
                footer_bg: Color::Rgb(30, 30, 40),
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
            },
            ui: UiColors {
                border_focused: Color::Rgb(9, 105, 218),
                border_unfocused: Color::Rgb(208, 215, 222),
                text_primary: Color::Rgb(36, 41, 47),
                text_secondary: Color::Rgb(87, 96, 106),
                text_muted: Color::Rgb(140, 149, 159),
                line_number: Color::Rgb(140, 149, 159),
                footer_bg: Color::Rgb(246, 248, 250),
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
}

pub fn init() {
    let mode = ThemeMode::detect();
    let _ = THEME.set(Theme::from_mode(mode));
}

pub fn get() -> &'static Theme {
    THEME.get_or_init(|| Theme::from_mode(ThemeMode::detect()))
}
