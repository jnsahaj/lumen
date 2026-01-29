use crate::command::diff::types::{DiffFullscreen, DiffLine, DiffPanelFocus};

/// Layout information for the diff panels
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PanelLayout {
    /// X position of the old panel content area
    pub old_panel_x: u16,
    /// Width of the old panel content area
    pub old_panel_width: u16,
    /// X position of the new panel content area
    pub new_panel_x: u16,
    /// Width of the new panel content area
    pub new_panel_width: u16,
    /// Width of the focus indicator (1 char)
    pub focus_indicator_width: u16,
    /// Width of the gutter (line numbers + space, typically 5 chars)
    pub gutter_width: u16,
    /// Width of the border (1 char)
    pub border_width: u16,
    /// Whether sidebar is shown
    pub show_sidebar: bool,
    /// Sidebar width if shown
    pub sidebar_width: u16,
    /// Current fullscreen mode
    pub diff_fullscreen: DiffFullscreen,
}

impl PanelLayout {
    /// Calculate panel layout from terminal dimensions and current state
    pub fn calculate(
        term_width: u16,
        sidebar_width: u16,
        show_sidebar: bool,
        diff_fullscreen: DiffFullscreen,
    ) -> Self {
        let focus_indicator_width = 1u16;
        let gutter_width = 5u16; // "1234 " format
        let border_width = 1u16;

        let diff_area_start = if show_sidebar { sidebar_width } else { 0 };
        let diff_area_width = term_width.saturating_sub(diff_area_start);

        let (old_panel_x, old_panel_width, new_panel_x, new_panel_width) = match diff_fullscreen {
            DiffFullscreen::OldOnly => {
                // Old panel takes full width
                let panel_x = diff_area_start + border_width;
                let panel_width = diff_area_width.saturating_sub(border_width * 2);
                (panel_x, panel_width, 0, 0)
            }
            DiffFullscreen::NewOnly => {
                // New panel takes full width
                let panel_x = diff_area_start + border_width;
                let panel_width = diff_area_width.saturating_sub(border_width * 2);
                (0, 0, panel_x, panel_width)
            }
            DiffFullscreen::None => {
                // Side-by-side: split evenly
                let half_width = diff_area_width / 2;
                let old_x = diff_area_start + border_width;
                let old_width = half_width.saturating_sub(border_width);
                // New panel shares border with old panel
                let new_x = diff_area_start + half_width;
                let new_width = diff_area_width.saturating_sub(half_width).saturating_sub(border_width);
                (old_x, old_width, new_x, new_width)
            }
        };

        Self {
            old_panel_x,
            old_panel_width,
            new_panel_x,
            new_panel_width,
            focus_indicator_width,
            gutter_width,
            border_width,
            show_sidebar,
            sidebar_width,
            diff_fullscreen,
        }
    }

    /// Determine which panel (if any) is at a given x coordinate
    pub fn panel_at_x(&self, x: u16) -> Option<DiffPanelFocus> {
        match self.diff_fullscreen {
            DiffFullscreen::OldOnly => {
                if self.old_panel_width > 0 && x >= self.old_panel_x && x < self.old_panel_x + self.old_panel_width {
                    Some(DiffPanelFocus::Old)
                } else {
                    None
                }
            }
            DiffFullscreen::NewOnly => {
                if self.new_panel_width > 0 && x >= self.new_panel_x && x < self.new_panel_x + self.new_panel_width {
                    Some(DiffPanelFocus::New)
                } else {
                    None
                }
            }
            DiffFullscreen::None => {
                if self.old_panel_width > 0 && x >= self.old_panel_x && x < self.old_panel_x + self.old_panel_width {
                    Some(DiffPanelFocus::Old)
                } else if self.new_panel_width > 0 && x >= self.new_panel_x && x < self.new_panel_x + self.new_panel_width {
                    Some(DiffPanelFocus::New)
                } else {
                    None
                }
            }
        }
    }

    /// Check if an x coordinate is within the gutter (line numbers) area of a panel
    pub fn is_in_gutter(&self, x: u16, panel: DiffPanelFocus) -> bool {
        let (panel_x, panel_width) = match panel {
            DiffPanelFocus::Old => (self.old_panel_x, self.old_panel_width),
            DiffPanelFocus::New => (self.new_panel_x, self.new_panel_width),
            DiffPanelFocus::None => return false,
        };

        if panel_width == 0 {
            return false;
        }

        let rel_x = x.saturating_sub(panel_x);

        // Layout within panel: [focus_indicator 1][line_num 4][space 1][content...]
        // Old panel always has focus indicator, new panel only in fullscreen mode
        let gutter_start = match panel {
            DiffPanelFocus::Old => self.focus_indicator_width,
            DiffPanelFocus::New => {
                if self.diff_fullscreen == DiffFullscreen::NewOnly {
                    self.focus_indicator_width
                } else {
                    0
                }
            }
            DiffPanelFocus::None => return false,
        };

        let gutter_end = gutter_start + self.gutter_width;
        rel_x >= gutter_start && rel_x < gutter_end
    }

