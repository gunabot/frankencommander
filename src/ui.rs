#![forbid(unsafe_code)]

use ftui::layout::{Constraint, Flex};
use ftui::render::cell::PackedRgba;
use ftui::style::Style;
use ftui::text::{Text, WrapMode};
use ftui::widgets::block::Block;
use ftui::widgets::borders::Borders;
use ftui::widgets::paragraph::Paragraph;
use ftui::widgets::status_line::{StatusItem, StatusLine};
use ftui::widgets::table::{Row, Table};
use ftui::widgets::{StatefulWidget, Widget};
use ftui::Frame;

use crate::app::ThemeColors;
use crate::fs_ops::{format_time, sort_indicator, sort_label};
use crate::menu::{menu_items, MENU_TITLES};
use crate::model::{ActivePane, CopyDialogFocus, CopyDialogState, LayoutCache, MenuAction, Modal, Pane, PanelMode, SortMode, Viewer};

pub const MENU_HEIGHT: u16 = 1;
pub const STATUS_HEIGHT: u16 = 1;
pub const CMDLINE_HEIGHT: u16 = 1;
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
    _active: ActivePane,
    _status: &str,
    theme: ThemeColors,
) {
    let bg = Block::new().style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    bg.render(area, frame);

    // NC5 style: show selected file details for each panel, or selection summary
    let left_status = panel_status_text(left);
    let right_status = panel_status_text(right);

    // Split area in half for left and right panel status
    let half_width = area.width / 2;

    // Left panel status
    let left_truncated: String = left_status.chars().take(half_width as usize - 1).collect();
    let left_para = Paragraph::new(Text::from(left_truncated))
        .style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    let left_area = ftui::core::geometry::Rect::new(area.x, area.y, half_width, 1);
    left_para.render(left_area, frame);

    // Separator
    let sep_area = ftui::core::geometry::Rect::new(area.x + half_width, area.y, 1, 1);
    let sep_para = Paragraph::new(Text::from("│"))
        .style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    sep_para.render(sep_area, frame);

    // Right panel status
    let right_truncated: String = right_status.chars().take(half_width as usize - 1).collect();
    let right_para = Paragraph::new(Text::from(right_truncated))
        .style(Style::new().fg(theme.status_fg).bg(theme.status_bg));
    let right_area = ftui::core::geometry::Rect::new(area.x + half_width + 1, area.y, half_width - 1, 1);
    right_para.render(right_area, frame);
}

fn panel_status_text(pane: &Pane) -> String {
    if !pane.selected.is_empty() {
        // NC5 style: "X bytes in Y selected files"
        let selected_size: u64 = pane.entries
            .iter()
            .filter(|e| pane.selected.contains(&e.path))
            .map(|e| e.size)
            .sum();
        format!("{} bytes in {} selected", selected_size, pane.selected.len())
    } else if let Some(entry) = pane.selected_entry() {
        // NC5 style: "filename  size  date  time" for current file
        let (date, time) = format_time(entry.modified);
        if entry.is_dir {
            if entry.name == ".." {
                format!("..  ►UP-DIR◄  {}  {}", date, time)
            } else {
                format!("{}  ►DIR◄  {}  {}", entry.name, date, time)
            }
        } else {
            format!("{}  {}  {}  {}", entry.name, entry.size, date, time)
        }
    } else {
        String::new()
    }
}

