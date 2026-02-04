#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::{
    cursor::MoveTo,
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use zip::ZipArchive;
use ftui::prelude::*;
use ftui::core::geometry::Rect;
use ftui::layout::{Constraint, Flex};
use ftui::render::cell::PackedRgba;
use ftui::render::diff_strategy::DiffStrategyConfig;
use ftui::style::Style;
use ftui::text::{Text, WrapMode, display_width};
use ftui::widgets::block::Block;
use ftui::widgets::borders::Borders;
use ftui::widgets::paragraph::Paragraph;
use ftui::widgets::status_line::{StatusItem, StatusLine};
use ftui::widgets::table::{Row, Table, TableState};
use ftui::widgets::{StatefulWidget, Widget};
use ftui::{KeyEventKind, MouseButton, MouseEvent, MouseEventKind, Program, ProgramConfig, RuntimeDiffConfig};
use ftui::render::budget::FrameBudgetConfig;
use time::{OffsetDateTime, UtcOffset};

const MENU_HEIGHT: u16 = 1;
const STATUS_HEIGHT: u16 = 1;
const KEYBAR_HEIGHT: u16 = 1;
const HEADER_HEIGHT: u16 = 1;
const DOUBLE_CLICK_MS: u64 = 400;

const MENU_TITLES: [&str; 6] = ["File", "Command", "Options", "Left", "Right", "Help"];

fn main() -> io::Result<()> {
    let mut budget = FrameBudgetConfig::with_total(Duration::from_millis(50));
    budget.allow_frame_skip = false;
    budget.degradation_cooldown = 0;
    let strategy_config = DiffStrategyConfig {
        c_scan: 1000.0,
        c_emit: 1.0,
        c_row: 100.0,
        prior_alpha: 1.0,
        prior_beta: 1.0,
        decay: 1.0,
        conservative: false,
        conservative_quantile: 0.95,
        min_observation_cells: 0,
        hysteresis_ratio: 0.0,
        uncertainty_guard_variance: 0.0,
    };
    let diff_config = RuntimeDiffConfig::default()
        .with_bayesian_enabled(true)
        .with_dirty_rows_enabled(false)
        .with_dirty_spans_enabled(false)
        .with_tile_skip_enabled(false)
        .with_strategy_config(strategy_config);
    let config = ProgramConfig::fullscreen()
        .with_mouse()
        .with_budget(budget)
        .with_diff_config(diff_config);
    let mut program = Program::with_config(App::new()?, config)?;
    program.run()
}

#[derive(Debug, Clone, Copy)]
struct ThemeColors {
    screen_bg: PackedRgba,
    menu_bg: PackedRgba,
    menu_fg: PackedRgba,
    panel_bg: PackedRgba,
    panel_fg: PackedRgba,
    system_fg: PackedRgba,
    panel_border_active: PackedRgba,
    panel_border_inactive: PackedRgba,
    header_bg: PackedRgba,
    header_fg: PackedRgba,
    selection_bg: PackedRgba,
    selection_fg: PackedRgba,
    keybar_bg: PackedRgba,
    keybar_fg: PackedRgba,
    status_bg: PackedRgba,
    status_fg: PackedRgba,
    dialog_bg: PackedRgba,
    dialog_fg: PackedRgba,
}

impl ThemeColors {
    fn classic() -> Self {
        Self {
            screen_bg: PackedRgba::rgb(0, 0, 128),
            menu_bg: PackedRgba::rgb(0, 128, 128),
            menu_fg: PackedRgba::rgb(255, 255, 255),
            panel_bg: PackedRgba::rgb(0, 0, 128),
            panel_fg: PackedRgba::rgb(192, 192, 192),
            system_fg: PackedRgba::rgb(128, 128, 128),
            panel_border_active: PackedRgba::rgb(0, 255, 255),
            panel_border_inactive: PackedRgba::rgb(128, 128, 128),
            header_bg: PackedRgba::rgb(0, 0, 128),
            header_fg: PackedRgba::rgb(255, 255, 255),
            selection_bg: PackedRgba::rgb(255, 255, 0),
            selection_fg: PackedRgba::rgb(0, 0, 0),
            keybar_bg: PackedRgba::rgb(0, 128, 128),
            keybar_fg: PackedRgba::rgb(255, 255, 255),
            status_bg: PackedRgba::rgb(0, 0, 128),
            status_fg: PackedRgba::rgb(255, 255, 255),
            dialog_bg: PackedRgba::rgb(192, 192, 192),
            dialog_fg: PackedRgba::rgb(0, 0, 0),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
    is_system: bool,
}

#[derive(Debug)]
struct Pane {
    cwd: PathBuf,
    entries: Vec<Entry>,
    state: RefCell<TableState>,
    selected: HashSet<PathBuf>,
    sort_mode: SortMode,
    dirs_first: bool,
    vfs: Option<VfsState>,
    panelized: Option<Vec<PathBuf>>,
}

impl Pane {
    fn new(cwd: PathBuf) -> io::Result<Self> {
        let mut pane = Self {
            cwd,
            entries: Vec::new(),
            state: RefCell::new(TableState::default()),
            selected: HashSet::new(),
            sort_mode: SortMode::NameAsc,
            dirs_first: true,
            vfs: None,
            panelized: None,
        };
        pane.refresh(RefreshMode::Reset, false)?;
        Ok(pane)
    }

    fn refresh(&mut self, mode: RefreshMode, show_hidden: bool) -> io::Result<()> {
        if let Some(panelized) = &self.panelized {
            self.entries = read_panelized(panelized)?;
        } else if let Some(vfs) = &self.vfs {
            self.entries = read_zip_entries(vfs, show_hidden)?;
        } else {
            self.entries = read_entries(&self.cwd, self.sort_mode, self.dirs_first, show_hidden)?;
        }
        self.selected.retain(|path| self.entries.iter().any(|e| &e.path == path));

        let mut state = self.state.borrow_mut();
        if self.entries.is_empty() {
            state.select(None);
            state.offset = 0;
            return Ok(());
        }

        match mode {
            RefreshMode::Reset => {
                state.select(Some(0));
                state.offset = 0;
            }
            RefreshMode::Keep => {
                let current = state.selected.unwrap_or(0).min(self.entries.len() - 1);
                state.select(Some(current));
            }
        }

        Ok(())
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let state = self.state.borrow();
        let idx = state.selected?;
        self.entries.get(idx)
    }

    fn move_selection(&mut self, delta: i32, view_height: usize) {
        if self.entries.is_empty() {
            let mut state = self.state.borrow_mut();
            state.select(None);
            state.offset = 0;
            return;
        }
        let mut state = self.state.borrow_mut();
        let current = state.selected.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, (self.entries.len() - 1) as i32) as usize;
        state.select(Some(next));
        ensure_visible(&mut state, view_height);
    }

    fn go_parent(&mut self, show_hidden: bool) -> io::Result<()> {
        if let Some(vfs) = &mut self.vfs {
            if let Some(parent) = zip_parent_prefix(&vfs.prefix) {
                vfs.prefix = parent;
                return self.refresh(RefreshMode::Reset, show_hidden);
            }
            self.vfs = None;
        }
        if self.panelized.is_some() {
            self.panelized = None;
            return self.refresh(RefreshMode::Reset, show_hidden);
        }
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.refresh(RefreshMode::Reset, show_hidden)?;
        }
        Ok(())
    }

