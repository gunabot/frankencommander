#![forbid(unsafe_code)]

use std::io;

use crate::app::ensure_visible;
use crate::fs_ops::{read_entries, read_panelized};
use crate::model::{Pane, RefreshMode};
use crate::vfs::{read_zip_entries, zip_child_prefix, zip_parent_prefix};

impl Pane {
    pub fn refresh(&mut self, mode: RefreshMode, show_hidden: bool) -> io::Result<()> {
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

    pub fn selected_entry(&self) -> Option<&crate::model::Entry> {
        let state = self.state.borrow();
        let idx = state.selected?;
        self.entries.get(idx)
    }

    pub fn move_selection(&mut self, delta: i32, view_height: usize) {
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

    pub fn go_parent(&mut self, show_hidden: bool) -> io::Result<()> {
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

    pub fn enter_selected(&mut self, show_hidden: bool) -> io::Result<bool> {
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
            self.vfs = Some(crate::model::VfsState {
                zip_path: entry_path,
                prefix: String::new(),
            });
            self.refresh(RefreshMode::Reset, show_hidden)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn toggle_select(&mut self) {
        let Some(entry) = self.selected_entry() else { return };
        let path = entry.path.clone();
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
    }

    pub fn select_all(&mut self) {
        self.selected = self.entries.iter().map(|e| e.path.clone()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn invert_selection(&mut self) {
        let mut next = std::collections::HashSet::new();
        for entry in &self.entries {
            if !self.selected.contains(&entry.path) {
                next.insert(entry.path.clone());
            }
        }
        self.selected = next;
    }

    pub fn selected_total_size(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| self.selected.contains(&e.path))
            .map(|e| e.size)
            .sum()
    }
}