fn render_copy_move_dialog(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    state: &CopyDialogState,
    is_copy: bool,
    theme: ThemeColors,
) {
    let style = Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let title = if is_copy { "Copy" } else { "Rename" };
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.panel_border_active))
        .style(style)
        .title(title);
    let inner = block.inner(area);
    block.render(area, frame);

    // Label: Copy/Rename "filename" to
    let action = if is_copy { "Copy" } else { "Rename or move" };
    let label = format!("{} \"{}\" to", action, state.source_name);
    let label_para = Paragraph::new(Text::from(label)).style(style);
    let label_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
    label_para.render(label_area, frame);

    // Input field with dotted fill
    let field_width = (inner.width as usize).saturating_sub(2);
    let input_display = if state.dest.len() <= field_width {
        let padding = field_width.saturating_sub(state.dest.len());
        format!("[{}{}]", state.dest, ".".repeat(padding))
    } else {
        let start = state.dest.len().saturating_sub(field_width);
        format!("[{}]", &state.dest[start..])
    };
    let input_style = if state.focus == CopyDialogFocus::Input {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else {
        style
    };
    let input_para = Paragraph::new(Text::from(input_display)).style(input_style);
    let input_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 1, inner.width, 1);
    input_para.render(input_area, frame);

    // Checkboxes row 1
    let cb1 = if state.include_subdirs { "[x]" } else { "[ ]" };
    let cb2 = if state.copy_newer_only { "[x]" } else { "[ ]" };
    let cb1_style = if state.focus == CopyDialogFocus::IncludeSubdirs {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let cb2_style = if state.focus == CopyDialogFocus::CopyNewerOnly {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };

    let cb1_text = format!("{} Include subdirectories", cb1);
    let cb2_text = format!("{} Copy newer files only", cb2);
    let half = inner.width / 2;

    let cb1_para = Paragraph::new(Text::from(cb1_text)).style(cb1_style);
    let cb1_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 3, half, 1);
    cb1_para.render(cb1_area, frame);

    let cb2_para = Paragraph::new(Text::from(cb2_text)).style(cb2_style);
    let cb2_area = ftui::core::geometry::Rect::new(inner.x + half, inner.y + 3, half, 1);
    cb2_para.render(cb2_area, frame);

    // Checkboxes row 2
    let cb3 = if state.use_filters { "[x]" } else { "[ ]" };
    let cb4 = if state.check_target_space { "[x]" } else { "[ ]" };
    let cb3_style = if state.focus == CopyDialogFocus::UseFilters {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let cb4_style = if state.focus == CopyDialogFocus::CheckTargetSpace {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };

    let cb3_text = format!("{} Use Filters", cb3);
    let cb4_text = format!("{} Check target space", cb4);

    let cb3_para = Paragraph::new(Text::from(cb3_text)).style(cb3_style);
    let cb3_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 4, half, 1);
    cb3_para.render(cb3_area, frame);

    let cb4_para = Paragraph::new(Text::from(cb4_text)).style(cb4_style);
    let cb4_area = ftui::core::geometry::Rect::new(inner.x + half, inner.y + 4, half, 1);
    cb4_para.render(cb4_area, frame);

    // Buttons row
    let btn_copy = if is_copy { "[ Copy ]" } else { "[Rename/Move]" };
    let btn_tree = "[F10-Tree]";
    let btn_filters = "[Filters]";
    let btn_cancel = "[Cancel]";

    let btn_copy_style = if state.focus == CopyDialogFocus::BtnCopy {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let btn_tree_style = if state.focus == CopyDialogFocus::BtnTree {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let btn_filters_style = if state.focus == CopyDialogFocus::BtnFilters {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let btn_cancel_style = if state.focus == CopyDialogFocus::BtnCancel {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };

    let btn_y = inner.y + 6;
    let btn_spacing = inner.width / 4;

    let btn_copy_para = Paragraph::new(Text::from(btn_copy)).style(btn_copy_style);
    let btn_copy_area = ftui::core::geometry::Rect::new(inner.x, btn_y, btn_spacing, 1);
    btn_copy_para.render(btn_copy_area, frame);

    let btn_tree_para = Paragraph::new(Text::from(btn_tree)).style(btn_tree_style);
    let btn_tree_area = ftui::core::geometry::Rect::new(inner.x + btn_spacing, btn_y, btn_spacing, 1);
    btn_tree_para.render(btn_tree_area, frame);

    let btn_filters_para = Paragraph::new(Text::from(btn_filters)).style(btn_filters_style);
    let btn_filters_area = ftui::core::geometry::Rect::new(inner.x + btn_spacing * 2, btn_y, btn_spacing, 1);
    btn_filters_para.render(btn_filters_area, frame);

    let btn_cancel_para = Paragraph::new(Text::from(btn_cancel)).style(btn_cancel_style);
    let btn_cancel_area = ftui::core::geometry::Rect::new(inner.x + btn_spacing * 3, btn_y, btn_spacing, 1);
    btn_cancel_para.render(btn_cancel_area, frame);

    // Set cursor position if focused on input
    if state.focus == CopyDialogFocus::Input {
        let cursor_x = area.x + 2 + state.cursor.min(field_width) as u16;
        let cursor_y = area.y + 2;
        frame.set_cursor(Some((cursor_x, cursor_y)));
    }
}

fn render_delete_dialog(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    source_name: &str,
    source_count: usize,
    use_filters: bool,
    focus: usize,
    theme: ThemeColors,
) {
    let style = Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.panel_border_active))
        .style(style)
        .title("Delete");
    let inner = block.inner(area);
    block.render(area, frame);

    // Message
    let msg = if source_count == 1 {
        format!("Do you wish to delete \"{}\"", source_name)
    } else {
        format!("Do you wish to delete {} items?", source_count)
    };
    let msg_para = Paragraph::new(Text::from(msg)).style(style);
    let msg_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 1, inner.width, 1);
    msg_para.render(msg_area, frame);

    // Checkbox
    let cb = if use_filters { "[x]" } else { "[ ]" };
    let cb_style = if focus == 0 {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let cb_text = format!("{} Use Filters", cb);
    let cb_para = Paragraph::new(Text::from(cb_text)).style(cb_style);
    let cb_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 3, inner.width, 1);
    cb_para.render(cb_area, frame);

    // Buttons
    let btn_delete_style = if focus == 1 {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let btn_filters_style = if focus == 2 {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };
    let btn_cancel_style = if focus == 3 {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else { style };

    let btn_y = inner.y + 5;
    let btn_spacing = inner.width / 3;

    let btn_del_para = Paragraph::new(Text::from("[Delete]")).style(btn_delete_style);
    let btn_del_area = ftui::core::geometry::Rect::new(inner.x, btn_y, btn_spacing, 1);
    btn_del_para.render(btn_del_area, frame);

    let btn_fil_para = Paragraph::new(Text::from("[Filters]")).style(btn_filters_style);
    let btn_fil_area = ftui::core::geometry::Rect::new(inner.x + btn_spacing, btn_y, btn_spacing, 1);
    btn_fil_para.render(btn_fil_area, frame);

    let btn_can_para = Paragraph::new(Text::from("[Cancel]")).style(btn_cancel_style);
    let btn_can_area = ftui::core::geometry::Rect::new(inner.x + btn_spacing * 2, btn_y, btn_spacing, 1);
    btn_can_para.render(btn_can_area, frame);
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

pub fn render_modal(frame: &mut Frame, modal: &Modal, theme: ThemeColors, left: &Pane, right: &Pane) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
    let width = full.width.min(70).max(30);
    let height = match modal {
        Modal::CopyDialog(_) | Modal::MoveDialog(_) => 12,
        Modal::DeleteDialog { .. } => 10,
        Modal::Prompt { .. } => 8,
        Modal::Confirm { .. } => 8,
        Modal::FindResults { .. } => 10,
        Modal::Tree { .. } => 12,
        Modal::DriveMenu { .. } => 10,
        Modal::Config { .. } => 12,
        Modal::PanelOptions { .. } => 9,
        Modal::UserMenu { .. } => 10,
        Modal::About => 8,
        Modal::Help { .. } => 18,
        Modal::PullDown { .. } => 10,
    };
    let x = full.x + (full.width.saturating_sub(width)) / 2;
    let y = full.y + (full.height.saturating_sub(height)) / 2;
    let area = ftui::core::geometry::Rect::new(x, y, width, height);
    let style = Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg);

    // NC5-style shadow effect (draw shadow first, then dialog)
    let shadow_area = ftui::core::geometry::Rect::new(x + 2, y + 1, width, height);
    let shadow_style = Style::new().bg(PackedRgba::rgb(0, 0, 0));
    let shadow = Block::new().style(shadow_style);
    shadow.render(shadow_area, frame);

    let fill = Block::new().style(style);
    fill.render(area, frame);
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.panel_border_active))
        .style(style);

    match modal {
        Modal::CopyDialog(state) | Modal::MoveDialog(state) => {
            render_copy_move_dialog(frame, area, state, matches!(modal, Modal::CopyDialog(_)), theme);
        }
        Modal::DeleteDialog { sources, source_name, use_filters, focus } => {
            render_delete_dialog(frame, area, source_name, sources.len(), *use_filters, *focus, theme);
        }
        Modal::Prompt { title, label, value, cursor, .. } => {
            // NC5-style prompt with dotted input field
            let inner = block.inner(area);
            block.render(area, frame);

            // Title
            let title_para = Paragraph::new(Text::from(title.as_str()))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // Label
            let label_para = Paragraph::new(Text::from(label.as_str()))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let label_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 2, inner.width, 1);
            label_para.render(label_area, frame);

            // NC5-style input field with dotted fill: [.........................]
            let field_width = (inner.width as usize).saturating_sub(2);
            let input_display = if value.len() <= field_width {
                let padding = field_width.saturating_sub(value.len());
                format!("[{}{}]", value, ".".repeat(padding))
            } else {
                let start = value.len().saturating_sub(field_width);
                format!("[{}]", &value[start..])
            };
            let input_para = Paragraph::new(Text::from(input_display))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let input_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 3, inner.width, 1);
            input_para.render(input_area, frame);

            // Button hint
            let btn_text = "[ Enter ] [ Esc ]";
            let btn_para = Paragraph::new(Text::from(btn_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let btn_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 5, inner.width, 1);
            btn_para.render(btn_area, frame);

            let cursor_x = area.x + 2 + (*cursor).min(field_width) as u16;
            let cursor_y = area.y + 1 + 3;
            frame.set_cursor(Some((cursor_x, cursor_y)));
        }
        Modal::Confirm { title, message, .. } => {
            // NC5-style confirm dialog
            let inner = block.inner(area);
            block.render(area, frame);

            // Title
            let title_para = Paragraph::new(Text::from(title.as_str()))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // Message
            let msg_para = Paragraph::new(Text::from(message.as_str()))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let msg_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 2, inner.width, 2);
            msg_para.render(msg_area, frame);

            // NC5-style buttons
            let btn_text = "[ Yes ]    [ No ]";
            let btn_para = Paragraph::new(Text::from(btn_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let btn_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 5, inner.width, 1);
            btn_para.render(btn_area, frame);
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
        Modal::Config { page, selected, show_hidden, auto_save, confirm_delete, confirm_overwrite } => {
            let inner = block.inner(area);
            block.render(area, frame);

            // Title
            let title_para = Paragraph::new(Text::from("Configuration"))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // Page tabs (NC5 style)
            let pages = ["Screen", "Confirmations", "Other"];
            let tab_width = (inner.width as usize) / pages.len();
            for (i, label) in pages.iter().enumerate() {
                let tab_style = if i == *page {
                    Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                } else {
                    Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                };
                let tab_text = format!(" {} ", label);
                let tab_para = Paragraph::new(Text::from(tab_text)).style(tab_style);
                let tab_x = inner.x + (i * tab_width) as u16;
                let tab_area = ftui::core::geometry::Rect::new(tab_x, inner.y + 1, tab_width as u16, 1);
                tab_para.render(tab_area, frame);
            }

            // Page content
            let content_y = inner.y + 3;
            match page {
                0 => {
                    // Screen options
                    let checkbox = if *show_hidden { "[x]" } else { "[ ]" };
                    let item_style = if *selected == 0 {
                        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                    } else {
                        Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                    };
                    let item_text = format!("{} Show hidden files", checkbox);
                    let item_para = Paragraph::new(Text::from(item_text)).style(item_style);
                    let item_area = ftui::core::geometry::Rect::new(inner.x, content_y, inner.width, 1);
                    item_para.render(item_area, frame);
                }
                1 => {
                    // Confirmations
                    let cb1 = if *confirm_delete { "[x]" } else { "[ ]" };
                    let cb1_style = if *selected == 0 {
                        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                    } else {
                        Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                    };
                    let cb1_text = format!("{} Confirm file delete", cb1);
                    let cb1_para = Paragraph::new(Text::from(cb1_text)).style(cb1_style);
                    let cb1_area = ftui::core::geometry::Rect::new(inner.x, content_y, inner.width, 1);
                    cb1_para.render(cb1_area, frame);

                    let cb2 = if *confirm_overwrite { "[x]" } else { "[ ]" };
                    let cb2_style = if *selected == 1 {
                        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                    } else {
                        Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                    };
                    let cb2_text = format!("{} Confirm file overwrite", cb2);
                    let cb2_para = Paragraph::new(Text::from(cb2_text)).style(cb2_style);
                    let cb2_area = ftui::core::geometry::Rect::new(inner.x, content_y + 1, inner.width, 1);
                    cb2_para.render(cb2_area, frame);
                }
                _ => {
                    // Other options
                    let checkbox = if *auto_save { "[x]" } else { "[ ]" };
                    let item_style = if *selected == 0 {
                        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                    } else {
                        Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                    };
                    let item_text = format!("{} Auto save setup", checkbox);
                    let item_para = Paragraph::new(Text::from(item_text)).style(item_style);
                    let item_area = ftui::core::geometry::Rect::new(inner.x, content_y, inner.width, 1);
                    item_para.render(item_area, frame);
                }
            }

            // Button hint
            let btn_text = "←/→ Pages  Space Toggle  Esc Close";
            let btn_para = Paragraph::new(Text::from(btn_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let btn_area = ftui::core::geometry::Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
            btn_para.render(btn_area, frame);
        }
        Modal::PanelOptions { pane, selected, dirs_first, sort_mode } => {
            let inner = block.inner(area);
            block.render(area, frame);

            let target = match pane {
                ActivePane::Left => "Left",
                ActivePane::Right => "Right",
            };

            // Title
            let title = format!("{} panel options", target);
            let title_para = Paragraph::new(Text::from(title))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // NC5-style checkboxes
            let checkbox0 = if *dirs_first { "[x]" } else { "[ ]" };
            let item0_style = if *selected == 0 {
                Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
            } else {
                Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
            };
            let item0_text = format!("{} Directories first", checkbox0);
            let item0_para = Paragraph::new(Text::from(item0_text)).style(item0_style);
            let item0_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 2, inner.width, 1);
            item0_para.render(item0_area, frame);

            let item1_style = if *selected == 1 {
                Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
            } else {
                Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
            };
            let item1_text = format!("    Sort: {}", sort_label(*sort_mode));
            let item1_para = Paragraph::new(Text::from(item1_text)).style(item1_style);
            let item1_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 3, inner.width, 1);
            item1_para.render(item1_area, frame);

            // Button hint
            let btn_text = "[ Enter ] Toggle   [ Esc ] Close";
            let btn_para = Paragraph::new(Text::from(btn_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let btn_area = ftui::core::geometry::Rect::new(inner.x, inner.y + 6, inner.width, 1);
            btn_para.render(btn_area, frame);
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
        Modal::Help { page, scroll } => {
            let inner = block.inner(area);
            block.render(area, frame);

            // NC5-style help pages
            let pages = ["Overview", "Keys", "Panels", "Files"];
            let help_content: &[&str] = match page {
                0 => &[
                    "FrankenCommander Help",
                    "",
                    "A Norton Commander 5.0 clone built with",
                    "FrankenTUI (Rust).",
                    "",
                    "Use ←/→ to navigate between help pages.",
                    "Use ↑/↓ or PgUp/PgDn to scroll.",
                    "Press Esc to close help.",
                    "",
                    "This file manager provides a dual-pane",
                    "interface for browsing and managing files",
                    "with NC5-style keyboard shortcuts.",
                ],
                1 => &[
                    "Keyboard Shortcuts",
                    "",
                    "F1       Help (this screen)",
                    "F2       User menu",
                    "F3       View file contents",
                    "F4       Edit file with $EDITOR",
                    "F5       Copy files/directories",
                    "F6       Move/Rename files",
                    "F7       Make new directory",
                    "F8       Delete files/directories",
                    "F9       Pull-down menu",
                    "F10      Quit",
                    "F11      File attributes (chmod)",
                    "",
                    "Tab      Switch active pane",
                    "Ins      Select/unselect file",
                    "Space    Select/unselect file",
                    "+        Select all files",
                    "-        Clear all selections",
                    "*        Invert selection",
                ],
                2 => &[
                    "Panel Modes",
                    "",
                    "Ctrl+1   Brief mode (3-column)",
                    "Ctrl+2   Full mode (name/size/date)",
                    "Ctrl+3   Info mode (directory info)",
                    "Ctrl+4   Quick view mode (preview)",
                    "",
                    "Panel Operations:",
                    "Ctrl+F1  Toggle left panel",
                    "Ctrl+F2  Toggle right panel",
                    "Ctrl+O   Command line mode",
                    "",
                    "Sort Modes (via menu):",
                    "Name, Extension, Time, Size, Unsorted",
                ],
                _ => &[
                    "File Operations",
                    "",
                    "Alt+F1   Drive menu (left panel)",
                    "Alt+F2   Drive menu (right panel)",
                    "Alt+F7   Find file",
                    "Ctrl+F8  Sync directories",
                    "",
                    "Quick Search:",
                    "Type characters to jump to matching",
                    "file names. Backspace to edit search.",
                    "",
                    "Selection:",
                    "Insert or Space to toggle selection.",
                    "Selected files shown with * marker.",
                    "Operations apply to selected files",
                    "or current file if none selected.",
                ],
            };

            // Title with page tabs
            let title_para = Paragraph::new(Text::from("Help"))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // Page tabs
            let tab_width = (inner.width as usize) / pages.len();
            for (i, label) in pages.iter().enumerate() {
                let tab_style = if i == *page {
                    Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                } else {
                    Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                };
                let tab_text = format!(" {} ", label);
                let tab_para = Paragraph::new(Text::from(tab_text)).style(tab_style);
                let tab_x = inner.x + (i * tab_width) as u16;
                let tab_area = ftui::core::geometry::Rect::new(tab_x, inner.y + 1, tab_width as u16, 1);
                tab_para.render(tab_area, frame);
            }

            // Content area
            let content_y = inner.y + 3;
            let content_height = inner.height.saturating_sub(5) as usize;
            let visible_lines: Vec<&str> = help_content
                .iter()
                .skip(*scroll)
                .take(content_height)
                .copied()
                .collect();
            let content_text = visible_lines.join("\n");
            let content_para = Paragraph::new(Text::from(content_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let content_area = ftui::core::geometry::Rect::new(inner.x, content_y, inner.width, content_height as u16);
            content_para.render(content_area, frame);

            // Navigation hint
            let nav_text = "←/→ Pages  ↑/↓ Scroll  Esc Close";
            let nav_para = Paragraph::new(Text::from(nav_text))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let nav_area = ftui::core::geometry::Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
            nav_para.render(nav_area, frame);
        }
        Modal::PullDown { menu_idx, item_idx } => {
            let items = menu_items(*menu_idx);
            let inner = block.inner(area);
            block.render(area, frame);

            // Title
            let title_para = Paragraph::new(Text::from(MENU_TITLES[*menu_idx]))
                .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
            let title_area = ftui::core::geometry::Rect::new(inner.x, inner.y, inner.width, 1);
            title_para.render(title_area, frame);

            // Menu items with shortcuts, checkmarks, and separators
            let mut y_offset = 1u16;
            for (idx, item) in items.iter().enumerate() {
                let item_style = if idx == *item_idx {
                    Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
                } else {
                    Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg)
                };

                // Dynamic checkmarks for panel modes and sort modes
                let checkmark = if *menu_idx == 3 {
                    // Left panel menu
                    match item.action {
                        // View modes
                        MenuAction::LeftBrief => if left.mode == PanelMode::Brief { "√ " } else { "  " },
                        MenuAction::LeftFull => if left.mode == PanelMode::Full { "√ " } else { "  " },
                        MenuAction::LeftInfo => if left.mode == PanelMode::Info { "√ " } else { "  " },
                        MenuAction::LeftTree => if left.mode == PanelMode::Tree { "√ " } else { "  " },
                        MenuAction::LeftQuickView => if left.mode == PanelMode::QuickView { "√ " } else { "  " },
                        // Sort modes
                        MenuAction::LeftSortName => if matches!(left.sort_mode, SortMode::NameAsc | SortMode::NameDesc) { "√ " } else { "  " },
                        MenuAction::LeftSortExt => if matches!(left.sort_mode, SortMode::ExtAsc | SortMode::ExtDesc) { "√ " } else { "  " },
                        MenuAction::LeftSortTime => if matches!(left.sort_mode, SortMode::TimeAsc | SortMode::TimeDesc) { "√ " } else { "  " },
                        MenuAction::LeftSortSize => if matches!(left.sort_mode, SortMode::SizeAsc | SortMode::SizeDesc) { "√ " } else { "  " },
                        MenuAction::LeftUnsorted => if left.sort_mode == SortMode::Unsorted { "√ " } else { "  " },
                        _ => "  ",
                    }
                } else if *menu_idx == 4 {
                    // Right panel menu
                    match item.action {
                        // View modes
                        MenuAction::RightBrief => if right.mode == PanelMode::Brief { "√ " } else { "  " },
                        MenuAction::RightFull => if right.mode == PanelMode::Full { "√ " } else { "  " },
                        MenuAction::RightInfo => if right.mode == PanelMode::Info { "√ " } else { "  " },
                        MenuAction::RightTree => if right.mode == PanelMode::Tree { "√ " } else { "  " },
                        MenuAction::RightQuickView => if right.mode == PanelMode::QuickView { "√ " } else { "  " },
                        // Sort modes
                        MenuAction::RightSortName => if matches!(right.sort_mode, SortMode::NameAsc | SortMode::NameDesc) { "√ " } else { "  " },
                        MenuAction::RightSortExt => if matches!(right.sort_mode, SortMode::ExtAsc | SortMode::ExtDesc) { "√ " } else { "  " },
                        MenuAction::RightSortTime => if matches!(right.sort_mode, SortMode::TimeAsc | SortMode::TimeDesc) { "√ " } else { "  " },
                        MenuAction::RightSortSize => if matches!(right.sort_mode, SortMode::SizeAsc | SortMode::SizeDesc) { "√ " } else { "  " },
                        MenuAction::RightUnsorted => if right.sort_mode == SortMode::Unsorted { "√ " } else { "  " },
                        _ => "  ",
                    }
                } else {
                    match item.checked {
                        Some(true) => "√ ",
                        Some(false) => "  ",
                        None => "  ",
                    }
                };

                let label_width = 16usize;
                let padded_label = format!("{:<width$}", item.label, width = label_width);
                let shortcut_str = item.shortcut.unwrap_or("");
                let line = format!("{}{} {}", checkmark, padded_label, shortcut_str);

                let item_para = Paragraph::new(Text::from(line)).style(item_style);
                let item_area = ftui::core::geometry::Rect::new(inner.x, inner.y + y_offset, inner.width, 1);
                item_para.render(item_area, frame);
                y_offset += 1;

                // Draw separator after item if needed
                if item.separator_after && y_offset < inner.height {
                    let sep_line = "─".repeat(inner.width as usize);
                    let sep_para = Paragraph::new(Text::from(sep_line))
                        .style(Style::new().fg(theme.dialog_fg).bg(theme.dialog_bg));
                    let sep_area = ftui::core::geometry::Rect::new(inner.x, inner.y + y_offset, inner.width, 1);
                    sep_para.render(sep_area, frame);
                    y_offset += 1;
                }
            }
        }
    }
}

pub fn render_panel(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    pane: &Pane,
    active: bool,
    theme: ThemeColors,
    other_pane: Option<&Pane>,
) -> ftui::core::geometry::Rect {
    match pane.mode {
        PanelMode::Brief => render_panel_brief(frame, area, pane, active, theme),
        PanelMode::Full => render_panel_full(frame, area, pane, active, theme),
        PanelMode::Info => render_panel_info(frame, area, pane, active, theme),
        PanelMode::Tree => render_panel_tree(frame, area, pane, active, theme),
        PanelMode::QuickView => render_quick_view(frame, area, pane, active, theme, other_pane),
    }
}

fn render_panel_brief(
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
    let title = panel_title(pane);
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title(title.as_str());

    // Brief mode: 3 columns of filenames only (NC5 style)
    let inner = block.inner(area);
    block.render(area, frame);

    if pane.entries.is_empty() {
        return area;
    }

    let col_count = 3usize;
    let col_width = inner.width as usize / col_count;
    let rows_per_col = inner.height as usize;
    let state = pane.state.borrow();
    let offset = state.offset;
    let selected_idx = state.selected;
    drop(state);

    let highlight_style = if active {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else {
        Style::new().fg(theme.panel_fg).bg(theme.panel_bg)
    };
    let normal_style = Style::new().fg(theme.panel_fg).bg(theme.panel_bg);
    let marked_style = Style::new().fg(theme.selection_bg).bg(theme.panel_bg);

    for row in 0..rows_per_col {
        for col in 0..col_count {
            let entry_idx = offset + col * rows_per_col + row;
            if entry_idx >= pane.entries.len() {
                continue;
            }
            let entry = &pane.entries[entry_idx];
            let is_marked = pane.selected.contains(&entry.path);
            let is_selected = selected_idx == Some(entry_idx);

            let display_name = if entry.is_dir {
                entry.name.to_uppercase()
            } else {
                entry.name.to_lowercase()
            };
            let marker = if is_marked { "*" } else { " " };
            let name = format!("{}{}", marker, display_name);
            let truncated: String = name.chars().take(col_width.saturating_sub(1)).collect();
            let padded = format!("{:<width$}", truncated, width = col_width);

            let style = if is_selected {
                highlight_style
            } else if is_marked {
                marked_style
            } else {
                normal_style
            };

            let x = inner.x + (col as u16 * col_width as u16);
            let y = inner.y + row as u16;
            if y < inner.y + inner.height {
                let cell_area = ftui::core::geometry::Rect::new(x, y, col_width as u16, 1);
                let para = Paragraph::new(Text::from(padded)).style(style);
                para.render(cell_area, frame);
            }
        }
    }
    area
}

fn render_panel_full(
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
    let title = panel_title(pane);
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title(title.as_str());

    // NC5-style header with sort indicator
    let sort_arrow = sort_indicator(pane.sort_mode);
    let name_header = match pane.sort_mode {
        SortMode::NameAsc | SortMode::NameDesc => format!("{}Name", sort_arrow),
        _ => "Name".to_string(),
    };
    let size_header = match pane.sort_mode {
        SortMode::SizeAsc | SortMode::SizeDesc => format!("{}Size", sort_arrow),
        _ => "Size".to_string(),
    };
    let date_header = match pane.sort_mode {
        SortMode::TimeAsc | SortMode::TimeDesc => format!("{}Date", sort_arrow),
        _ => "Date".to_string(),
    };
    let header = Row::new([name_header, size_header, date_header, "Time".to_string()])
        .style(Style::new().fg(theme.header_fg).bg(theme.header_bg))
        .height(HEADER_HEIGHT);

    let rows = pane
        .entries
        .iter()
        .map(|entry| {
            let is_marked = pane.selected.contains(&entry.path);
            let marker = if is_marked { "*" } else { " " };
            // NC5 style: directories uppercase without brackets, files lowercase
            let display_name = if entry.is_dir {
                entry.name.to_uppercase()
            } else {
                entry.name.to_lowercase()
            };
            let name = format!("{}{}", marker, display_name);
            // NC5 style: ►UP--DIR◄ for parent, ►SUB-DIR◄ for subdirs
            let size = if entry.is_dir {
                if entry.name == ".." {
                    "►UP-DIR◄".to_string()
                } else {
                    "►DIR◄".to_string()
                }
            } else {
                entry.size.to_string()
            };
            let (date, time) = format_time(entry.modified);
            let mut row = Row::new([name, size, date, time]).height(1);
            if entry.is_system {
                row = row.style(Style::new().fg(theme.system_fg).bg(theme.panel_bg));
            }
            if is_marked {
                row = row.style(Style::new().fg(theme.selection_bg).bg(theme.panel_bg));
            }
            row
        })
        .collect::<Vec<_>>();

    let widths = [
        Constraint::Fill,
        Constraint::Fixed(8),
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

fn render_panel_info(
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
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title("Info");

    let inner = block.inner(area);
    block.render(area, frame);

    // Show directory/file info
    let mut lines = Vec::new();
    lines.push(format!("Path: {}", pane.cwd.display()));
    lines.push(String::new());

    let total_files = pane.entries.iter().filter(|e| !e.is_dir).count();
    let total_dirs = pane.entries.iter().filter(|e| e.is_dir).count();
    let total_size: u64 = pane.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    lines.push(format!("Files: {}", total_files));
    lines.push(format!("Directories: {}", total_dirs));
    lines.push(format!("Total size: {} bytes", total_size));

    if !pane.selected.is_empty() {
        lines.push(String::new());
        lines.push(format!("Selected: {} items", pane.selected.len()));
        lines.push(format!("Selected size: {} bytes", pane.selected_total_size()));
    }

    let text = lines.join("\n");
    let para = Paragraph::new(Text::from(text))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg));
    para.render(inner, frame);
    area
}

fn render_panel_tree(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    pane: &Pane,
    active: bool,
    theme: ThemeColors,
) -> ftui::core::geometry::Rect {
    use crate::fs_ops::build_tree;

    let border_color = if active {
        theme.panel_border_active
    } else {
        theme.panel_border_inactive
    };
    let title = format!("Tree - {}", pane.cwd.display());
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title(title.as_str());

    let inner = block.inner(area);
    block.render(area, frame);

    // Build tree from current directory
    let show_hidden = pane.entries.iter().any(|e| e.name.starts_with('.'));
    let tree_items = build_tree(&pane.cwd, 5, show_hidden);

    let state = pane.state.borrow();
    let selected_idx = state.selected.unwrap_or(0);
    let offset = state.offset;
    drop(state);

    let view_height = inner.height as usize;
    let highlight_style = if active {
        Style::new().fg(theme.selection_fg).bg(theme.selection_bg)
    } else {
        Style::new().fg(theme.panel_fg).bg(theme.panel_bg)
    };
    let normal_style = Style::new().fg(theme.panel_fg).bg(theme.panel_bg);

    for (row, item) in tree_items.iter().enumerate().skip(offset).take(view_height) {
        let indent = "  ".repeat(item.depth);
        let name = item
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_uppercase())
            .unwrap_or_else(|| item.path.display().to_string());
        let line = format!("{}{}", indent, name);
        let truncated: String = line.chars().take(inner.width as usize).collect();

        let style = if row == selected_idx { highlight_style } else { normal_style };
        let y = inner.y + (row - offset) as u16;
        if y < inner.y + inner.height {
            let line_area = ftui::core::geometry::Rect::new(inner.x, y, inner.width, 1);
            let para = Paragraph::new(Text::from(truncated)).style(style);
            para.render(line_area, frame);
        }
    }

    area
}

fn render_quick_view(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    _pane: &Pane,
    active: bool,
    theme: ThemeColors,
    other_pane: Option<&Pane>,
) -> ftui::core::geometry::Rect {
    let border_color = if active {
        theme.panel_border_active
    } else {
        theme.panel_border_inactive
    };
    let block = Block::bordered()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border_color))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .title("Quick View");

    let inner = block.inner(area);
    block.render(area, frame);

    // Show preview of selected file in opposite pane
    let Some(other) = other_pane else {
        let para = Paragraph::new(Text::from("No file selected"))
            .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg));
        para.render(inner, frame);
        return area;
    };

    let Some(entry) = other.selected_entry() else {
        let para = Paragraph::new(Text::from("No file selected"))
            .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg));
        para.render(inner, frame);
        return area;
    };

    if entry.is_dir {
        let para = Paragraph::new(Text::from(format!("<DIR> {}", entry.name)))
            .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg));
        para.render(inner, frame);
        return area;
    }

    // Try to read first few lines of the file for preview
    let preview = match std::fs::read_to_string(&entry.path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().take(inner.height as usize).collect();
            lines.join("\n")
        }
        Err(_) => format!("{}\n{} bytes", entry.name, entry.size),
    };

    let para = Paragraph::new(Text::from(preview))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg))
        .wrap(WrapMode::None);
    para.render(inner, frame);
    area
}

