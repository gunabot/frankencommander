#![forbid(unsafe_code)]

use crate::model::{MenuAction, MenuItem};

pub const MENU_TITLES: [&str; 6] = ["File", "Command", "Options", "Left", "Right", "Help"];

pub fn menu_items(menu_idx: usize) -> &'static [MenuItem] {
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