    /// Get the x offset where content starts within a panel
    pub fn content_x_offset(&self, panel: DiffPanelFocus) -> u16 {
        match panel {
            DiffPanelFocus::Old => {
                // Old panel: [border 1][focus 1][gutter 5][content...]
                self.focus_indicator_width + self.gutter_width
            }
            DiffPanelFocus::New => {
                if self.diff_fullscreen == DiffFullscreen::NewOnly {
                    // Fullscreen new: [border 1][focus 1][gutter 5][content...]
                    self.focus_indicator_width + self.gutter_width
                } else {
                    // Side-by-side new: no focus indicator, no left border
                    // [gutter 5][content...]
                    self.gutter_width
                }
            }
            DiffPanelFocus::None => 0,
        }
    }

    /// Convert screen coordinates to content position
    #[allow(dead_code)]
    /// Returns None if the position is not valid (e.g., empty placeholder line)
    pub fn screen_to_content(
        &self,
        x: u16,
        y: u16,
        panel: DiffPanelFocus,
        scroll: u16,
        h_scroll: u16,
        _header_height: u16,
        content_start_y: u16,
        side_by_side: &[DiffLine],
        context_line_count: usize,
    ) -> Option<(usize, usize)> {
        let (panel_x, panel_width) = match panel {
            DiffPanelFocus::Old => (self.old_panel_x, self.old_panel_width),
            DiffPanelFocus::New => (self.new_panel_x, self.new_panel_width),
            DiffPanelFocus::None => return None,
        };

        if panel_width == 0 {
            return None;
        }

        // Calculate line index from y coordinate
        // y is in screen coordinates, need to account for header and content area
        if y < content_start_y {
            return None;
        }

        let rel_y = (y - content_start_y) as usize;

        // Skip context lines at top
        if rel_y < context_line_count {
            return None; // Click on context line
        }

        let content_rel_y = rel_y - context_line_count;
        let line_idx = scroll as usize + content_rel_y;

        if line_idx >= side_by_side.len() {
            return None;
        }

        // Calculate column from x coordinate
        let rel_x = x.saturating_sub(panel_x);
        let content_offset = self.content_x_offset(panel);

        if rel_x < content_offset {
            // Click is in gutter area, column 0
            return Some((line_idx, 0));
        }

        let content_x = rel_x - content_offset;
        let column = (content_x + h_scroll) as usize;

        Some((line_idx, column))
    }
}

/// Check if a cursor position is valid for selection
/// Returns false for empty placeholder lines (no content)
#[allow(dead_code)]
pub fn is_valid_cursor_position(
    line: usize,
    panel: DiffPanelFocus,
    side_by_side: &[DiffLine],
) -> bool {
    if line >= side_by_side.len() {
        return false;
    }

    let diff_line = &side_by_side[line];
    match panel {
        DiffPanelFocus::Old => diff_line.old_line.is_some(),
        DiffPanelFocus::New => diff_line.new_line.is_some(),
        DiffPanelFocus::None => false,
    }
}

/// Extract selected text from the diff
pub fn extract_selected_text(
    selection: &crate::command::diff::types::Selection,
    side_by_side: &[DiffLine],
) -> Option<String> {
    if !selection.is_active() {
        return None;
    }

    let (start, end) = selection.normalized_range();
    let mut result = String::new();

    for line_idx in start.line..=end.line {
        if line_idx >= side_by_side.len() {
            break;
        }

        let diff_line = &side_by_side[line_idx];
        let line_content = match selection.panel {
            DiffPanelFocus::Old => diff_line.old_line.as_ref().map(|(_, text)| text.as_str()),
            DiffPanelFocus::New => diff_line.new_line.as_ref().map(|(_, text)| text.as_str()),
            DiffPanelFocus::None => None,
        };

        if let Some(text) = line_content {
            match selection.mode {
                crate::command::diff::types::SelectionMode::Line => {
                    // Line mode: include full line
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(text);
                }
                crate::command::diff::types::SelectionMode::Character => {
                    if !result.is_empty() {
                        result.push('\n');
                    }

                    if start.line == end.line {
                        // Single line selection
                        let start_col = start.column.min(text.len());
                        let end_col = end.column.min(text.len());
                        if start_col < end_col {
                            result.push_str(&text[start_col..end_col]);
                        }
                    } else if line_idx == start.line {
                        // First line
                        let start_col = start.column.min(text.len());
                        result.push_str(&text[start_col..]);
                    } else if line_idx == end.line {
                        // Last line
                        let end_col = end.column.min(text.len());
                        result.push_str(&text[..end_col]);
                    } else {
                        // Middle line - full content
                        result.push_str(text);
                    }
                }
                crate::command::diff::types::SelectionMode::None => {}
            }
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