fn panel_title(pane: &Pane) -> String {
    if let Some(vfs) = &pane.vfs {
        if vfs.prefix.is_empty() {
            format!("{}:", vfs.zip_path.display())
        } else {
            format!("{}:{}", vfs.zip_path.display(), vfs.prefix)
        }
    } else if pane.panelized.is_some() {
        "Search results".to_string()
    } else {
        pane.cwd.display().to_string()
    }
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
) -> (Option<LayoutCache>, ftui::core::geometry::Rect, ftui::core::geometry::Rect, ftui::core::geometry::Rect) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
    let layout = Flex::vertical().constraints([
        Constraint::Fixed(MENU_HEIGHT),
        Constraint::Fill,
        Constraint::Fixed(STATUS_HEIGHT),
        Constraint::Fixed(CMDLINE_HEIGHT),
        Constraint::Fixed(KEYBAR_HEIGHT),
    ]);
    let areas = layout.split(full);
    let menu_area = areas[0];
    let body_area = areas[1];
    let status_area = areas[2];
    let cmdline_area = areas[3];
    let key_area = areas[4];

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
            left_area = render_panel(frame, col_areas[0], left, active == ActivePane::Left, theme, Some(right));
            right_area = render_panel(frame, col_areas[1], right, active == ActivePane::Right, theme, Some(left));
        } else if !hide_left {
            left_area = render_panel(frame, body_area, left, active == ActivePane::Left, theme, None);
        } else if !hide_right {
            right_area = render_panel(frame, body_area, right, active == ActivePane::Right, theme, None);
        }
        layout_cache = Some(LayoutCache { left_table: left_area, right_table: right_area });
    }

    (layout_cache, status_area, cmdline_area, key_area)
}

