#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use ftui::widgets::table::TableState;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub is_system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePane {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    NameAsc,
    NameDesc,
    TimeAsc,
    TimeDesc,
}

#[derive(Debug, Clone)]
pub struct Viewer {
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct LayoutCache {
    pub left_table: ftui::core::geometry::Rect,
    pub right_table: ftui::core::geometry::Rect,
}

#[derive(Debug, Clone)]
pub struct ClickInfo {
    pub pane: ActivePane,
    pub row: usize,
    pub at: std::time::Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum RefreshMode {
    Reset,
    Keep,
}

#[derive(Debug, Clone)]
pub enum PendingPrompt {
    CopyTo { sources: Vec<PathBuf> },
    MoveTo { sources: Vec<PathBuf> },
    Mkdir { base: PathBuf },
    Find { base: PathBuf },
    Chmod { target: PathBuf },
}

#[derive(Debug, Clone)]
pub enum PendingConfirm {
    Delete { sources: Vec<PathBuf> },
    Overwrite {
        kind: OverwriteKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    Sync {
        ops: Vec<PathBuf>,
        src_root: PathBuf,
        dst_root: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub enum Modal {
    Prompt {
        title: String,
        label: String,
        value: String,
        cursor: usize,
        action: PendingPrompt,
    },
    Confirm {
        title: String,
        message: String,
        action: PendingConfirm,
    },
    FindResults {
        query: String,
        items: Vec<PathBuf>,
        selected: usize,
        scroll: usize,
    },
    Tree {
        pane: ActivePane,
        items: Vec<TreeItem>,
        selected: usize,
        scroll: usize,
    },
    DriveMenu {
        pane: ActivePane,
        items: Vec<PathBuf>,
        selected: usize,
        scroll: usize,
    },
    Config {
        selected: usize,
        show_hidden: bool,
    },
    PanelOptions {
        pane: ActivePane,
        selected: usize,
        dirs_first: bool,
        sort_mode: SortMode,
    },
    UserMenu {
        items: Vec<UserMenuItem>,
        selected: usize,
        scroll: usize,
        config_path: PathBuf,
    },
    About,
    Help,
    PullDown {
        menu_idx: usize,
        item_idx: usize,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum OverwriteKind {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerAction {
    None,
    Close,
    Quit,
}

#[derive(Debug, Clone, Copy)]
pub enum MenuAction {
    None,
    Quit,
    View,
    Edit,
    Copy,
    Move,
    Tree,
    Find,
    Config,
    PanelOptions,
    LeftSortName,
    LeftSortTime,
    RightSortName,
    RightSortTime,
    Help,
    About,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
}

#[derive(Debug, Clone)]
pub struct TreeItem {
    pub path: PathBuf,
    pub depth: usize,
}

#[derive(Debug, Clone)]
pub struct VfsState {
    pub zip_path: PathBuf,
    pub prefix: String,
}

#[derive(Debug, Clone)]
pub struct UserMenuItem {
    pub label: String,
    pub command: String,
}

#[derive(Debug)]
pub struct Pane {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub state: RefCell<TableState>,
    pub selected: HashSet<PathBuf>,
    pub sort_mode: SortMode,
    pub dirs_first: bool,
    pub vfs: Option<VfsState>,
    pub panelized: Option<Vec<PathBuf>>,
}

impl Pane {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            state: RefCell::new(TableState::default()),
            selected: HashSet::new(),
            sort_mode: SortMode::NameAsc,
            dirs_first: true,
            vfs: None,
            panelized: None,
        }
    }
}
