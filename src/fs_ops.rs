#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::model::{Entry, SortMode, TreeItem, UserMenuItem};

pub fn read_entries(
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
            SortMode::ExtAsc => cmp_ext(a, b).then_with(|| cmp_name(a, b)),
            SortMode::ExtDesc => cmp_ext(b, a).then_with(|| cmp_name(a, b)),
            SortMode::TimeAsc => cmp_time(a, b).then_with(|| cmp_name(a, b)),
            SortMode::TimeDesc => cmp_time(b, a).then_with(|| cmp_name(a, b)),
            SortMode::SizeAsc => cmp_size(a, b).then_with(|| cmp_name(a, b)),
            SortMode::SizeDesc => cmp_size(b, a).then_with(|| cmp_name(a, b)),
            SortMode::Unsorted => Ordering::Equal,
        }
    });

    Ok(entries)
}

pub fn read_panelized(paths: &[PathBuf]) -> io::Result<Vec<Entry>> {
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

pub fn cmp_name(a: &Entry, b: &Entry) -> Ordering {
    a.name.to_lowercase().cmp(&b.name.to_lowercase())
}

pub fn cmp_ext(a: &Entry, b: &Entry) -> Ordering {
    let ext_a = a.name.rsplit('.').next().unwrap_or("").to_lowercase();
    let ext_b = b.name.rsplit('.').next().unwrap_or("").to_lowercase();
    ext_a.cmp(&ext_b)
}

pub fn cmp_time(a: &Entry, b: &Entry) -> Ordering {
    let a_time = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    let b_time = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    a_time.cmp(&b_time)
}

pub fn cmp_size(a: &Entry, b: &Entry) -> Ordering {
    a.size.cmp(&b.size)
}

pub fn toggle_name_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::NameAsc => SortMode::NameDesc,
        SortMode::NameDesc => SortMode::NameAsc,
        _ => SortMode::NameAsc,
    }
}

pub fn toggle_ext_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::ExtAsc => SortMode::ExtDesc,
        SortMode::ExtDesc => SortMode::ExtAsc,
        _ => SortMode::ExtAsc,
    }
}

pub fn toggle_time_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::TimeAsc => SortMode::TimeDesc,
        SortMode::TimeDesc => SortMode::TimeAsc,
        _ => SortMode::TimeDesc,
    }
}

pub fn toggle_size_sort(mode: SortMode) -> SortMode {
    match mode {
        SortMode::SizeAsc => SortMode::SizeDesc,
        SortMode::SizeDesc => SortMode::SizeAsc,
        _ => SortMode::SizeDesc,
    }
}

pub fn sort_label(mode: SortMode) -> &'static str {
    match mode {
        SortMode::NameAsc => "Name ↑",
        SortMode::NameDesc => "Name ↓",
        SortMode::ExtAsc => "Ext ↑",
        SortMode::ExtDesc => "Ext ↓",
        SortMode::TimeAsc => "Time ↑",
        SortMode::TimeDesc => "Time ↓",
        SortMode::SizeAsc => "Size ↑",
        SortMode::SizeDesc => "Size ↓",
        SortMode::Unsorted => "Unsorted",
    }
}

pub fn sort_indicator(mode: SortMode) -> &'static str {
    match mode {
        SortMode::NameAsc | SortMode::ExtAsc | SortMode::TimeAsc | SortMode::SizeAsc => "↑",
        SortMode::NameDesc | SortMode::ExtDesc | SortMode::TimeDesc | SortMode::SizeDesc => "↓",
        SortMode::Unsorted => "",
    }
}

pub fn format_time(time: Option<SystemTime>) -> (String, String) {
    let Some(time) = time else {
        return ("".to_string(), "".to_string());
    };
    let date_fmt = time::format_description::parse("[day]-[month]-[year repr:last_two]").unwrap();
    let time_fmt = time::format_description::parse("[hour]:[minute]").unwrap();
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let dt = time::OffsetDateTime::from(time).to_offset(offset);
    let date = dt.format(&date_fmt).unwrap_or_default();
    let clock = dt.format(&time_fmt).unwrap_or_default();
    (date, clock)
}

pub fn read_file_lines(path: &Path) -> io::Result<Vec<String>> {
    let data = fs::read(path)?;
    let content = String::from_utf8_lossy(&data);
    Ok(content.lines().map(|line| line.to_string()).collect())
}

pub fn find_conflicts(sources: &[PathBuf], dest: &Path) -> Option<usize> {
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

pub fn copy_sources(sources: &[PathBuf], dest: &Path, overwrite: bool) -> io::Result<()> {
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

pub fn move_sources(sources: &[PathBuf], dest: &Path, overwrite: bool) -> io::Result<()> {
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

pub fn copy_entry(src: &Path, dest: &Path) -> io::Result<()> {
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

pub fn move_entry(src: &Path, dest: &Path) -> io::Result<()> {
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

pub fn remove_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
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

pub fn find_matches(base: &Path, query: &str, show_hidden: bool) -> Vec<PathBuf> {
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

pub fn build_tree(base: &Path, max_depth: usize, show_hidden: bool) -> Vec<TreeItem> {
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

pub fn list_drive_roots() -> Vec<PathBuf> {
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

pub fn user_menu_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/nuc".to_string());
    Path::new(&home).join(".frankencommander").join("usermenu.txt")
}

pub fn ensure_user_menu_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let sample = "List|ls -la\nEdit config|$EDITOR ~/.frankencommander/usermenu.txt\n";
        fs::write(path, sample)?;
    }
    Ok(())
}

pub fn load_user_menu(path: &Path) -> Vec<UserMenuItem> {
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

pub fn sync_plan(src: &Path, dst: &Path) -> Vec<PathBuf> {
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

pub fn sync_execute(ops: &[PathBuf], src_root: &Path, dst_root: &Path) -> io::Result<usize> {
    let mut count = 0;
    for src in ops {
        let rel = src.strip_prefix(src_root).unwrap_or(src);
        let target = dst_root.join(rel);
        copy_entry(src, &target)?;
        count += 1;
    }
    Ok(count)
}