pub fn render_cmdline(
    frame: &mut Frame,
    area: ftui::core::geometry::Rect,
    pane: &Pane,
    cmdline: &str,
    theme: ThemeColors,
) {
    // NC5 style command prompt: "C:\NC>" or current directory path
    let prompt = format!("{}> {}", pane.cwd.display(), cmdline);
    let para = Paragraph::new(Text::from(prompt))
        .style(Style::new().fg(theme.panel_fg).bg(theme.panel_bg));
    para.render(area, frame);
}

pub fn render_background(frame: &mut Frame, theme: ThemeColors) {
    let full = ftui::core::geometry::Rect::new(0, 0, frame.width(), frame.height());
    let background = Block::new().style(Style::new().fg(theme.panel_fg).bg(theme.screen_bg));
    background.render(full, frame);
}

pub fn render_status_and_keybar(
    frame: &mut Frame,
    status_area: ftui::core::geometry::Rect,
    cmdline_area: ftui::core::geometry::Rect,
    key_area: ftui::core::geometry::Rect,
    theme: ThemeColors,
    left: &Pane,
    right: &Pane,
    active: ActivePane,
    status: &str,
    cmdline: &str,
) {
    render_status(frame, status_area, left, right, active, status, theme);
    let active_pane = match active {
        ActivePane::Left => left,
        ActivePane::Right => right,
    };
    render_cmdline(frame, cmdline_area, active_pane, cmdline, theme);
    render_keybar(frame, key_area, theme);
}

pub fn render_modal_wrapper(frame: &mut Frame, modal: &Modal, theme: ThemeColors, left: &Pane, right: &Pane) {
    render_modal(frame, modal, theme, left, right);
}
