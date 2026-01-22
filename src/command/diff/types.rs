use super::context::ContextConfig;

pub fn expand_tabs(s: &str, tab_width: usize) -> String {
    if tab_width == 0 {
        return s.replace('\t', "");
    }
    let mut result = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = tab_width - (col % tab_width);
            for _ in 0..spaces {
                result.push(' ');
            }
            col += spaces;
        } else {
            result.push(c);
            col += 1;
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

impl FileStatus {
    pub fn symbol(&self) -> &'static str {
        match self {
            FileStatus::Added => "A",
            FileStatus::Modified => "M",
            FileStatus::Deleted => "D",
        }
    }
}

pub struct FileDiff {
    pub filename: String,
    pub old_content: String,
    pub new_content: String,
    pub status: FileStatus,
    pub is_binary: bool,
}

/// Detect if content is binary by checking for null bytes in the first 8KB
pub fn is_binary_content(content: &str) -> bool {
    content.bytes().take(8192).any(|b| b == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum FilePanelPosition {
    #[default]
    Left,
    Bottom,
}

/// Settings for the diff view UI. Designed to be easily extended
/// with additional configuration options in the future.
#[derive(Clone)]
pub struct DiffViewSettings {
    pub context: ContextConfig,
    pub tab_width: usize,
    pub file_panel_position: FilePanelPosition,
}

impl Default for DiffViewSettings {
    fn default() -> Self {
        Self {
            context: ContextConfig::default(),
            tab_width: 4,
            file_panel_position: FilePanelPosition::default(),
        }
    }
}

/// Represents a segment of text with optional emphasis for word-level highlighting
#[derive(Clone, Debug)]
pub struct InlineSegment {
    pub text: String,
    /// If true, this segment represents a changed word that should be emphasized
    pub emphasized: bool,
}

pub struct DiffLine {
    pub old_line: Option<(usize, String)>,
    pub new_line: Option<(usize, String)>,
    pub change_type: ChangeType,
    /// Word-level segments for the old line (only populated for Modified lines)
    pub old_segments: Option<Vec<InlineSegment>>,
    /// Word-level segments for the new line (only populated for Modified lines)
    pub new_segments: Option<Vec<InlineSegment>>,
}

#[derive(Clone, Copy)]
pub enum ChangeType {
    Equal,
    Delete,
    Insert,
    /// A paired delete+insert, shown on the same row (GitHub-style)
    Modified,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum FocusedPanel {
    FilePanel,
    #[default]
    DiffView,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum DiffFullscreen {
    #[default]
    None,
    OldOnly,
    NewOnly,
}

#[derive(Clone)]
pub enum FilePanelItem {
    Directory {
        name: String,
        path: String,
        depth: usize,
    },
    File {
        name: String,
        path: String,
        file_index: usize,
        depth: usize,
        status: FileStatus,
    },
}

pub fn build_file_tree(file_diffs: &[FileDiff]) -> Vec<FilePanelItem> {
    use std::collections::{BTreeMap, BTreeSet};

    if file_diffs.is_empty() {
        return Vec::new();
    }

    let mut file_paths: Vec<(String, usize, FileStatus)> = file_diffs
        .iter()
        .enumerate()
        .map(|(idx, diff)| (diff.filename.clone(), idx, diff.status))
        .collect();
    file_paths.sort_by(|a, b| a.0.cmp(&b.0));

    // Count children for each directory path
    let mut dir_children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    dir_children.insert(String::new(), BTreeSet::new()); // root

    for (path, _, _) in &file_paths {
        let parts: Vec<&str> = path.split('/').collect();

        // Add file as child of its parent directory
        if parts.len() > 1 {
            let parent_path = parts[..parts.len() - 1].join("/");
            dir_children
                .entry(parent_path)
                .or_default()
                .insert(path.clone());
        } else {
            // File at root level
            dir_children.get_mut("").unwrap().insert(path.clone());
        }

        // Add each directory as child of its parent
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            let parent_path = if i == 0 {
                String::new()
            } else {
                parts[..i].join("/")
            };

            dir_children.entry(dir_path.clone()).or_default();
            dir_children
                .entry(parent_path)
                .or_default()
                .insert(dir_path);
        }
    }

    // Find the collapsed path for a directory (collapse single-child chains)
    fn get_collapsed_dir(
        dir_path: &str,
        dir_children: &BTreeMap<String, BTreeSet<String>>,
        file_paths_set: &BTreeSet<String>,
    ) -> String {
        let mut current = dir_path.to_string();

        loop {
            if let Some(children) = dir_children.get(&current) {
                // If directory has exactly one child and it's a directory (not a file)
                if children.len() == 1 {
                    let child = children.iter().next().unwrap();
                    if !file_paths_set.contains(child) {
                        // It's a directory, continue collapsing
                        current = child.clone();
                        continue;
                    }
                }
            }
            break;
        }

        current
    }

    let file_paths_set: BTreeSet<String> = file_paths.iter().map(|(p, _, _)| p.clone()).collect();

    let mut items: Vec<FilePanelItem> = Vec::new();
    let mut added_dirs: BTreeSet<String> = BTreeSet::new();
    // Maps any directory path to its collapsed version
    let mut path_to_collapsed: BTreeMap<String, String> = BTreeMap::new();
    // Maps collapsed path to its depth
    let mut collapsed_depth: BTreeMap<String, usize> = BTreeMap::new();

    for (path, file_idx, status) in &file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        let file_name = parts.last().unwrap_or(&"").to_string();

        // Process directories
        let mut i = 0;
        while i < parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");

            // Check if this directory should be collapsed
            let collapsed_path = get_collapsed_dir(&dir_path, &dir_children, &file_paths_set);

            if !added_dirs.contains(&collapsed_path) {
                added_dirs.insert(collapsed_path.clone());

                // Register all intermediate paths as mapping to this collapsed path
                // Use entry().or_insert() to avoid overwriting existing mappings
                let collapsed_parts: Vec<&str> = collapsed_path.split('/').collect();
                for j in 0..collapsed_parts.len() {
                    let intermediate = collapsed_parts[..=j].join("/");
                    path_to_collapsed
                        .entry(intermediate)
                        .or_insert(collapsed_path.clone());
                }

                // Calculate depth based on parent's collapsed path
                let depth = if i == 0 {
                    0
                } else {
                    let parent_dir = parts[..i].join("/");
                    if let Some(parent_collapsed) = path_to_collapsed.get(&parent_dir) {
                        collapsed_depth
                            .get(parent_collapsed)
                            .map(|d| d + 1)
                            .unwrap_or(0)
                    } else {
                        0
                    }
                };

                collapsed_depth.insert(collapsed_path.clone(), depth);

                // Calculate display name: relative to parent collapsed directory
                let display_name = if i == 0 {
                    collapsed_path.clone()
                } else {
                    let parent_dir = parts[..i].join("/");
                    if let Some(parent_collapsed) = path_to_collapsed.get(&parent_dir) {
                        // Strip the parent collapsed path prefix
                        collapsed_path
                            .strip_prefix(&format!("{}/", parent_collapsed))
                            .unwrap_or(&collapsed_path)
                            .to_string()
                    } else {
                        collapsed_path.clone()
                    }
                };

                items.push(FilePanelItem::Directory {
                    name: display_name,
                    path: collapsed_path.clone(),
                    depth,
                });

                // Skip to the end of the collapsed path
                i = collapsed_parts.len();
                continue;
            } else {
                // Directory already added, skip ahead
                let collapsed_parts: Vec<&str> = collapsed_path.split('/').collect();
                i = collapsed_parts.len();
                continue;
            }
        }

        // Calculate file depth based on its parent directory
        let file_depth = if parts.len() > 1 {
            let parent_dir = parts[..parts.len() - 1].join("/");
            if let Some(parent_collapsed) = path_to_collapsed.get(&parent_dir) {
                collapsed_depth
                    .get(parent_collapsed)
                    .map(|d| d + 1)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        };

        items.push(FilePanelItem::File {
            name: file_name,
            path: path.clone(),
            file_index: *file_idx,
            depth: file_depth,
            status: *status,
        });
    }

    items
}
