#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::{
    cursor::MoveTo,
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ftui::prelude::*;
use ftui::render::cell::PackedRgba;
use ftui::render::diff_strategy::DiffStrategyConfig;
use ftui::{KeyEventKind, MouseButton, MouseEvent, MouseEventKind, Program, ProgramConfig, RuntimeDiffConfig};
use ftui::render::budget::FrameBudgetConfig;
use time::OffsetDateTime;

use crate::fs_ops::{
    build_tree, copy_sources, find_conflicts, find_matches, list_drive_roots, load_user_menu,
    move_sources, read_file_lines, sync_execute, sync_plan, toggle_name_sort, toggle_time_sort,
    user_menu_path, ensure_user_menu_file,
};
use crate::menu::{menu_items, MENU_TITLES};
use crate::model::{
    ActivePane, ClickInfo, LayoutCache, MenuAction, Modal, OverwriteKind, Pane, PendingConfirm,
    PendingPrompt, RefreshMode, SortMode, Viewer, ViewerAction, VfsState,
};
use crate::ui::{
    render_background, render_layout, render_modal_wrapper, render_status_and_keybar, render_viewer,
};
use crate::vfs::read_zip_file_lines;

const DOUBLE_CLICK_MS: u64 = 400;

#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub screen_bg: PackedRgba,
    pub menu_bg: PackedRgba,
    pub menu_fg: PackedRgba,
    pub panel_bg: PackedRgba,
    pub panel_fg: PackedRgba,
    pub system_fg: PackedRgba,
    pub panel_border_active: PackedRgba,
    pub panel_border_inactive: PackedRgba,
    pub header_bg: PackedRgba,
    pub header_fg: PackedRgba,
    pub selection_bg: PackedRgba,
    pub selection_fg: PackedRgba,
    pub keybar_bg: PackedRgba,
    pub keybar_fg: PackedRgba,
    pub status_bg: PackedRgba,
    pub status_fg: PackedRgba,
    pub dialog_bg: PackedRgba,
    pub dialog_fg: PackedRgba,
}

impl ThemeColors {
    pub fn classic() -> Self {
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
pub enum Msg {
    Event(Event),
    Quit,
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        Msg::Event(event)
    }
}

#[derive(Debug)]
pub struct App {
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

impl App {
    pub fn new() -> io::Result<Self> {
        let cwd = std::env::current_dir()?;
        let mut left = Pane::new(cwd.clone());
        let mut right = Pane::new(cwd);
        left.refresh(RefreshMode::Reset, false)?;
        right.refresh(RefreshMode::Reset, false)?;

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

    pub fn run() -> io::Result<()> {
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
        let Some(layout) = layout.as_ref() else { return 0 };
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
                self.viewer = Some(Viewer { path: path.to_path_buf(), lines, scroll: 0 });
            }
            Err(err) => {
                self.status = format!("View failed: {err}");
            }
        }
    }

    fn open_zip_viewer(&mut self, vfs: &VfsState, entry_path: &Path) {
        match read_zip_file_lines(vfs, entry_path) {
            Ok(lines) => {
                self.viewer = Some(Viewer { path: entry_path.to_path_buf(), lines, scroll: 0 });
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
            dest_dir
                .join(
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
        self.modal = Some(Modal::Prompt {
            title: "Find file".to_string(),
            label: "Search:".to_string(),
            value: String::new(),
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
        self.modal = Some(Modal::Tree { pane, items, selected: 0, scroll: 0 });
    }

    fn open_drive_menu(&mut self, pane: ActivePane) {
        let items = list_drive_roots();
        self.modal = Some(Modal::DriveMenu { pane, items, selected: 0, scroll: 0 });
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
            action: PendingConfirm::Sync { ops, src_root: src, dst_root: dst },
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
            action: PendingPrompt::Chmod { target: entry.path.clone() },
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
                action = handle_viewer_key(key, viewer);
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
            Modal::Prompt { value, cursor, action, .. } => {
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

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.viewer.is_some() || self.modal.is_some() || self.hide_all {
            return;
        }
        self.log_event(&format!("mouse {:?} @({}, {})", mouse.kind, mouse.x, mouse.y));
        let layout = {
            let layout_ref = self.layout.borrow();
            match layout_ref.as_ref() {
                Some(layout) => layout.clone(),
                None => return,
            }
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
                                self.last_click = Some(ClickInfo { pane, row: absolute, at: Instant::now() });
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

    fn render(&self, frame: &mut Frame) {
        frame.enable_hit_testing();
        frame.set_cursor(None);
        {
            let mut force_clear = self.force_clear_frames.borrow_mut();
            if *force_clear > 0 {
                frame.clear();
                *force_clear = force_clear.saturating_sub(1);
            }
        }

        render_background(frame, self.theme);

        if let Some(viewer) = &self.viewer {
            render_viewer(viewer, frame, self.theme);
            return;
        }

        let (layout_cache, status_area, key_area) = render_layout(
            frame,
            self.theme,
            &self.left,
            &self.right,
            self.active,
            self.hide_left,
            self.hide_right,
            self.hide_all,
            &self.cmdline,
            self.cmd_cursor,
        );

        *self.layout.borrow_mut() = layout_cache;

        render_status_and_keybar(
            frame,
            status_area,
            key_area,
            self.theme,
            &self.left,
            &self.right,
            self.active,
            &self.status,
        );

        if let Some(modal) = &self.modal {
            render_modal_wrapper(frame, modal, self.theme, &self.left, &self.right);
        }
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

pub fn ensure_visible(state: &mut ftui::widgets::table::TableState, view_height: usize) {
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

pub fn selected_paths(pane: &Pane) -> Vec<PathBuf> {
    if pane.selected.is_empty() {
        return pane.selected_entry().map(|e| e.path.clone()).into_iter().collect();
    }
    pane
        .entries
        .iter()
        .filter(|e| pane.selected.contains(&e.path))
        .map(|e| e.path.clone())
        .collect()
}

pub fn handle_viewer_key(key: KeyEvent, viewer: &mut Viewer) -> ViewerAction {
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

pub fn hit_test_rows(x: u16, y: u16, layout: &LayoutCache) -> Option<(ActivePane, usize)> {
    if layout.left_table.contains(x, y) {
        let row = (y - layout.left_table.y) as usize;
        return Some((ActivePane::Left, row.saturating_sub(crate::ui::HEADER_HEIGHT as usize)));
    }
    if layout.right_table.contains(x, y) {
        let row = (y - layout.right_table.y) as usize;
        return Some((ActivePane::Right, row.saturating_sub(crate::ui::HEADER_HEIGHT as usize)));
    }
    None
}

pub fn run_external_editor(editor: &str, path: &Path) -> io::Result<()> {
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
