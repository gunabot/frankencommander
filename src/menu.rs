#![forbid(unsafe_code)]

use crate::model::{MenuAction, MenuItem};

pub const MENU_TITLES: [&str; 6] = ["File", "Command", "Options", "Left", "Right", "Help"];

pub fn menu_items(menu_idx: usize) -> &'static [MenuItem] {
    match menu_idx {
        0 => &[
            MenuItem { label: "View", action: MenuAction::View, shortcut: Some("F3"), checked: None, separator_after: false },
            MenuItem { label: "Edit", action: MenuAction::Edit, shortcut: Some("F4"), checked: None, separator_after: false },
            MenuItem { label: "Copy", action: MenuAction::Copy, shortcut: Some("F5"), checked: None, separator_after: false },
            MenuItem { label: "Move", action: MenuAction::Move, shortcut: Some("F6"), checked: None, separator_after: true },
            MenuItem { label: "Quit", action: MenuAction::Quit, shortcut: Some("F10"), checked: None, separator_after: false },
        ],
        1 => &[
            MenuItem { label: "Directory tree", action: MenuAction::Tree, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Find file", action: MenuAction::Find, shortcut: Some("Alt+F7"), checked: None, separator_after: false },
        ],
        2 => &[
            MenuItem { label: "Configuration", action: MenuAction::Config, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Panel options", action: MenuAction::PanelOptions, shortcut: None, checked: None, separator_after: false },
        ],
        3 => &[
            // Panel view modes
            MenuItem { label: "Brief", action: MenuAction::LeftBrief, shortcut: Some("Ctrl+1"), checked: None, separator_after: false },
            MenuItem { label: "Full", action: MenuAction::LeftFull, shortcut: Some("Ctrl+2"), checked: None, separator_after: false },
            MenuItem { label: "Info", action: MenuAction::LeftInfo, shortcut: Some("Ctrl+3"), checked: None, separator_after: false },
            MenuItem { label: "Tree", action: MenuAction::LeftTree, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Quick view", action: MenuAction::LeftQuickView, shortcut: Some("Ctrl+4"), checked: None, separator_after: false },
            MenuItem { label: "On/Off", action: MenuAction::LeftOnOff, shortcut: Some("Ctrl+F1"), checked: None, separator_after: true },
            // Sort modes
            MenuItem { label: "Name", action: MenuAction::LeftSortName, shortcut: Some("Ctrl+F3"), checked: None, separator_after: false },
            MenuItem { label: "Extension", action: MenuAction::LeftSortExt, shortcut: Some("Ctrl+F4"), checked: None, separator_after: false },
            MenuItem { label: "Time", action: MenuAction::LeftSortTime, shortcut: Some("Ctrl+F5"), checked: None, separator_after: false },
            MenuItem { label: "Size", action: MenuAction::LeftSortSize, shortcut: Some("Ctrl+F6"), checked: None, separator_after: false },
            MenuItem { label: "Unsorted", action: MenuAction::LeftUnsorted, shortcut: Some("Ctrl+F7"), checked: None, separator_after: true },
            // Other actions
            MenuItem { label: "Re-read", action: MenuAction::LeftReread, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Filter...", action: MenuAction::LeftFilter, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Drive...", action: MenuAction::LeftDrive, shortcut: Some("Alt+F1"), checked: None, separator_after: false },
        ],
        4 => &[
            // Panel view modes
            MenuItem { label: "Brief", action: MenuAction::RightBrief, shortcut: Some("Ctrl+1"), checked: None, separator_after: false },
            MenuItem { label: "Full", action: MenuAction::RightFull, shortcut: Some("Ctrl+2"), checked: None, separator_after: false },
            MenuItem { label: "Info", action: MenuAction::RightInfo, shortcut: Some("Ctrl+3"), checked: None, separator_after: false },
            MenuItem { label: "Tree", action: MenuAction::RightTree, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Quick view", action: MenuAction::RightQuickView, shortcut: Some("Ctrl+4"), checked: None, separator_after: false },
            MenuItem { label: "On/Off", action: MenuAction::RightOnOff, shortcut: Some("Ctrl+F2"), checked: None, separator_after: true },
            // Sort modes
            MenuItem { label: "Name", action: MenuAction::RightSortName, shortcut: Some("Ctrl+F3"), checked: None, separator_after: false },
            MenuItem { label: "Extension", action: MenuAction::RightSortExt, shortcut: Some("Ctrl+F4"), checked: None, separator_after: false },
            MenuItem { label: "Time", action: MenuAction::RightSortTime, shortcut: Some("Ctrl+F5"), checked: None, separator_after: false },
            MenuItem { label: "Size", action: MenuAction::RightSortSize, shortcut: Some("Ctrl+F6"), checked: None, separator_after: false },
            MenuItem { label: "Unsorted", action: MenuAction::RightUnsorted, shortcut: Some("Ctrl+F7"), checked: None, separator_after: true },
            // Other actions
            MenuItem { label: "Re-read", action: MenuAction::RightReread, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Filter...", action: MenuAction::RightFilter, shortcut: None, checked: None, separator_after: false },
            MenuItem { label: "Drive...", action: MenuAction::RightDrive, shortcut: Some("Alt+F2"), checked: None, separator_after: false },
        ],
        _ => &[
            MenuItem { label: "Help", action: MenuAction::Help, shortcut: Some("F1"), checked: None, separator_after: true },
            MenuItem { label: "About", action: MenuAction::About, shortcut: None, checked: None, separator_after: false },
        ],
    }
}