    fn enter_selected(&mut self, show_hidden: bool) -> io::Result<bool> {
        let Some(entry) = self.selected_entry() else {
            return Ok(false);
        };
        let entry_path = entry.path.clone();
        let entry_name = entry.name.clone();
        let is_dir = entry.is_dir;
        if is_dir {
            if let Some(vfs) = &mut self.vfs {
                vfs.prefix = zip_child_prefix(&vfs.prefix, &entry_path);
                self.refresh(RefreshMode::Reset, show_hidden)?;
            } else {
                self.cwd = entry_path;
                self.panelized = None;
                self.refresh(RefreshMode::Reset, show_hidden)?;
            }
            return Ok(true);
        }
        if entry_name.to_lowercase().ends_with(".zip") && self.vfs.is_none() {
            self.vfs = Some(VfsState {
                zip_path: entry_path,
                prefix: String::new(),
            });
            self.refresh(RefreshMode::Reset, show_hidden)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn toggle_select(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let path = entry.path.clone();
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
    }

    fn select_all(&mut self) {
        self.selected = self.entries.iter().map(|e| e.path.clone()).collect();
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
    }

    fn invert_selection(&mut self) {
        let mut next = HashSet::new();
        for entry in &self.entries {
            if !self.selected.contains(&entry.path) {
                next.insert(entry.path.clone());
            }
        }
        self.selected = next;
    }

    fn selected_total_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.path))
            .map(|e| e.size)
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePane {
    Left,
    Right,
}

#[derive(Debug, Clone)]
struct Viewer {
    path: PathBuf,
    lines: Vec<String>,
    scroll: usize,
}

#[derive(Debug, Clone, Copy)]
struct LayoutCache {
    left_table: Rect,
    right_table: Rect,
}

#[derive(Debug, Clone)]
struct ClickInfo {
    pane: ActivePane,
    row: usize,
    at: Instant,
}

#[derive(Debug, Clone, Copy)]
enum RefreshMode {
    Reset,
    Keep,
}

#[derive(Debug, Clone)]
enum PendingPrompt {
    CopyTo { sources: Vec<PathBuf> },
    MoveTo { sources: Vec<PathBuf> },
    Mkdir { base: PathBuf },
    Find { base: PathBuf },
    Chmod { target: PathBuf },
}

#[derive(Debug, Clone)]
enum PendingConfirm {
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
enum Modal {
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
enum OverwriteKind {
    Copy,
    Move,
}

#[derive(Debug)]
struct App {
    left: Pane,
    right: Pane,
    active: ActivePane,
    status: String,
    viewer: Option<Viewer>,
    layout: RefCell<Option<LayoutCache>>,
    last_click: Option<ClickInfo>,
    theme: ThemeColors,
    modal: Option<Modal>,
    log: Option<std::fs::File>,
    force_clear_frames: RefCell<u8>,
    show_hidden: bool,
    hide_left: bool,
    hide_right: bool,
    hide_all: bool,
    cmdline: String,
    cmd_cursor: usize,
}

#[derive(Debug, Clone)]
enum Msg {
    Event(Event),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerAction {
    None,
    Close,
    Quit,
}

#[derive(Debug, Clone, Copy)]
enum MenuAction {
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
struct MenuItem {
    label: &'static str,
    action: MenuAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortMode {
    NameAsc,
    NameDesc,
    TimeAsc,
    TimeDesc,
}

#[derive(Debug, Clone)]
struct TreeItem {
    path: PathBuf,
    depth: usize,
}

#[derive(Debug, Clone)]
struct VfsState {
    zip_path: PathBuf,
    prefix: String,
}

#[derive(Debug, Clone)]
struct UserMenuItem {
    label: String,
    command: String,
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        Msg::Event(event)
    }
}

impl App {
    fn new() -> io::Result<Self> {
        let cwd = std::env::current_dir()?;
        let left = Pane::new(cwd.clone())?;
        let right = Pane::new(cwd)?;
        let log = match std::env::var("FC_DEBUG_LOG") {
            Ok(_) => std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/frankencommander.log")
                .ok(),
            Err(_) => None,
        };
        Ok(Self {
            left,
            right,
            active: ActivePane::Left,
            status: String::from("Ready"),
            viewer: None,
            layout: RefCell::new(None),
            last_click: None,
            theme: ThemeColors::classic(),
            modal: None,
            log,
            force_clear_frames: RefCell::new(0),
            show_hidden: false,
            hide_left: false,
            hide_right: false,
            hide_all: false,
            cmdline: String::new(),
            cmd_cursor: 0,
        })
    }

    fn log_event(&mut self, msg: &str) {
        if let Some(file) = self.log.as_mut() {
            let _ = writeln!(file, "{} {}", OffsetDateTime::now_utc(), msg);
        }
    }

    fn active_pane_mut(&mut self) -> &mut Pane {
        match self.active {
            ActivePane::Left => &mut self.left,
            ActivePane::Right => &mut self.right,
        }
    }

    fn inactive_pane_mut(&mut self) -> &mut Pane {
        match self.active {
            ActivePane::Left => &mut self.right,
            ActivePane::Right => &mut self.left,
        }
    }

    fn active_pane(&self) -> &Pane {
        match self.active {
            ActivePane::Left => &self.left,
            ActivePane::Right => &self.right,
        }
    }

    fn list_height(&self, pane: ActivePane) -> usize {
        let layout = self.layout.borrow();
        let Some(layout) = layout.as_ref() else {
            return 0;
        };
        let rect = match pane {
            ActivePane::Left => layout.left_table,
            ActivePane::Right => layout.right_table,
        };
        rect.height as usize
    }

    fn open_viewer(&mut self) {
        let Some(entry) = self.active_pane().selected_entry() else {
            self.status = "No file selected".to_string();
            return;
        };
        if entry.is_dir {
            self.status = "Cannot view directory".to_string();
            return;
        }
        if let Some(vfs) = self.active_pane().vfs.clone() {
            let path = entry.path.clone();
            self.open_zip_viewer(&vfs, &path);
            return;
        }
        let path = entry.path.clone();
        self.open_viewer_path(&path);
    }

    fn open_viewer_path(&mut self, path: &Path) {
        match read_file_lines(path) {
            Ok(lines) => {
                self.viewer = Some(Viewer {
                    path: path.to_path_buf(),
                    lines,
                    scroll: 0,
                });
            }
            Err(err) => {
                self.status = format!("View failed: {err}");
            }
        }
    }

    fn open_zip_viewer(&mut self, vfs: &VfsState, entry_path: &Path) {
        match read_zip_file_lines(vfs, entry_path) {
            Ok(lines) => {
                self.viewer = Some(Viewer {
                    path: entry_path.to_path_buf(),
                    lines,
                    scroll: 0,
                });
            }
            Err(err) => {
                self.status = format!("View failed: {err}");
            }
        }
    }

    fn open_editor(&mut self) {
        let Some(entry) = self.active_pane().selected_entry() else {
            self.status = "No file selected".to_string();
            return;
        };
        if entry.is_dir {
            self.status = "Cannot edit directory".to_string();
            return;
        }
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
        let result = run_external_editor(&editor, &entry.path);
        *self.force_clear_frames.borrow_mut() = 3;
        match result {
            Ok(()) => {
                self.status = "Editor closed".to_string();
                let show_hidden = self.show_hidden;
                let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
            }
            Err(err) => {
                self.status = format!("Editor failed: {err}");
            }
        }
    }

    fn begin_copy(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Copy from archive not supported".to_string();
            return;
        }
        let sources = selected_paths(self.active_pane());
        if sources.is_empty() {
            self.status = "No file selected".to_string();
            return;
        }
        let dest_dir = self.inactive_pane_mut().cwd.clone();
        let default = if sources.len() == 1 {
            dest_dir.join(
                self.active_pane()
                    .selected_entry()
                    .map(|e| e.name.as_str())
                    .unwrap_or(""),
            )
            .display()
            .to_string()
        } else {
            dest_dir.display().to_string()
        };
        self.modal = Some(Modal::Prompt {
            title: "Copy".to_string(),
            label: "Copy to:".to_string(),
            value: default.clone(),
            cursor: default.len(),
            action: PendingPrompt::CopyTo { sources },
        });
    }

    fn begin_move(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Move in archive not supported".to_string();
            return;
        }
        let sources = selected_paths(self.active_pane());
        if sources.is_empty() {
            self.status = "No file selected".to_string();
            return;
        }
        let dest_dir = self.inactive_pane_mut().cwd.clone();
        let default = if sources.len() == 1 {
            dest_dir
                .join(sources[0].file_name().unwrap_or_default())
                .display()
                .to_string()
        } else {
            dest_dir.display().to_string()
        };
        self.modal = Some(Modal::Prompt {
            title: "Move/Rename".to_string(),
            label: "To:".to_string(),
            value: default.clone(),
            cursor: default.len(),
            action: PendingPrompt::MoveTo { sources },
        });
    }

    fn begin_mkdir(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Mkdir in archive not supported".to_string();
            return;
        }
        let base = self.active_pane().cwd.clone();
        let default = "new_folder".to_string();
        self.modal = Some(Modal::Prompt {
            title: "Make directory".to_string(),
            label: "Directory name:".to_string(),
            value: default.clone(),
            cursor: default.len(),
            action: PendingPrompt::Mkdir { base },
        });
    }

    fn begin_delete(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Delete in archive not supported".to_string();
            return;
        }
        let sources = selected_paths(self.active_pane());
        if sources.is_empty() {
            self.status = "No file selected".to_string();
            return;
        }
        let message = format!("Delete {} item(s)?", sources.len());
        self.modal = Some(Modal::Confirm {
            title: "Delete".to_string(),
            message,
            action: PendingConfirm::Delete { sources },
        });
    }

    fn begin_find(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Find in archive not supported".to_string();
            return;
        }
        let base = self.active_pane().cwd.clone();
        let default = String::new();
        self.modal = Some(Modal::Prompt {
            title: "Find file".to_string(),
            label: "Search:".to_string(),
            value: default,
            cursor: 0,
            action: PendingPrompt::Find { base },
        });
    }

