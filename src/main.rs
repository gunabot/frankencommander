#![forbid(unsafe_code)]

mod app;
mod fs_ops;
mod menu;
mod model;
mod pane;
mod ui;
mod vfs;

fn main() -> std::io::Result<()> {
    app::App::run()
}
