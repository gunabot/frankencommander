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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    NameAsc,
    NameDesc,
    ExtAsc,
    ExtDesc,
    TimeAsc,
    TimeDesc,
    SizeAsc,
    SizeDesc,
    Unsorted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelMode {
    Brief,
    #[default]
    Full,
    Info,
    Tree,
    QuickView,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyDialogFocus {
    Input,
    IncludeSubdirs,
    CopyNewerOnly,
    UseFilters,
    CheckTargetSpace,
    BtnCopy,
    BtnTree,
    BtnFilters,
    BtnCancel,
}

#[derive(Debug, Clone)]
pub struct CopyDialogState {
    pub sources: Vec<PathBuf>,
    pub source_name: String,
    pub dest: String,
    pub cursor: usize,
    pub include_subdirs: bool,
    pub copy_newer_only: bool,
    pub use_filters: bool,
    pub check_target_space: bool,
    pub focus: CopyDialogFocus,
}

#[derive(Debug, Clone)]
pub enum Modal {
    CopyDialog(CopyDialogState),
    MoveDialog(CopyDialogState),
    DeleteDialog {
        sources: Vec<PathBuf>,
        source_name: String,
        use_filters: bool,
        focus: usize, // 0=checkbox, 1=Delete, 2=Filters, 3=Cancel
    },
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
        page: usize,      // 0=Screen, 1=Panel Options, 2=Confirmations
        selected: usize,
        show_hidden: bool,
        auto_save: bool,
        confirm_delete: bool,
        confirm_overwrite: bool,
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
    Help {
        page: usize,  // 0=Overview, 1=Keys, 2=Panels, 3=Files
        scroll: usize,
    },
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
    // Left panel actions
    LeftBrief,
    LeftFull,
    LeftInfo,
    LeftTree,
    LeftQuickView,
    LeftOnOff,
    LeftSortName,
    LeftSortExt,
    LeftSortTime,
    LeftSortSize,
    LeftUnsorted,
    LeftReread,
    LeftFilter,
    LeftDrive,
    // Right panel actions
    RightBrief,
    RightFull,
    RightInfo,
    RightTree,
    RightQuickView,
    RightOnOff,
    RightSortName,
    RightSortExt,
    RightSortTime,
    RightSortSize,
    RightUnsorted,
    RightReread,
    RightFilter,
    RightDrive,
    Help,
    About,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
    pub shortcut: Option<&'static str>,
    pub checked: Option<bool>,
    pub separator_after: bool,
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
    pub mode: PanelMode,
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
            mode: PanelMode::default(),
        }
    }
}