    fn open_tree(&mut self) {
        let pane = self.active;
        let base = match pane {
            ActivePane::Left => &self.left.cwd,
            ActivePane::Right => &self.right.cwd,
        };
        let items = build_tree(base, 2, self.show_hidden);
        self.modal = Some(Modal::Tree {
            pane,
            items,
            selected: 0,
            scroll: 0,
        });
    }

    fn open_drive_menu(&mut self, pane: ActivePane) {
        let items = list_drive_roots();
        self.modal = Some(Modal::DriveMenu {
            pane,
            items,
            selected: 0,
            scroll: 0,
        });
    }

    fn open_user_menu(&mut self) {
        let config_path = user_menu_path();
        let _ = ensure_user_menu_file(&config_path);
        let items = load_user_menu(&config_path);
        self.modal = Some(Modal::UserMenu {
            items,
            selected: 0,
            scroll: 0,
            config_path,
        });
    }

    fn begin_sync_dirs(&mut self) {
        if self.left.vfs.is_some() || self.right.vfs.is_some() {
            self.status = "Sync in archive not supported".to_string();
            return;
        }
        let (src, dst) = match self.active {
            ActivePane::Left => (self.left.cwd.clone(), self.right.cwd.clone()),
            ActivePane::Right => (self.right.cwd.clone(), self.left.cwd.clone()),
        };
        let ops = sync_plan(&src, &dst);
        if ops.is_empty() {
            self.status = "Directories already in sync".to_string();
            return;
        }
        let message = format!("Sync {} item(s)?", ops.len());
        self.modal = Some(Modal::Confirm {
            title: "Synchronize".to_string(),
            message,
            action: PendingConfirm::Sync {
                ops,
                src_root: src,
                dst_root: dst,
            },
        });
    }

    fn begin_chmod(&mut self) {
        if self.active_pane().vfs.is_some() {
            self.status = "Attributes in archive not supported".to_string();
            return;
        }
        let Some(entry) = self.active_pane().selected_entry() else {
            self.status = "No file selected".to_string();
            return;
        };
        let mode = entry
            .path
            .metadata()
            .map(|m| format!("{:o}", m.permissions().mode() & 0o777))
            .unwrap_or_else(|_| "644".to_string());
        self.modal = Some(Modal::Prompt {
            title: "Attributes".to_string(),
            label: "Chmod (octal):".to_string(),
            value: mode.clone(),
            cursor: mode.len(),
            action: PendingPrompt::Chmod {
                target: entry.path.clone(),
            },
        });
    }

    fn handle_cmdline_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        if key.kind != KeyEventKind::Press {
            return Cmd::none();
        }
        match key.code {
            KeyCode::Char('o') if key.modifiers.contains(Modifiers::CTRL) => {
                self.hide_all = !self.hide_all;
            }
            KeyCode::Char(ch) => {
                self.cmdline.insert(self.cmd_cursor, ch);
                self.cmd_cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cmd_cursor > 0 {
                    self.cmd_cursor -= 1;
                    self.cmdline.remove(self.cmd_cursor);
                }
            }
            KeyCode::Delete => {
                if self.cmd_cursor < self.cmdline.len() {
                    self.cmdline.remove(self.cmd_cursor);
                }
            }
            KeyCode::Left => {
                if self.cmd_cursor > 0 {
                    self.cmd_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cmd_cursor < self.cmdline.len() {
                    self.cmd_cursor += 1;
                }
            }
            KeyCode::Enter => {
                self.status = format!("Command: {}", self.cmdline);
                self.cmdline.clear();
                self.cmd_cursor = 0;
            }
            _ => {}
        }
        Cmd::none()
    }

    fn handle_key(&mut self, key: KeyEvent) -> Cmd<Msg> {
        if key.kind != KeyEventKind::Press {
            return Cmd::none();
        }
        self.log_event(&format!("key {:?} {:?}", key.code, key.modifiers));
        if self.hide_all {
            return self.handle_cmdline_key(key);
        }
        if let Some(modal) = self.modal.take() {
            return self.handle_modal_key(key, modal);
        }
        if self.viewer.is_some() {
            let mut action = ViewerAction::None;
            if let Some(viewer) = self.viewer.as_mut() {
                action = Self::handle_viewer_key(key, viewer);
            }
            match action {
                ViewerAction::None => {}
                ViewerAction::Close => self.viewer = None,
                ViewerAction::Quit => return Cmd::quit(),
            }
            return Cmd::none();
        }

        let view_height = self.list_height(self.active);

        match key.code {
            KeyCode::F(1) if key.modifiers.contains(Modifiers::ALT) => {
                self.open_drive_menu(ActivePane::Left);
            }
            KeyCode::F(2) if key.modifiers.contains(Modifiers::ALT) => {
                self.open_drive_menu(ActivePane::Right);
            }
            KeyCode::F(1) if key.modifiers.contains(Modifiers::CTRL) => {
                self.hide_left = !self.hide_left;
                if self.hide_left && self.active == ActivePane::Left {
                    self.active = ActivePane::Right;
                }
            }
            KeyCode::F(2) if key.modifiers.contains(Modifiers::CTRL) => {
                self.hide_right = !self.hide_right;
                if self.hide_right && self.active == ActivePane::Right {
                    self.active = ActivePane::Left;
                }
            }
            KeyCode::Char('o') if key.modifiers.contains(Modifiers::CTRL) => {
                self.hide_all = !self.hide_all;
            }
            KeyCode::F(8) if key.modifiers.contains(Modifiers::CTRL) => {
                self.begin_sync_dirs();
            }
            KeyCode::F(1) => self.modal = Some(Modal::Help),
            KeyCode::F(2) => self.open_user_menu(),
            KeyCode::F(9) => self.modal = Some(Modal::PullDown { menu_idx: 0, item_idx: 0 }),
            KeyCode::F(10) => return Cmd::quit(),
            KeyCode::F(11) => self.begin_chmod(),
            KeyCode::Tab => {
                self.active = match self.active {
                    ActivePane::Left if !self.hide_right => ActivePane::Right,
                    ActivePane::Right if !self.hide_left => ActivePane::Left,
                    _ => self.active,
                };
            }
            KeyCode::Up => self.active_pane_mut().move_selection(-1, view_height),
            KeyCode::Down => self.active_pane_mut().move_selection(1, view_height),
            KeyCode::PageUp => self.active_pane_mut().move_selection(-(view_height as i32), view_height),
            KeyCode::PageDown => self.active_pane_mut().move_selection(view_height as i32, view_height),
            KeyCode::Left | KeyCode::Backspace => {
                let show_hidden = self.show_hidden;
                if let Err(err) = self.active_pane_mut().go_parent(show_hidden) {
                    self.status = format!("Up failed: {err}");
                }
            }
            KeyCode::Right | KeyCode::Enter => {
                let show_hidden = self.show_hidden;
                match self.active_pane_mut().enter_selected(show_hidden) {
                    Ok(true) => {}
                    Ok(false) => {
                        if matches!(key.code, KeyCode::Enter) {
                            self.open_viewer();
                        }
                    }
                    Err(err) => self.status = format!("Open failed: {err}"),
                }
            }
            KeyCode::Char(' ') | KeyCode::Insert => self.active_pane_mut().toggle_select(),
            KeyCode::F(3) => self.open_viewer(),
            KeyCode::F(4) => self.open_editor(),
            KeyCode::F(5) => self.begin_copy(),
            KeyCode::F(6) => self.begin_move(),
            KeyCode::F(7) => self.begin_mkdir(),
            KeyCode::F(8) => self.begin_delete(),
            KeyCode::Char('q') if key.modifiers.contains(Modifiers::CTRL) => return Cmd::quit(),
            KeyCode::Char('+') => self.active_pane_mut().select_all(),
            KeyCode::Char('-') => self.active_pane_mut().clear_selection(),
            KeyCode::Char('*') => self.active_pane_mut().invert_selection(),
            _ => {}
        }

        Cmd::none()
    }

