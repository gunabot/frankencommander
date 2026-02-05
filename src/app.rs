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
    move_sources, read_file_lines, sync_execute, sync_plan, toggle_ext_sort, toggle_name_sort,
    toggle_size_sort, toggle_time_sort, user_menu_path, ensure_user_menu_file,
};
use crate::menu::{menu_items, MENU_TITLES};
use crate::model::{
    ActivePane, ClickInfo, CopyDialogFocus, CopyDialogState, LayoutCache, MenuAction, Modal,
    OverwriteKind, Pane, PanelMode, PendingConfirm, PendingPrompt, RefreshMode, SortMode, Viewer,
    ViewerAction, VfsState,
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
            // Approximate VGA 16-color palette (NC 5.0 era)
            screen_bg: PackedRgba::rgb(0, 0, 170),        // dark blue
            menu_bg: PackedRgba::rgb(0, 170, 170),        // cyan
            menu_fg: PackedRgba::rgb(255, 255, 255),      // bright white
            panel_bg: PackedRgba::rgb(0, 0, 170),         // dark blue
            panel_fg: PackedRgba::rgb(170, 170, 170),     // light gray
            system_fg: PackedRgba::rgb(85, 85, 85),       // dark gray
            panel_border_active: PackedRgba::rgb(85, 255, 255), // bright cyan
            panel_border_inactive: PackedRgba::rgb(85, 85, 85), // dark gray
            header_bg: PackedRgba::rgb(0, 0, 170),
            header_fg: PackedRgba::rgb(255, 255, 255),
            selection_bg: PackedRgba::rgb(170, 170, 0),   // yellow
            selection_fg: PackedRgba::rgb(0, 0, 0),       // black
            keybar_bg: PackedRgba::rgb(0, 170, 170),
            keybar_fg: PackedRgba::rgb(255, 255, 255),
            status_bg: PackedRgba::rgb(0, 0, 170),
            status_fg: PackedRgba::rgb(255, 255, 255),
            dialog_bg: PackedRgba::rgb(170, 170, 170),    // light gray
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
    quick_search: Option<String>,
    quick_search_time: Option<Instant>,
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
            quick_search: None,
            quick_search_time: None,
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
        let source_name = if sources.len() == 1 {
            self.active_pane()
                .selected_entry()
                .map(|e| e.name.clone())
                .unwrap_or_default()
        } else {
            format!("{} files", sources.len())
        };
        let dest_dir = self.inactive_pane_mut().cwd.clone();
        let dest = if sources.len() == 1 {
            dest_dir
                .join(&source_name)
                .display()
                .to_string()
        } else {
            dest_dir.display().to_string()
        };
        self.modal = Some(Modal::CopyDialog(CopyDialogState {
            sources,
            source_name,
            dest: dest.clone(),
            cursor: dest.len(),
            include_subdirs: false,
            copy_newer_only: false,
            use_filters: false,
            check_target_space: false,
            focus: CopyDialogFocus::Input,
        }));
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
        let source_name = if sources.len() == 1 {
            sources[0].file_name().unwrap_or_default().to_string_lossy().to_string()
        } else {
            format!("{} files", sources.len())
        };
        let dest_dir = self.inactive_pane_mut().cwd.clone();
        let dest = if sources.len() == 1 {
            dest_dir
                .join(&source_name)
                .display()
                .to_string()
        } else {
            dest_dir.display().to_string()
        };
        self.modal = Some(Modal::MoveDialog(CopyDialogState {
            sources,
            source_name,
            dest: dest.clone(),
            cursor: dest.len(),
            include_subdirs: false,
            copy_newer_only: false,
            use_filters: false,
            check_target_space: false,
            focus: CopyDialogFocus::Input,
        }));
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
        let source_name = if sources.len() == 1 {
            self.active_pane()
                .selected_entry()
                .map(|e| e.name.clone())
                .unwrap_or_default()
        } else {
            format!("{} files", sources.len())
        };
        self.modal = Some(Modal::DeleteDialog {
            sources,
            source_name,
            use_filters: false,
            focus: 1, // Focus on Delete button
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
            // Panel mode switching (Ctrl+1 Brief, Ctrl+2 Full, Ctrl+3 Info, Ctrl+4 QuickView)
            KeyCode::Char('1') if key.modifiers.contains(Modifiers::CTRL) => {
                self.active_pane_mut().mode = PanelMode::Brief;
            }
            KeyCode::Char('2') if key.modifiers.contains(Modifiers::CTRL) => {
                self.active_pane_mut().mode = PanelMode::Full;
            }
            KeyCode::Char('3') if key.modifiers.contains(Modifiers::CTRL) => {
                self.active_pane_mut().mode = PanelMode::Info;
            }
            KeyCode::Char('4') if key.modifiers.contains(Modifiers::CTRL) => {
                self.active_pane_mut().mode = PanelMode::QuickView;
            }
            KeyCode::F(1) => self.modal = Some(Modal::Help { page: 0, scroll: 0 }),
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
            KeyCode::Left => {
                let show_hidden = self.show_hidden;
                if let Err(err) = self.active_pane_mut().go_parent(show_hidden) {
                    self.status = format!("Up failed: {err}");
                }
            }
            KeyCode::Backspace => {
                // If quick search is active, remove last character
                if let Some(ref mut qs) = self.quick_search {
                    qs.pop();
                    if qs.is_empty() {
                        self.quick_search = None;
                        self.quick_search_time = None;
                        self.status = "Ready".to_string();
                    } else {
                        self.quick_search_time = Some(Instant::now());
                        self.status = format!("Quick search: {}", qs);
                        self.do_quick_search();
                    }
                } else {
                    // Go to parent directory
                    let show_hidden = self.show_hidden;
                    if let Err(err) = self.active_pane_mut().go_parent(show_hidden) {
                        self.status = format!("Up failed: {err}");
                    }
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
            KeyCode::Escape => {
                // Clear quick search on Escape
                if self.quick_search.is_some() {
                    self.quick_search = None;
                    self.quick_search_time = None;
                    self.status = "Ready".to_string();
                }
            }
            KeyCode::Char(ch) if ch.is_alphanumeric() || ch == '.' || ch == '_' => {
                // Quick search: typing characters jumps to matching file
                if !key.modifiers.contains(Modifiers::CTRL) && !key.modifiers.contains(Modifiers::ALT) {
                    self.handle_quick_search_char(ch);
                }
            }
            _ => {}
        }

        Cmd::none()
    }

    fn handle_quick_search_char(&mut self, ch: char) {
        const QUICK_SEARCH_TIMEOUT_MS: u64 = 1500;

        // Reset search if too much time has passed
        if let Some(last_time) = self.quick_search_time {
            if last_time.elapsed() > Duration::from_millis(QUICK_SEARCH_TIMEOUT_MS) {
                self.quick_search = None;
            }
        }

        // Append character to search string
        let search = self.quick_search.get_or_insert_with(String::new);
        search.push(ch.to_ascii_lowercase());
        self.quick_search_time = Some(Instant::now());
        self.status = format!("Quick search: {}", search);

        self.do_quick_search();
    }

    fn do_quick_search(&mut self) {
        let search = match &self.quick_search {
            Some(s) => s.clone(),
            None => return,
        };
        let view_height = self.list_height(self.active);
        let pane = self.active_pane_mut();

        // Find first entry starting with the search string
        for (idx, entry) in pane.entries.iter().enumerate() {
            if entry.name.to_lowercase().starts_with(&search) {
                let mut state = pane.state.borrow_mut();
                state.select(Some(idx));
                ensure_visible(&mut state, view_height);
                return;
            }
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent, mut modal: Modal) -> Cmd<Msg> {
        match &mut modal {
            Modal::Help { page, scroll } => {
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Left => {
                        if *page > 0 {
                            *page -= 1;
                            *scroll = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Right => {
                        if *page < 3 {
                            *page += 1;
                            *scroll = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Up => {
                        *scroll = scroll.saturating_sub(1);
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        *scroll += 1;
                        self.modal = Some(modal);
                    }
                    KeyCode::PageUp => {
                        *scroll = scroll.saturating_sub(5);
                        self.modal = Some(modal);
                    }
                    KeyCode::PageDown => {
                        *scroll += 5;
                        self.modal = Some(modal);
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::About => {
                if matches!(key.code, KeyCode::Escape | KeyCode::Enter | KeyCode::F(10)) {
                    self.modal = None;
                } else {
                    self.modal = Some(modal);
                }
            }
            Modal::Config { page, selected, show_hidden, auto_save, confirm_delete, confirm_overwrite } => {
                let items_per_page = match *page {
                    0 => 1,  // Screen page: show_hidden
                    1 => 2,  // Confirmations: confirm_delete, confirm_overwrite
                    _ => 1,
                };
                match key.code {
                    KeyCode::Escape | KeyCode::F(10) => self.modal = None,
                    KeyCode::Left => {
                        if *page > 0 {
                            *page -= 1;
                            *selected = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Right => {
                        if *page < 2 {
                            *page += 1;
                            *selected = 0;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Up => {
                        if *selected > 0 {
                            *selected -= 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if *selected + 1 < items_per_page {
                            *selected += 1;
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        match (*page, *selected) {
                            (0, 0) => {
                                self.show_hidden = !self.show_hidden;
                                *show_hidden = self.show_hidden;
                                let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                            }
                            (1, 0) => *confirm_delete = !*confirm_delete,
                            (1, 1) => *confirm_overwrite = !*confirm_overwrite,
                            (2, 0) => *auto_save = !*auto_save,
                            _ => {}
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
                                    SortMode::NameDesc => SortMode::ExtAsc,
                                    SortMode::ExtAsc => SortMode::ExtDesc,
                                    SortMode::ExtDesc => SortMode::TimeAsc,
                                    SortMode::TimeAsc => SortMode::TimeDesc,
                                    SortMode::TimeDesc => SortMode::SizeAsc,
                                    SortMode::SizeAsc => SortMode::SizeDesc,
                                    SortMode::SizeDesc => SortMode::Unsorted,
                                    SortMode::Unsorted => SortMode::NameAsc,
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
                                    self.modal = Some(Modal::Config {
                                        page: 0,
                                        selected: 0,
                                        show_hidden: self.show_hidden,
                                        auto_save: false,
                                        confirm_delete: true,
                                        confirm_overwrite: true,
                                    });
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
                                // Left panel view modes
                                MenuAction::LeftBrief => {
                                    self.left.mode = PanelMode::Brief;
                                }
                                MenuAction::LeftFull => {
                                    self.left.mode = PanelMode::Full;
                                }
                                MenuAction::LeftInfo => {
                                    self.left.mode = PanelMode::Info;
                                }
                                MenuAction::LeftTree => {
                                    self.left.mode = PanelMode::Tree;
                                }
                                MenuAction::LeftQuickView => {
                                    self.left.mode = PanelMode::QuickView;
                                }
                                MenuAction::LeftOnOff => {
                                    self.hide_left = !self.hide_left;
                                    if self.hide_left && self.active == ActivePane::Left {
                                        self.active = ActivePane::Right;
                                    }
                                }
                                // Left panel sort modes
                                MenuAction::LeftSortName => {
                                    self.left.sort_mode = toggle_name_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftSortExt => {
                                    self.left.sort_mode = toggle_ext_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftSortTime => {
                                    self.left.sort_mode = toggle_time_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftSortSize => {
                                    self.left.sort_mode = toggle_size_sort(self.left.sort_mode);
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftUnsorted => {
                                    self.left.sort_mode = SortMode::Unsorted;
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                // Left panel other actions
                                MenuAction::LeftReread => {
                                    let _ = self.left.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::LeftFilter => {
                                    self.status = "Filters not implemented".to_string();
                                }
                                MenuAction::LeftDrive => {
                                    self.open_drive_menu(ActivePane::Left);
                                    return Cmd::none();
                                }
                                // Right panel view modes
                                MenuAction::RightBrief => {
                                    self.right.mode = PanelMode::Brief;
                                }
                                MenuAction::RightFull => {
                                    self.right.mode = PanelMode::Full;
                                }
                                MenuAction::RightInfo => {
                                    self.right.mode = PanelMode::Info;
                                }
                                MenuAction::RightTree => {
                                    self.right.mode = PanelMode::Tree;
                                }
                                MenuAction::RightQuickView => {
                                    self.right.mode = PanelMode::QuickView;
                                }
                                MenuAction::RightOnOff => {
                                    self.hide_right = !self.hide_right;
                                    if self.hide_right && self.active == ActivePane::Right {
                                        self.active = ActivePane::Left;
                                    }
                                }
                                // Right panel sort modes
                                MenuAction::RightSortName => {
                                    self.right.sort_mode = toggle_name_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightSortExt => {
                                    self.right.sort_mode = toggle_ext_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightSortTime => {
                                    self.right.sort_mode = toggle_time_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightSortSize => {
                                    self.right.sort_mode = toggle_size_sort(self.right.sort_mode);
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightUnsorted => {
                                    self.right.sort_mode = SortMode::Unsorted;
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                // Right panel other actions
                                MenuAction::RightReread => {
                                    let _ = self.right.refresh(RefreshMode::Keep, self.show_hidden);
                                }
                                MenuAction::RightFilter => {
                                    self.status = "Filters not implemented".to_string();
                                }
                                MenuAction::RightDrive => {
                                    self.open_drive_menu(ActivePane::Right);
                                    return Cmd::none();
                                }
                                MenuAction::Help => {
                                    self.modal = Some(Modal::Help { page: 0, scroll: 0 });
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
            Modal::CopyDialog(_) => {
                return self.handle_copy_move_dialog_key(key, modal, true);
            }
            Modal::MoveDialog(_) => {
                return self.handle_copy_move_dialog_key(key, modal, false);
            }
            Modal::DeleteDialog { sources, use_filters, focus, .. } => {
                match key.code {
                    KeyCode::Escape => self.modal = None,
                    KeyCode::Tab => {
                        // Cycle through: 0=checkbox, 1=Delete, 2=Filters, 3=Cancel
                        *focus = (*focus + 1) % 4;
                        self.modal = Some(modal);
                    }
                    KeyCode::BackTab => {
                        *focus = if *focus == 0 { 3 } else { *focus - 1 };
                        self.modal = Some(modal);
                    }
                    KeyCode::Char(' ') if *focus == 0 => {
                        *use_filters = !*use_filters;
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        match *focus {
                            0 => {
                                // Toggle checkbox
                                *use_filters = !*use_filters;
                                self.modal = Some(modal);
                            }
                            1 => {
                                // Delete button
                                let sources_clone = sources.clone();
                                self.modal = None;
                                self.execute_confirm(PendingConfirm::Delete { sources: sources_clone });
                            }
                            2 => {
                                // Filters button (not implemented yet)
                                self.status = "Filters not implemented".to_string();
                                self.modal = Some(modal);
                            }
                            3 => {
                                // Cancel
                                self.modal = None;
                            }
                            _ => self.modal = Some(modal),
                        }
                    }
                    _ => self.modal = Some(modal),
                }
            }
        }
        Cmd::none()
    }

    fn handle_copy_move_dialog_key(&mut self, key: KeyEvent, mut modal: Modal, is_copy: bool) -> Cmd<Msg> {
        let state = match &mut modal {
            Modal::CopyDialog(s) | Modal::MoveDialog(s) => s,
            _ => {
                self.modal = Some(modal);
                return Cmd::none();
            }
        };

        match key.code {
            KeyCode::Escape => {
                self.modal = None;
            }
            KeyCode::Tab => {
                // Cycle through focus elements
                state.focus = match state.focus {
                    CopyDialogFocus::Input => CopyDialogFocus::IncludeSubdirs,
                    CopyDialogFocus::IncludeSubdirs => CopyDialogFocus::CopyNewerOnly,
                    CopyDialogFocus::CopyNewerOnly => CopyDialogFocus::UseFilters,
                    CopyDialogFocus::UseFilters => CopyDialogFocus::CheckTargetSpace,
                    CopyDialogFocus::CheckTargetSpace => CopyDialogFocus::BtnCopy,
                    CopyDialogFocus::BtnCopy => CopyDialogFocus::BtnTree,
                    CopyDialogFocus::BtnTree => CopyDialogFocus::BtnFilters,
                    CopyDialogFocus::BtnFilters => CopyDialogFocus::BtnCancel,
                    CopyDialogFocus::BtnCancel => CopyDialogFocus::Input,
                };
                self.modal = Some(modal);
            }
            KeyCode::BackTab => {
                state.focus = match state.focus {
                    CopyDialogFocus::Input => CopyDialogFocus::BtnCancel,
                    CopyDialogFocus::IncludeSubdirs => CopyDialogFocus::Input,
                    CopyDialogFocus::CopyNewerOnly => CopyDialogFocus::IncludeSubdirs,
                    CopyDialogFocus::UseFilters => CopyDialogFocus::CopyNewerOnly,
                    CopyDialogFocus::CheckTargetSpace => CopyDialogFocus::UseFilters,
                    CopyDialogFocus::BtnCopy => CopyDialogFocus::CheckTargetSpace,
                    CopyDialogFocus::BtnTree => CopyDialogFocus::BtnCopy,
                    CopyDialogFocus::BtnFilters => CopyDialogFocus::BtnTree,
                    CopyDialogFocus::BtnCancel => CopyDialogFocus::BtnFilters,
                };
                self.modal = Some(modal);
            }
            KeyCode::Char(' ') => {
                // Toggle checkbox if focused on one
                match state.focus {
                    CopyDialogFocus::IncludeSubdirs => state.include_subdirs = !state.include_subdirs,
                    CopyDialogFocus::CopyNewerOnly => state.copy_newer_only = !state.copy_newer_only,
                    CopyDialogFocus::UseFilters => state.use_filters = !state.use_filters,
                    CopyDialogFocus::CheckTargetSpace => state.check_target_space = !state.check_target_space,
                    CopyDialogFocus::Input => {
                        state.dest.insert(state.cursor, ' ');
                        state.cursor += 1;
                    }
                    _ => {}
                }
                self.modal = Some(modal);
            }
            KeyCode::Enter => {
                match state.focus {
                    CopyDialogFocus::Input | CopyDialogFocus::BtnCopy => {
                        // Execute copy/move
                        let sources = state.sources.clone();
                        let dest = PathBuf::from(&state.dest);
                        self.modal = None;

                        if is_copy {
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
                                return Cmd::none();
                            }
                            let show_hidden = self.show_hidden;
                            match copy_sources(&sources, &dest, false) {
                                Ok(()) => {
                                    self.status = "Copy complete".to_string();
                                    let _ = self.inactive_pane_mut().refresh(RefreshMode::Keep, show_hidden);
                                }
                                Err(err) => {
                                    self.status = format!("Copy failed: {err}");
                                }
                            }
                        } else {
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
                                return Cmd::none();
                            }
                            let show_hidden = self.show_hidden;
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
                    }
                    CopyDialogFocus::IncludeSubdirs => state.include_subdirs = !state.include_subdirs,
                    CopyDialogFocus::CopyNewerOnly => state.copy_newer_only = !state.copy_newer_only,
                    CopyDialogFocus::UseFilters => state.use_filters = !state.use_filters,
                    CopyDialogFocus::CheckTargetSpace => state.check_target_space = !state.check_target_space,
                    CopyDialogFocus::BtnTree => {
                        self.status = "Tree browser not implemented".to_string();
                        self.modal = Some(modal);
                        return Cmd::none();
                    }
                    CopyDialogFocus::BtnFilters => {
                        self.status = "Filters not implemented".to_string();
                        self.modal = Some(modal);
                        return Cmd::none();
                    }
                    CopyDialogFocus::BtnCancel => {
                        self.modal = None;
                    }
                }
                if self.modal.is_some() {
                    self.modal = Some(modal);
                }
            }
            KeyCode::Left if state.focus == CopyDialogFocus::Input => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
                self.modal = Some(modal);
            }
            KeyCode::Right if state.focus == CopyDialogFocus::Input => {
                if state.cursor < state.dest.len() {
                    state.cursor += 1;
                }
                self.modal = Some(modal);
            }
            KeyCode::Backspace if state.focus == CopyDialogFocus::Input => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                    state.dest.remove(state.cursor);
                }
                self.modal = Some(modal);
            }
            KeyCode::Delete if state.focus == CopyDialogFocus::Input => {
                if state.cursor < state.dest.len() {
                    state.dest.remove(state.cursor);
                }
                self.modal = Some(modal);
            }
            KeyCode::Char(ch) if state.focus == CopyDialogFocus::Input => {
                state.dest.insert(state.cursor, ch);
                state.cursor += 1;
                self.modal = Some(modal);
            }
            KeyCode::Home if state.focus == CopyDialogFocus::Input => {
                state.cursor = 0;
                self.modal = Some(modal);
            }
            KeyCode::End if state.focus == CopyDialogFocus::Input => {
                state.cursor = state.dest.len();
                self.modal = Some(modal);
            }
            _ => self.modal = Some(modal),
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

        let (layout_cache, status_area, cmdline_area, key_area) = render_layout(
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
            cmdline_area,
            key_area,
            self.theme,
            &self.left,
            &self.right,
            self.active,
            &self.status,
            &self.cmdline,
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
