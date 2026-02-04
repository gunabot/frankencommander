# FrankenCommander (fc)

NC 5.0-inspired file manager built on FrankenTUI.

## Run

```bash
cargo run
```

Requires Rust nightly (FrankenTUI uses nightly + edition 2024).

## Highlights

- Two-pane NC-style layout with classic colors
- ZIP drill-in (open `.zip` like a directory, view files)
- Find + panelize (Ctrl+P in results)
- Drive menus (Alt+F1 / Alt+F2) mapped to `/`, `/home`, `/tmp`, `/mnt/*`, `/media/*`
- User menu (F2) backed by `~/.frankencommander/usermenu.txt`

## Keys

- `Tab` switch panes
- Arrow keys navigate
- `Enter` open directory or ZIP
- `Backspace` up directory or exit panelized view
- `F1` help
- `F2` user menu
- `F3` view file
- `F4` edit file
- `F5` copy
- `F6` move
- `F7` mkdir
- `F8` delete
- `F9` menu
- `F10` quit
- `F11` attributes (chmod octal)
- `Alt+F1` / `Alt+F2` drive menu
- `Ctrl+F1` / `Ctrl+F2` hide left/right panel
- `Ctrl+O` command-line-only view
- `Ctrl+F8` sync dirs (active → inactive)
- `Ctrl+P` panelize from Find results

## FrankenTUI

Built on the FrankenTUI runtime, widgets, and renderer.
