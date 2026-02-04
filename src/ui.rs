#![forbid(unsafe_code)]

use ftui::layout::{Constraint, Flex};
use ftui::style::Style;
use ftui::text::{Text, WrapMode, display_width};
use ftui::widgets::block::Block;
use ftui::widgets::borders::Borders;
use ftui::widgets::paragraph::Paragraph;
use ftui::widgets::status_line::{StatusItem, StatusLine};
use ftui::widgets::table::{Row, Table};
use ftui::widgets::{StatefulWidget, Widget};
use ftui::Frame;

use crate::app::ThemeColors;
use crate::fs_ops::{format_time, sort_label};
use crate::menu::{menu_items, MENU_TITLES};
use crate::model::{ActivePane, LayoutCache, Modal, Pane, Viewer};

pub const MENU_HEIGHT: u16 = 1;
pub const STATUS_HEIGHT: u16 = 1;
pub const KEYBAR_HEIGHT: u16 = 1;
pub const HEADER_HEIGHT: u16 = 1;

pub fn render_viewer(viewer: &Viewer, frame: &mut Frame, theme: ThemeColors) {
    let area = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
    let style = Style::new().fg(theme.panel_fg).bg(theme.panel_bg);
    let paragraph = Paragraph::new(Text::from(viewer.lines.join("\n")))
        .wrap(WrapMode::None)
        .scroll((viewer.scroll as u16, 0))
        .style(style)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.panel_border_active))
                .borders(Borders::ALL)
                .title("View"),
        );
    paragraph.render(area, frame);
}

pub fn render_status(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
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
    let selected_size = pane
        .entries
        .iter()
        .filter(|e| pane.selected.contains(&e.path))
        .map(|e| e.size)
        .sum::<u64>();
    let right = format!("Sel: {} Size: {}", selected_count, selected_size);
    let spacing = area.width.saturating_sub(display_width(status) as u16 + right.len() as u16 + 2);
    let line = format!("{}{}{}", status, " ".repeat(spacing as usize), right);
    let paragraph = Paragraph::new(Text::from(line))
        .style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    paragraph.render(area, frame);
}

pub fn render_keybar(frame: &mut Frame, area: ftui::core::geometry::Rect, theme: ThemeColors) {
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

pub fn render_modal(frame: &mut Frame, modal: &Modal, theme: ThemeColors, _left: &Pane, _right: &Pane) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
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
    let area = ftui::core::geometry::Rect::new(x, y, width, height);
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
            lines.push(format!("{} Sort: {}", marker1, sort_label(*sort_mode)));
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

pub fn render_panel(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    pane: &Pane,
    active: bool,
    theme: ThemeColors,
) -> ftui::core::geometry::Rect {
    let border_color = if active {
        theme.panel_border_active
    } else {
        theme.panel_border_inactive
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
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title(title.as_str());

    let header = Row::new(["Name", "Size", "Date", "Time"])
        .style(Style::new().fg(theme.header_fg).bg(theme.header_bg))
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
                row = row.style(Style::new().fg(theme.system_fg).bg(theme.panel_bg));
            }
            if is_marked {
                row = row.style(Style::new().fg(theme.selection_fg).bg(theme.panel_bg));
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
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else {
        Style::new().fg(theme.panel_fg).bg(theme.panel_bg)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .highlight_style(highlight_style);

    let mut state = pane.state.borrow_mut();
    StatefulWidget::render(&table, area, frame, &mut state);
    area
}

pub fn render_menu(frame: &mut Frame, area: ftui::core::geometry::Rect, theme: ThemeColors) {
    let menu_bg = Block::new().style(Style::new().fg(theme.menu_fg).bg(theme.menu_bg));
    menu_bg.render(area, frame);
    let menu = Paragraph::new(Text::from(" File  Command  Options  Left  Right  Help "))
        .style(Style::new().fg(theme.menu_fg).bg(theme.menu_bg));
    menu.render(area, frame);
}

pub fn render_layout(
    frame: &mut Frame,
    theme: ThemeColors,
    left: &Pane,
    right: &Pane,
    active: ActivePane,
    hide_left: bool,
    hide_right: bool,
    hide_all: bool,
    cmdline: &str,
    cmd_cursor: usize,
) -> (Option<LayoutCache>, ftui::core::geometry::Rect, ftui::core::geometry::Rect) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
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

    render_menu(frame, menu_area, theme);

    if hide_all {
        let block = Block::bordered()
            .border_style(Style::new().fg(theme.panel_border_active))
            .title("Command line");
        let paragraph = Paragraph::new(Text::from(cmdline.to_string()))
            .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
            .block(block);
        paragraph.render(body_area, frame);
        let cursor_x = body_area.x + 1 + cmd_cursor as u16;
        let cursor_y = body_area.y + 1;
        frame.set_cursor(Some((cursor_x, cursor_y)));
    }

    let mut layout_cache = None;
    if !hide_all {
        let mut left_area = ftui::core::geometry::Rect::new(0, 0, 0, 0);
        let mut right_area = ftui::core::geometry::Rect::new(0, 0, 0, 0);
        if !hide_left && !hide_right {
            let columns = Flex::horizontal().constraints([
                Constraint::Ratio(1, 2),
                Constraint::Ratio(1, 2),
            ]);
            let col_areas = columns.split(body_area);
            left_area = render_panel(frame, col_areas[0], left, active == ActivePane::Left, theme);
            right_area = render_panel(frame, col_areas[1], right, active == ActivePane::Right, theme);
        } else if !hide_left {
            left_area = render_panel(frame, body_area, left, active == ActivePane::Left, theme);
        } else if !hide_right {
            right_area = render_panel(frame, body_area, right, active == ActivePane::Right, theme);
        }
        layout_cache = Some(LayoutCache { left_table: left_area, right_table: right_area });
    }

    (layout_cache, status_area, key_area)
}

pub fn render_background(frame: &mut Frame, theme: ThemeColors) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
    let background = Block::new().style(Style::new().fg(theme.panel_fg).bg(theme.screen_bg));
    background.render(full, frame);
}

pub fn render_status_and_keybar(
    frame: &mut Frame,
    status_area: ftui::core::geometry::Rect,
    key_area: ftui::core::geometry::Rect,
    theme: ThemeColors,
    left: &Pane,
    right: &Pane,
    active: ActivePane,
    status: &str,
) {
    render_status(frame, status_area, left, right, active, status, theme);
    render_keybar(frame, key_area, theme);
}

pub fn render_modal_wrapper(frame: &mut Frame, modal: &Modal, theme: ThemeColors, left: &Pane, right: &Pane) {
    render_modal(frame, modal, theme, left, right);
}