    fn handle_modal_key(&mut self, key: KeyEvent, mut modal: Modal) -> Cmd<Msg> {
        match &mut modal {
            Modal::Help => {
                if matches!(key.code, KeyCode::Escape | KeyCode::Enter | KeyCode::F(10)) {
                    self.modal = None;
                } else {
                    self.modal = Some(modal);
                }
            }
            Modal::About => {
                if matches!(key.code, KeyCode::Escape | KeyCode::Enter | KeyCode::F(10)) {
                    self.modal = None;
                } else {
                    self.modal = Some(modal);
                }
            }
            Modal::Config { selected, show_hidden } => {
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < 1 {
                            *selected += 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        if *selected == 0 {
                            self.show_hidden = !self.show_hidden;
                            let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                            let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                            *show_hidden = self.show_hidden;
                        }
                        self.modal = Some(modal);
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::PanelOptions { pane, selected, dirs_first, sort_mode } => {
                let count = 2;
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < count {
                            *selected += 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        let target = match pane {
                            ActivePane::Left => &mut self.left,
                            ActivePane::Right => &mut self.right,
                        };
                        match *selected {
                            0 => {
                                target.dirs_first = !target.dirs_first;
                                *dirs_first = target.dirs_first;
                            }
                            1 => {
                                target.sort_mode = match target.sort_mode {
                                    SortMode::NameAsc => SortMode::NameDesc,
                                    SortMode::NameDesc => SortMode::TimeDesc,
                                    SortMode::TimeDesc => SortMode::TimeAsc,
                                    SortMode::TimeAsc => SortMode::NameAsc,
                                };
                                *sort_mode = target.sort_mode;
                            }
                            _ => {}
                        }
                        let _ = target.refresh(RefreshMode::Keep, self.show_hidden);
                        self.modal = Some(modal);
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::UserMenu { items, selected, scroll, config_path } => {
                let view_height = 6usize;
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::F(4) => {
                        let _ = ensure_user_menu_file(config_path);
                        let _ = run_external_editor(
                            &std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string()),
                            config_path,
                        );
                        *self.force_clear_frames.borrow_mut() = 3;
                        let refreshed = load_user_menu(config_path);
                        self.modal = Some(Modal::UserMenu {
                            items: refreshed,
                            selected: 0,
                            scroll: 0,
                            config_path: config_path.clone(),
                        });
                    }
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        if *selected < *scroll {
                            *scroll = *selected;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < items.len() {
                            *selected += 1;
                        }
                        if *selected >= *scroll + view_height {
                            *scroll = selected.saturating_sub(view_height - 1);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        if let Some(item) = items.get(*selected) {
                            self.status = format!("Run: {}", item.command);
                        }
                        self.modal = None;
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::PullDown { menu_idx, item_idx } => {
                match key.code {
                    KeyCode::Escape | KeyCode::F(9) => {
                        self.modal = None;
                    }
                    KeyCode::Left => {
                        if *menu_idx > 0 {
                            *menu_idx -= 1;
                            *item_idx = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Right => {
                        if *menu_idx + 1 < MENU_TITLES.len() {
                            *menu_idx += 1;
                            *item_idx = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        let items = menu_items(*menu_idx);
                        if *item_idx + 1 < items.len() {
                            *item_idx += 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Up => {
                        if *item_idx > 0 {
                            *item_idx -= 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        let items = menu_items(*menu_idx);
                        if let Some(item) = items.get(*item_idx) {
                            match item.action {
                                MenuAction::Quit => return Cmd::quit(),
                                MenuAction::View => self.open_viewer(),
                                MenuAction::Edit => self.open_editor(),
                                MenuAction::Copy => {
                                    self.begin_copy();
                                    return Cmd::none();
                                }
                                MenuAction::Move => {
                                    self.begin_move();
                                    return Cmd::none();
                                }
                                MenuAction::Tree => {
                                    self.open_tree();
                                    return Cmd::none();
                                }
                                MenuAction::Find => {
                                    self.begin_find();
                                    return Cmd::none();
                                }
                                MenuAction::Config => {
                                    self.modal = Some(Modal::Config { selected: 0, show_hidden: self.show_hidden });
                                    return Cmd::none();
                                }
                                MenuAction::PanelOptions => {
                                    self.modal = Some(Modal::PanelOptions {
                                        pane: self.active,
                                        selected: 0,
                                        dirs_first: self.active_pane().dirs_first,
                                        sort_mode: self.active_pane().sort_mode,
                                    });
                                    return Cmd::none();
                                }
                                MenuAction::LeftSortName => {
                                    self.left.sort_mode = toggle_name_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftSortTime => {
                                    self.left.sort_mode = toggle_time_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightSortName => {
                                    self.right.sort_mode = toggle_name_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightSortTime => {
                                    self.right.sort_mode = toggle_time_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::Help => {
                                    self.modal = Some(Modal::Help);
                                    return Cmd::none();
                                }
                                MenuAction::About => {
                                    self.modal = Some(Modal::About);
                                    return Cmd::none();
                                }
                                MenuAction::None => {}
                            }
                        }
                        self.modal = None;
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::Confirm { action, .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.execute_confirm(action.clone());
                        self.modal = None;
                    }
                    KeyCode::Char('n') | KeyCode::Escape => {
                        self.modal = None;
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::Prompt {
                value,
                cursor,
                action,
                ..
            } => {
                match key.code {
                    KeyCode::Escape => {
                        self.modal = None;
                    }
                    KeyCode::Enter => {
                        let input = value.trim().to_string();
                        if !input.is_empty() {
                            self.execute_prompt(action.clone(), input);
                        }
                    }
                    KeyCode::Left => {
                        if *cursor > 0 {
                            *cursor -= 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Right => {
                        if *cursor < value.len() {
                            *cursor += 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Backspace => {
                        if *cursor > 0 {
                            *cursor -= 1;
                            value.remove(*cursor);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Delete => {
                        if *cursor < value.len() {
                            value.remove(*cursor);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Char(ch) => {
                        value.insert(*cursor, ch);
                        *cursor += 1;
                        self.modal = Some(modal);
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::FindResults { items, selected, scroll, .. } => {
                let view_height = 6usize;
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Char('p') if key.modifiers.contains(Modifiers::CTRL) => {
                        let show_hidden = self.show_hidden;
                        let list = items.clone();
                        let pane = self.active_pane_mut();
                        pane.panelized = Some(list);
                        let _ = pane.refresh(RefreshMode::Reset, show_hidden);
                        self.modal = None;
                    }
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        if *selected < *scroll {
                            *scroll = *selected;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < items.len() {
                            *selected += 1;
                        }
                        if *selected >= *scroll + view_height {
                            *scroll = selected.saturating_sub(view_height - 1);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        let show_hidden = self.show_hidden;
                        if let Some(path) = items.get(*selected) {
                            if path.is_dir() {
                                let target = self.active_pane_mut();
                                target.cwd = path.clone();
                                let _ = target.refresh(RefreshMode::Reset, show_hidden);
                                self.modal = None;
                            } else {
                                self.modal = None;
                                self.open_viewer_path(path);
                            }
                        } else {
                            self.modal = Some(modal);
                        }
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::Tree { pane, items, selected, scroll } => {
                let view_height = 8usize;
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        if *selected < *scroll {
                            *scroll = *selected;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < items.len() {
                            *selected += 1;
                        }
                        if *selected >= *scroll + view_height {
                            *scroll = selected.saturating_sub(view_height - 1);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        if let Some(item) = items.get(*selected) {
                            match pane {
                                ActivePane::Left => self.left.cwd = item.path.clone(),
                                ActivePane::Right => self.right.cwd = item.path.clone(),
                            }
                            let _ = match pane {
                                ActivePane::Left => self.left.refresh(RefreshMode::Reset, self.show_hidden),
                                ActivePane::Right => self.right.refresh(RefreshMode::Reset, self.show_hidden),
                            };
                            self.modal = None;
                        } else {
                            self.modal = Some(modal);
                        }
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::DriveMenu { pane, items, selected, scroll } => {
                let view_height = 8usize;
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        if *selected < *scroll {
                            *scroll = *selected;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < items.len() {
                            *selected += 1;
                        }
                        if *selected >= *scroll + view_height {
                            *scroll = selected.saturating_sub(view_height - 1);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        if let Some(path) = items.get(*selected) {
                            match pane {
                                ActivePane::Left => {
                                    self.left.cwd = path.clone();
                                    self.left.vfs = None;
                                    self.left.panelized = None;
                                    let _ = self.left.refresh(RefreshMode::Reset, self.show_hidden);
                                }
                                ActivePane::Right => {
                                    self.right.cwd = path.clone();
                                    self.right.vfs = None;
                                    self.right.panelized = None;
                                    let _ = self.right.refresh(RefreshMode::Reset, self.show_hidden);
                                }
                            }
                        }
                        self.modal = None;
                    }
                    _ => self.modal = Some(modal),
                }
            }
        }
        Cmd::none()
    }

    fn execute_prompt(&mut self, action: PendingPrompt, input: String) {
        let show_hidden = self.show_hidden;
        match action {
            PendingPrompt::CopyTo { sources } => {
                let dest = PathBuf::from(input);
                if let Some(conflicts) = find_conflicts(&sources, &dest) {
                    self.modal = Some(Modal::Confirm {
                        title: "Overwrite".to_string(),
                        message: format!("Overwrite {} item(s)?", conflicts),
                        action: PendingConfirm::Overwrite {
                            kind: OverwriteKind::Copy,
                            sources,
                            dest,
                        },
                    });
                    return;
                }
                match copy_sources(&sources, &dest, false) {
                    Ok(()) => {
                        self.status = "Copy complete".to_string();
                        let _ = self.inactive_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                    }
                    Err(err) => {
                        self.status = format!("Copy failed: {err}");
                    }
                }
            }
            PendingPrompt::MoveTo { sources } => {
                let dest = PathBuf::from(input);
                if let Some(conflicts) = find_conflicts(&sources, &dest) {
                    self.modal = Some(Modal::Confirm {
                        title: "Overwrite".to_string(),
                        message: format!("Overwrite {} item(s)?", conflicts),
                        action: PendingConfirm::Overwrite {
                            kind: OverwriteKind::Move,
                            sources,
                            dest,
                        },
                    });
                    return;
                }
                match move_sources(&sources, &dest, false) {
                    Ok(()) => {
                        self.status = "Move complete".to_string();
                        let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                        let _ = self.inactive_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                    }
                    Err(err) => {
                        self.status = format!("Move failed: {err}");
                    }
                }
            }
            PendingPrompt::Mkdir { base } => {
                let path = base.join(input);
                if let Err(err) = fs::create_dir_all(&path) {
                    self.status = format!("Mkdir failed: {err}");
                    return;
                }
                self.status = format!("Created {}", path.display());
                let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
            }
            PendingPrompt::Find { base } => {
                let results = find_matches(&base, &input, show_hidden);
                if results.is_empty() {
                    self.status = "No matches".to_string();
                    self.modal = None;
                } else {
                    self.modal = Some(Modal::FindResults {
                        query: input,
                        items: results,
                        selected: 0,
                        scroll: 0,
                    });
                }
                return;
            }
            PendingPrompt::Chmod { target } => {
                let trimmed = input.trim_start_matches('0');
                let octal = u32::from_str_radix(trimmed, 8).unwrap_or(0o644);
                let perms = fs::Permissions::from_mode(octal & 0o777);
                if let Err(err) = fs::set_permissions(&target, perms) {
                    self.status = format!("Chmod failed: {err}");
                } else {
                    self.status = format!("Chmod {}", target.display());
                    let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                }
            }
        }
        self.modal = None;
    }

    fn execute_confirm(&mut self, action: PendingConfirm) {
        let show_hidden = self.show_hidden;
        match action {
            PendingConfirm::Delete { sources } => {
                for path in sources {
                    let result = if path.is_dir() {
                        fs::remove_dir_all(&path)
                    } else {
                        fs::remove_file(&path)
                    };
                    if let Err(err) = result {
                        self.status = format!("Delete failed: {err}");
                        return;
                    }
                }
                self.status = "Deleted".to_string();
                let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
            }
            PendingConfirm::Overwrite { kind, sources, dest } => {
                let result = match kind {
                    OverwriteKind::Copy => copy_sources(&sources, &dest, true),
                    OverwriteKind::Move => move_sources(&sources, &dest, true),
                };
                match result {
                    Ok(()) => {
                        self.status = "Operation complete".to_string();
                        let _ = self.active_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                        let _ = self.inactive_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                    }
                    Err(err) => {
                        self.status = format!("Overwrite failed: {err}");
                    }
                }
            }
            PendingConfirm::Sync { ops, src_root, dst_root } => {
                match sync_execute(&ops, &src_root, &dst_root) {
                    Ok(count) => {
                        self.status = format!("Synchronized {}", count);
                        let _ = self.left.refresh(RefreshMode::Keep, show_hidden);
                        let _ = self.right.refresh(RefreshMode::Keep, show_hidden);
                    }
                    Err(err) => {
                        self.status = format!("Sync failed: {err}");
                    }
                }
            }
        }
    }

    fn handle_viewer_key(key: KeyEvent, viewer: &mut Viewer) -> ViewerAction {
        match key.code {
            KeyCode::Escape => return ViewerAction::Close,
            KeyCode::Up => viewer.scroll = viewer.scroll.saturating_sub(1),
            KeyCode::Down => viewer.scroll = viewer.scroll.saturating_add(1),
            KeyCode::PageUp => viewer.scroll = viewer.scroll.saturating_sub(10),
            KeyCode::PageDown => viewer.scroll = viewer.scroll.saturating_add(10),
            KeyCode::F(10) => return ViewerAction::Quit,
            _ => {}
        }
        ViewerAction::None
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.viewer.is_some() || self.modal.is_some() {
            return;
        }
        self.log_event(&format!("mouse {:?} @({}, {})", mouse.kind, mouse.x, mouse.y));
        let layout = match *self.layout.borrow() {
            Some(layout) => layout,
            None => return,
        };
        let show_hidden = self.show_hidden;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let height = self.list_height(self.active);
                self.active_pane_mut().move_selection(-1, height);
            }
            MouseEventKind::ScrollDown => {
                let height = self.list_height(self.active);
                self.active_pane_mut().move_selection(1, height);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((pane, row)) = hit_test_rows(mouse.x, mouse.y, &layout) {
                    self.active = pane;
                    let height = self.list_height(self.active);
                    let double_clicked = self
                        .last_click
                        .as_ref()
                        .map(|last| {
                            last.pane == pane
                                && last.row == row
                                && last.at.elapsed() <= Duration::from_millis(DOUBLE_CLICK_MS)
                        })
                        .unwrap_or(false);
                    let mut opened_dir = false;
                    {
                        let pane_ref = self.active_pane_mut();
                        let offset = pane_ref.state.borrow().offset;
                        let absolute = row.saturating_add(offset);
                        if absolute < pane_ref.entries.len() {
                            let mut state = pane_ref.state.borrow_mut();
                            state.select(Some(absolute));
                            ensure_visible(&mut state, height);
                            drop(state);
                            if double_clicked {
                                opened_dir = pane_ref.enter_selected(show_hidden).unwrap_or(false);
                                self.last_click = None;
                            } else {
                                self.last_click = Some(ClickInfo {
                                    pane,
                                    row: absolute,
                                    at: Instant::now(),
                                });
                            }
                        }
                    }
                    if double_clicked && !opened_dir {
                        self.open_viewer();
                    }
                } else if mouse.y == 0 {
                    self.modal = Some(Modal::PullDown { menu_idx: 0, item_idx: 0 });
                }
            }
            _ => {}
        }
    }

    fn render_viewer(&self, viewer: &Viewer, frame: &mut Frame) {
        let area = Rect::new(0, 0, frame.width(), frame.height());
        let style = Style::new().fg(self.theme.panel_fg).bg(self.theme.panel_bg);
        let paragraph = Paragraph::new(Text::from(viewer.lines.join("\n")))
            .wrap(WrapMode::None)
            .scroll((viewer.scroll as u16, 0))
            .style(style)
            .block(
                Block::bordered()
                    .border_style(Style::new().fg(self.theme.panel_border_active))
                    .borders(Borders::ALL)
                    .title("View"),
            );
        paragraph.render(area, frame);
    }

    fn render(&self, frame: &mut Frame) {
        let full = Rect::new(0, 0, frame.width(), frame.height());
        frame.enable_hit_testing();
        frame.set_cursor(None);
        {
            let mut force_clear = self.force_clear_frames.borrow_mut();
            if *force_clear > 0 {
                frame.clear();
                *force_clear = force_clear.saturating_sub(1);
            }
        }

        let background = Block::new().style(Style::new().fg(self.theme.panel_fg).bg(self.theme.screen_bg));
        background.render(full, frame);

        if let Some(viewer) = &self.viewer {
            self.render_viewer(viewer, frame);
            return;
        }

        let layout = Flex::vertical().constraints([
            Constraint::Fixed(MENU_HEIGHT),
            Constraint::Fill,
            Constraint::Fixed(STATUS_HEIGHT),
            Constraint::Fixed(KEYBAR_HEIGHT),
        ]);
        let areas = layout.split(full);
        let menu_area = areas[0];
        let body_area = areas[1];
        let status_area = areas[2];
        let key_area = areas[3];

        let menu_bg = Block::new().style(Style::new().fg(self.theme.menu_fg).bg(self.theme.menu_bg));
        menu_bg.render(menu_area, frame);
        let menu = Paragraph::new(Text::from(" File  Command  Options  Left  Right  Help "))
            .style(Style::new().fg(self.theme.menu_fg).bg(self.theme.menu_bg));
        menu.render(menu_area, frame);

        if self.hide_all {
            let block = Block::bordered()
                .border_style(Style::new().fg(self.theme.panel_border_active))
                .title("Command line");
            let paragraph = Paragraph::new(Text::from(self.cmdline.clone()))
                .style(Style::new().fg(self.theme.panel_fg).bg(self.theme.panel_bg))
                .block(block);
            paragraph.render(body_area, frame);
            let cursor_x = body_area.x + 1 + self.cmd_cursor as u16;
            let cursor_y = body_area.y + 1;
            frame.set_cursor(Some((cursor_x, cursor_y)));
            *self.layout.borrow_mut() = None;
        } else {
            let mut left_area = Rect::new(0, 0, 0, 0);
            let mut right_area = Rect::new(0, 0, 0, 0);
            if !self.hide_left && !self.hide_right {
                let columns = Flex::horizontal().constraints([
                    Constraint::Ratio(1, 2),
                    Constraint::Ratio(1, 2),
                ]);
                let col_areas = columns.split(body_area);
                left_area = self.render_panel(frame, col_areas[0], &self.left, self.active == ActivePane::Left);
                right_area = self.render_panel(frame, col_areas[1], &self.right, self.active == ActivePane::Right);
            } else if !self.hide_left {
                left_area = self.render_panel(frame, body_area, &self.left, self.active == ActivePane::Left);
            } else if !self.hide_right {
                right_area = self.render_panel(frame, body_area, &self.right, self.active == ActivePane::Right);
            }
            *self.layout.borrow_mut() = Some(LayoutCache {
                left_table: left_area,
                right_table: right_area,
            });
        }

        render_status(frame, status_area, &self.left, &self.right, self.active, &self.status, self.theme);
        render_keybar(frame, key_area, self.theme);

        if let Some(modal) = &self.modal {
            render_modal(frame, modal, self.theme);
        }
    }

    fn render_panel(&self, frame: &mut Frame, area: Rect, pane: &Pane, active: bool) -> Rect {
        let border_color = if active {
            self.theme.panel_border_active
        } else {
            self.theme.panel_border_inactive
        };
        let title = if let Some(vfs) = &pane.vfs {
            if vfs.prefix.is_empty() {
                format!("{}:", vfs.zip_path.display())
            } else {
                format!("{}:{}", vfs.zip_path.display(), vfs.prefix)
            }
        } else if pane.panelized.is_some() {
            "Search results".to_string()
        } else {
            pane.cwd.display().to_string()
        };
        let block = Block::bordered()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border_color))
            .style(Style::new().fg(self.theme.panel_fg).bg(self.theme.panel_bg))
            .title(title.as_str());

        let header = Row::new(["Name", "Size", "Date", "Time"])
            .style(Style::new().fg(self.theme.header_fg).bg(self.theme.header_bg))
            .height(HEADER_HEIGHT);

        let rows = pane
            .entries
            .iter()
            .map(|entry| {
                let is_marked = pane.selected.contains(&entry.path);
                let marker = if is_marked { "*" } else { " " };
                let display_name = if entry.is_dir {
                    entry.name.to_uppercase()
                } else {
                    entry.name.to_lowercase()
                };
                let name = if entry.is_dir {
                    format!("{}<{}>", marker, display_name)
                } else {
                    format!("{}{}", marker, display_name)
                };
                let size = if entry.is_dir {
                    "<DIR>".to_string()
                } else {
                    entry.size.to_string()
                };
                let (date, time) = format_time(entry.modified);
                let mut row = Row::new([name, size, date, time]).height(1);
                if entry.is_system {
                    row = row.style(Style::new().fg(self.theme.system_fg).bg(self.theme.panel_bg));
                }
                if is_marked {
                    row = row.style(Style::new().fg(self.theme.selection_fg).bg(self.theme.panel_bg));
                }
                row
            })
            .collect::<Vec<_>>();

        let widths = [
            Constraint::Fill,
            Constraint::Fixed(6),
            Constraint::Fixed(8),
            Constraint::Fixed(5),
        ];

        let highlight_style = if active {
            Style::new().fg(self.theme.selection_fg).bg(self.theme.selection_bg)
        } else {
            Style::new().fg(self.theme.panel_fg).bg(self.theme.panel_bg)
        };

        let table = Table::new(rows, widths)
            .header(header)
            .block(block)
            .style(Style::new().fg(self.theme.panel_fg).bg(self.theme.panel_bg))
            .highlight_style(highlight_style);

        let mut state = pane.state.borrow_mut();
        StatefulWidget::render(&table, area, frame, &mut state);

        area
    }
}

impl Model for App {
    type Message = Msg;

    fn update(&mut self, msg: Msg) -> Cmd<Msg> {
        match msg {
            Msg::Event(Event::Key(key)) => self.handle_key(key),
            Msg::Event(Event::Mouse(mouse)) => {
                self.handle_mouse(mouse);
                Cmd::none()
            }
            Msg::Event(_) => Cmd::none(),
            Msg::Quit => Cmd::quit(),
        }
    }

    fn view(&self, frame: &mut Frame) {
        self.render(frame);
    }
}

fn ensure_visible(state: &mut TableState, view_height: usize) {
    if view_height == 0 {
        return;
    }
    let Some(selected) = state.selected else {
        return;
    };
    if selected < state.offset {
        state.offset = selected;
    } else if selected >= state.offset + view_height {
        state.offset = selected.saturating_sub(view_height - 1);
    }
}

fn selected_paths(pane: &Pane) -> Vec<PathBuf> {
    if pane.selected.is_empty() {
        return pane.selected_entry().map(|e| e.path.clone()).into_iter().collect();
    }
    pane.entries
        .iter()
        .filter(|e| pane.selected.contains(&e.path))
        .map(|e| e.path.clone())
        .collect()
}

fn render_status(
    frame: &mut Frame,
    area: Rect,
    left: &Pane,
    right: &Pane,
    active: ActivePane,
    status: &str,
    theme: ThemeColors,
) {
    let bg = Block::new().style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    bg.render(area, frame);
    let pane = match active {
        ActivePane::Left => left,
        ActivePane::Right => right,
    };
    let selected_count = if pane.selected.is_empty() { 0 } else { pane.selected.len() };
    let selected_size = pane.selected_total_size();
    let right = format!("Sel: {} Size: {}", selected_count, selected_size);
    let spacing = area.width.saturating_sub(display_width(status) as u16 + right.len() as u16 + 2);
    let line = format!("{}{}{}", status, " ".repeat(spacing as usize), right);
    let paragraph = Paragraph::new(Text::from(line))
        .style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    paragraph.render(area, frame);
}

fn render_keybar(frame: &mut Frame, area: Rect, theme: ThemeColors) {
    let bg = Block::new().style(Style::new().fg(theme.keybar_fg).bg(theme.keybar_bg));
    bg.render(area, frame);
    let items = [
        StatusItem::key_hint("F1", "Help"),
        StatusItem::key_hint("F2", "Menu"),
        StatusItem::key_hint("F3", "View"),
        StatusItem::key_hint("F4", "Edit"),
        StatusItem::key_hint("F5", "Copy"),
        StatusItem::key_hint("F6", "RenMov"),
        StatusItem::key_hint("F7", "Mkdir"),
        StatusItem::key_hint("F8", "Delete"),
        StatusItem::key_hint("F9", "PullDn"),
        StatusItem::key_hint("F10", "Quit"),
    ];
    let mut status = StatusLine::new().style(Style::new().fg(theme.keybar_fg).bg(theme.keybar_bg));
    for item in items {
        status = status.right(item);
    }
    status.render(area, frame);
}

fn render_modal(frame: &mut Frame, modal: &Modal, theme: ThemeColors) {
    let full = Rect::new(0, 0, frame.width(), frame.height());
    let width = full.width.min(60).max(20);
    let height = match modal {
        Modal::Prompt { .. } => 7,
        Modal::Confirm { .. } => 6,
        Modal::FindResults { .. } => 10,
        Modal::Tree { .. } => 12,
        Modal::DriveMenu { .. } => 10,
        Modal::Config { .. } => 6,
        Modal::PanelOptions { .. } => 7,
        Modal::UserMenu { .. } => 10,
        Modal::About => 6,
        Modal::Help => 10,
        Modal::PullDown { .. } => 8,
    };
    let x = full.x + (full.width.saturating_sub(width)) / 2;
    let y = full.y + (full.height.saturating_sub(height)) / 2;
    let area = Rect::new(x, y, width, height);
    let style = Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let fill = Block::new().style(style);
    fill.render(area, frame);
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.panel_border_active))
        .style(style);

    match modal {
        Modal::Prompt { title, label, value, cursor, .. } => {
            let text = format!("{}\n\n{}\n{}", title, label, value);
            let paragraph = Paragraph::new(Text::from(text)).style(style).block(block);
            paragraph.render(area, frame);
            let cursor_x = area.x + 1 + *cursor as u16;
            let cursor_y = area.y + 1 + 3;
            frame.set_cursor(Some((cursor_x, cursor_y)));
        }
        Modal::Confirm { title, message, .. } => {
            let text = format!("{}\n\n{}\n\nY=Yes  N=No", title, message);
            let paragraph = Paragraph::new(Text::from(text)).style(style).block(block);
            paragraph.render(area, frame);
        }
        Modal::FindResults { query, items, selected, scroll } => {
            let mut lines = vec![format!("Find results: {}", query)];
            let view_height = (area.height.saturating_sub(2)) as usize;
            let start = *scroll;
            let end = (*scroll + view_height).min(items.len());
            for (idx, path) in items.iter().enumerate().take(end).skip(start) {
                let marker = if idx == *selected { ">" } else { " " };
                lines.push(format!("{} {}", marker, path.display()));
            }
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::Tree { items, selected, scroll, .. } => {
            let mut lines = vec!["Directory tree".to_string()];
            let view_height = (area.height.saturating_sub(2)) as usize;
            let start = *scroll;
            let end = (*scroll + view_height).min(items.len());
            for (idx, item) in items.iter().enumerate().take(end).skip(start) {
                let marker = if idx == *selected { ">" } else { " " };
                let indent = "  ".repeat(item.depth);
                let name = item
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| item.path.display().to_string());
                lines.push(format!("{} {}{}", marker, indent, name));
            }
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::DriveMenu { pane, items, selected, scroll } => {
            let target = match pane {
                ActivePane::Left => "Left drive",
                ActivePane::Right => "Right drive",
            };
            let mut lines = vec![target.to_string()];
            let view_height = (area.height.saturating_sub(2)) as usize;
            let start = *scroll;
            let end = (*scroll + view_height).min(items.len());
            for (idx, path) in items.iter().enumerate().take(end).skip(start) {
                let marker = if idx == *selected { ">" } else { " " };
                lines.push(format!("{} {}", marker, path.display()));
            }
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::Config { selected, show_hidden } => {
            let mut lines = vec!["Configuration".to_string()];
            let marker = if *selected == 0 { ">" } else { " " };
            lines.push(format!(
                "{} Show hidden: {}",
                marker,
                if *show_hidden { "On" } else { "Off" }
            ));
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::PanelOptions { pane, selected, dirs_first, sort_mode } => {
            let target = match pane {
                ActivePane::Left => "Left",
                ActivePane::Right => "Right",
            };
            let mut lines = vec![format!("{} panel options", target)];
            let marker0 = if *selected == 0 { ">" } else { " " };
            let marker1 = if *selected == 1 { ">" } else { " " };
            lines.push(format!(
                "{} Dirs first: {}",
                marker0,
                if *dirs_first { "On" } else { "Off" }
            ));
            lines.push(format!(
                "{} Sort: {}",
                marker1,
                sort_label(*sort_mode)
            ));
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::UserMenu { items, selected, scroll, .. } => {
            let mut lines = vec!["User menu".to_string()];
            let view_height = (area.height.saturating_sub(2)) as usize;
            let start = *scroll;
            let end = (*scroll + view_height).min(items.len());
            for (idx, item) in items.iter().enumerate().take(end).skip(start) {
                let marker = if idx == *selected { ">" } else { " " };
                lines.push(format!("{} {}", marker, item.label));
            }
            lines.push(String::from("\nF4 Edit"));
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
        Modal::About => {
            let text = "FrankenCommander\n\nBuilt with FrankenTUI\n2026";
            let paragraph = Paragraph::new(Text::from(text)).style(style).block(block);
            paragraph.render(area, frame);
        }
        Modal::Help => {
            let text = "Help\n\nF3 View  F4 Edit  F5 Copy  F6 Move\nF7 Mkdir  F8 Delete  F10 Quit  F11 Attr\nTab Switch panes  Ins/Space Select\nAlt+F1/F2 Drives  Ctrl+F1/F2 Hide panels\nCtrl+O Command line  Ctrl+F8 Sync dirs\nNum+/Num-/Num* select all/clear/invert";
            let paragraph = Paragraph::new(Text::from(text)).style(style).block(block);
            paragraph.render(area, frame);
        }
        Modal::PullDown { menu_idx, item_idx } => {
            let items = menu_items(*menu_idx);
            let mut lines = vec![format!("{}", MENU_TITLES[*menu_idx])];
            for (idx, item) in items.iter().enumerate() {
                let marker = if idx == *item_idx { ">" } else { " " };
                lines.push(format!("{} {}", marker, item.label));
            }
            let paragraph = Paragraph::new(Text::from(lines.join("\n")))
                .style(style)
                .block(block);
            paragraph.render(area, frame);
        }
    }
}

fn menu_items(menu_idx: usize) -> &'static [MenuItem] {
    match menu_idx {
        0 => &[
            MenuItem { label: "View", action: MenuAction::View },
            MenuItem { label: "Edit", action: MenuAction::Edit },
            MenuItem { label: "Copy", action: MenuAction::Copy },
            MenuItem { label: "Move", action: MenuAction::Move },
            MenuItem { label: "Quit", action: MenuAction::Quit },
        ],
        1 => &[
            MenuItem { label: "Directory tree", action: MenuAction::Tree },
            MenuItem { label: "Find file", action: MenuAction::Find },
        ],
        2 => &[
            MenuItem { label: "Configuration", action: MenuAction::Config },
            MenuItem { label: "Panel options", action: MenuAction::PanelOptions },
        ],
        3 => &[
            MenuItem { label: "Sort name", action: MenuAction::LeftSortName },
            MenuItem { label: "Sort time", action: MenuAction::LeftSortTime },
        ],
        4 => &[
            MenuItem { label: "Sort name", action: MenuAction::RightSortName },
            MenuItem { label: "Sort time", action: MenuAction::RightSortTime },
        ],
        _ => &[
            MenuItem { label: "Help", action: MenuAction::Help },
            MenuItem { label: "About", action: MenuAction::About },
        ],
    }
}

fn hit_test_rows(x: u16, y: u16, layout: &LayoutCache) -> Option<(ActivePane, usize)> {
    if layout.left_table.contains(x, y) {
        let row = (y - layout.left_table.y) as usize;
        return Some((ActivePane::Left, row.saturating_sub(HEADER_HEIGHT as usize)));
    }
    if layout.right_table.contains(x, y) {
        let row = (y - layout.right_table.y) as usize;
        return Some((ActivePane::Right, row.saturating_sub(HEADER_HEIGHT as usize)));
    }
    None
}

fn format_time(time: Option<SystemTime>) -> (String, String) {
    let Some(time) = time else {
        return ("".to_string(), "".to_string());
    };
    let date_fmt = time::format_description::parse("[day]-[month]-[year repr:last_two]").unwrap();
    let time_fmt = time::format_description::parse("[hour]:[minute]").unwrap();
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let dt = OffsetDateTime::from(time).to_offset(offset);
    let date = dt.format(&date_fmt).unwrap_or_default();
    let clock = dt.format(&time_fmt).unwrap_or_default();
    (date, clock)
}

fn read_entries(
    dir: &Path,
    sort_mode: SortMode,
    dirs_first: bool,
    show_hidden: bool,
) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let metadata = item.metadata()?;
        let is_dir = metadata.is_dir();
        let size = metadata.len();
        let modified = metadata.modified().ok();
        let name = item.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let is_system = name.starts_with('.');
        entries.push(Entry {
            name,
            path,
            is_dir,
            size,
            modified,
            is_system,
        });
    }

    entries.sort_by(|a, b| {
        if dirs_first && a.is_dir != b.is_dir {
            return if a.is_dir { Ordering::Less } else { Ordering::Greater };
        }
        match sort_mode {
            SortMode::NameAsc => cmp_name(a, b),
            SortMode::NameDesc => cmp_name(b, a),
            SortMode::TimeAsc => cmp_time(a, b).then_with(|| cmp_name(a, b)),
            SortMode::TimeDesc => cmp_time(b, a).then_with(|| cmp_name(a, b)),
        }
    });

    Ok(entries)
}

fn read_panelized(paths: &[PathBuf]) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for path in paths {
        let Ok(metadata) = fs::metadata(path) else { continue };
        let is_dir = metadata.is_dir();
        let size = metadata.len();
        let modified = metadata.modified().ok();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let is_system = name.starts_with('.');
        entries.push(Entry {
            name,
            path: path.clone(),
            is_dir,
            size,
            modified,
            is_system,
        });
    }
    Ok(entries)
}

fn read_zip_entries(vfs: &VfsState, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let file = fs::File::open(&vfs.zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let prefix = vfs.prefix.as_str();
    let mut entries = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name().to_string();
        if !name.starts_with(prefix) {
            continue;
        }
        let rest = &name[prefix.len()..];
        if rest.is_empty() {
            continue;
        }
        let parts: Vec<&str> = rest.split('/').collect();
        let is_dir = file.is_dir() || rest.ends_with('/');
        if parts.len() > 1 {
            let dir_name = parts[0].to_string();
            if !show_hidden && dir_name.starts_with('.') {
                continue;
            }
            if seen_dirs.insert(dir_name.clone()) {
                let path = PathBuf::from(dir_name.clone());
                let is_system = dir_name.starts_with('.');
                entries.push(Entry {
                    name: dir_name,
                    path,
                    is_dir: true,
                    size: 0,
                    modified: None,
                    is_system,
                });
            }
            continue;
        }
        let base = parts[0].to_string();
        if base.is_empty() {
            continue;
        }
        if !show_hidden && base.starts_with('.') {
            continue;
        }
        let is_system = base.starts_with('.');
        entries.push(Entry {
            name: base.clone(),
            path: PathBuf::from(base),
            is_dir,
            size: file.size(),
            modified: None,
            is_system,
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

fn read_zip_file_lines(vfs: &VfsState, entry_path: &Path) -> io::Result<Vec<String>> {
    let file = fs::File::open(&vfs.zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let full = format!("{}{}", vfs.prefix, entry_path.to_string_lossy());
    let mut zip_file = archive.by_name(&full)?;
    let mut data = Vec::new();
    zip_file.read_to_end(&mut data)?;
    let content = String::from_utf8_lossy(&data);
    Ok(content.lines().map(|line| line.to_string()).collect())
}

fn zip_parent_prefix(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end_matches('/');
    let parent = Path::new(trimmed).parent()?.to_string_lossy().to_string();
    if parent.is_empty() {
        Some(String::new())
    } else {
        Some(format!("{}/", parent))
    }
}

fn zip_child_prefix(prefix: &str, entry_path: &Path) -> String {
    let child = entry_path.to_string_lossy();
    format!("{}{}/", prefix, child)
}

fn cmp_name(a: &Entry, b: &Entry) -> Ordering {
    a.name.to_lowercase().cmp(&b.name.to_lowercase())
}

fn cmp_time(a: &Entry, b: &Entry) -> Ordering {
    let a_time = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    let b_time = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    a_time.cmp(&b_time)
}

fn toggle_name_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::NameAsc => SortMode::NameDesc,
        SortMode::NameDesc => SortMode::NameAsc,
        SortMode::TimeAsc | SortMode::TimeDesc => SortMode::NameAsc,
    }
}

fn toggle_time_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::TimeAsc => SortMode::TimeDesc,
        SortMode::TimeDesc => SortMode::TimeAsc,
        SortMode::NameAsc | SortMode::NameDesc => SortMode::TimeDesc,
    }
}

fn sort_label(mode: SortMode) -> &'static str {
    match mode {
        SortMode::NameAsc => "Name Asc",
        SortMode::NameDesc => "Name Desc",
        SortMode::TimeAsc => "Time Asc",
        SortMode::TimeDesc => "Time Desc",
    }
}

fn find_matches(base: &Path, query: &str, show_hidden: bool) -> Vec<PathBuf> {
    let query = query.to_lowercase();
    let mut results = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            if name.to_lowercase().contains(&query) {
                results.push(path.clone());
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    results
}

fn build_tree(base: &Path, max_depth: usize, show_hidden: bool) -> Vec<TreeItem> {
    let mut items = Vec::new();
    let mut stack = vec![(base.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        items.push(TreeItem { path: dir.clone(), depth });
        if depth >= max_depth {
            continue;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        let mut children = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if !show_hidden && name.starts_with('.') {
                        continue;
                    }
                }
                children.push(path);
            }
        }
        children.sort();
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    items
}

fn list_drive_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(PathBuf::from("/"));
    roots.push(PathBuf::from("/home"));
    roots.push(PathBuf::from("/tmp"));
    for base in ["/mnt", "/media"] {
        if let Ok(read) = fs::read_dir(base) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    roots.push(path);
                }
            }
        }
    }
    roots.sort();
    roots
}

fn user_menu_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/nuc".to_string());
    Path::new(&home).join(".frankencommander").join("usermenu.txt")
}

fn ensure_user_menu_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let sample = "List|ls -la\nEdit config|$EDITOR ~/.frankencommander/usermenu.txt\n";
        fs::write(path, sample)?;
    }
    Ok(())
}

fn load_user_menu(path: &Path) -> Vec<UserMenuItem> {
    let mut items = Vec::new();
    let Ok(content) = fs::read_to_string(path) else { return items };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let label = parts.next().unwrap_or("").trim().to_string();
        let command = parts.next().unwrap_or("").trim().to_string();
        if !label.is_empty() {
            items.push(UserMenuItem { label, command });
        }
    }
    items
}

fn sync_plan(src: &Path, dst: &Path) -> Vec<PathBuf> {
    let mut ops = Vec::new();
    let mut stack = vec![src.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(read) = fs::read_dir(&path) else { continue };
        for entry in read.flatten() {
            let src_path = entry.path();
            let rel = src_path.strip_prefix(src).unwrap_or(&src_path);
            let dst_path = dst.join(rel);
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(src_path.clone());
                if !dst_path.exists() {
                    ops.push(src_path);
                }
            } else {
                let mut should_copy = !dst_path.exists();
                if let Ok(dst_meta) = fs::metadata(&dst_path) {
                    if let (Ok(src_m), Ok(dst_m)) = (meta.modified(), dst_meta.modified()) {
                        if src_m > dst_m {
                            should_copy = true;
                        }
                    }
                }
                if should_copy {
                    ops.push(src_path);
                }
            }
        }
    }
    ops
}

fn sync_execute(ops: &[PathBuf], src_root: &Path, dst_root: &Path) -> io::Result<usize> {
    let mut count = 0;
    for src in ops {
        let rel = src.strip_prefix(src_root).unwrap_or(src);
        let target = dst_root.join(rel);
        copy_entry(src, &target)?;
        count += 1;
    }
    Ok(count)
}

fn read_file_lines(path: &Path) -> io::Result<Vec<String>> {
    let data = fs::read(path)?;
    let content = String::from_utf8_lossy(&data);
    Ok(content.lines().map(|line| line.to_string()).collect())
}

fn find_conflicts(sources: &[PathBuf], dest: &Path) -> Option<usize> {
    let dest_is_dir = dest.is_dir() || sources.len() > 1;
    let mut conflicts = 0;
    for src in sources {
        let target = if dest_is_dir {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest.to_path_buf()
        };
        if target.exists() {
            conflicts += 1;
        }
    }
    if conflicts > 0 { Some(conflicts) } else { None }
}

fn remove_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_sources(sources: &[PathBuf], dest: &Path, overwrite: bool) -> io::Result<()> {
    let dest_is_dir = dest.is_dir() || sources.len() > 1;
    for src in sources {
        let target = if dest_is_dir {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest.to_path_buf()
        };
        if overwrite && target.exists() {
            remove_path(&target)?;
        }
        copy_entry(src, &target)?;
    }
    Ok(())
}

fn move_sources(sources: &[PathBuf], dest: &Path, overwrite: bool) -> io::Result<()> {
    let dest_is_dir = dest.is_dir() || sources.len() > 1;
    for src in sources {
        let target = if dest_is_dir {
            dest.join(src.file_name().unwrap_or_default())
        } else {
            dest.to_path_buf()
        };
        if overwrite && target.exists() {
            remove_path(&target)?;
        }
        move_entry(src, &target)?;
    }
    Ok(())
}

fn copy_entry(src: &Path, dest: &Path) -> io::Result<()> {
    if src.is_dir() {
        copy_dir_recursive(src, dest)
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dest)?;
        Ok(())
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    if !dest.exists() {
        fs::create_dir_all(dest)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn move_entry(src: &Path, dest: &Path) -> io::Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_entry(src, dest)?;
            if src.is_dir() {
                fs::remove_dir_all(src)
            } else {
                fs::remove_file(src)
            }
        }
    }
}

fn run_external_editor(editor: &str, path: &Path) -> io::Result<()> {
    let mut stdout = std::io::stdout();
    crossterm::terminal::disable_raw_mode().ok();
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    let status = std::process::Command::new(editor).arg(path).status();
    execute!(
        stdout,
        EnterAlternateScreen,
        Clear(ClearType::All),
        MoveTo(0, 0),
        EnableMouseCapture
    )?;
    crossterm::terminal::enable_raw_mode().ok();
    while event::poll(Duration::from_millis(0))? {
        let _ = event::read();
    }
    status.map(|_| ())
}
