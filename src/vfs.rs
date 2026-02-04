#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::model::{Entry, VfsState};

pub fn read_zip_entries(vfs: &VfsState, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let file = fs::File::open(&vfs.zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let prefix = vfs.prefix.as_str();
    let mut entries = Vec::new();
    let mut seen_dirs: HashSet<String> = HashSet::new();
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
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

pub fn read_zip_file_lines(vfs: &VfsState, entry_path: &Path) -> io::Result<Vec<String>> {
    let file = fs::File::open(&vfs.zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    let full = format!("{}{}", vfs.prefix, entry_path.to_string_lossy());
    let mut zip_file = archive.by_name(&full)?;
    let mut data = Vec::new();
    zip_file.read_to_end(&mut data)?;
    let content = String::from_utf8_lossy(&data);
    Ok(content.lines().map(|line| line.to_string()).collect())
}

pub fn zip_parent_prefix(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end_matches('/');
    let parent = Path::new(trimmed).parent()?.to_string_lossy().to_string();
    if parent.is_empty() {
        Some(String::new())
    } else {
        Some(format!("{}/", parent))
    }
}

pub fn zip_child_prefix(prefix: &str, entry_path: &Path) -> String {
    let child = entry_path.to_string_lossy();
    format!("{}{}/", prefix, child)
}
