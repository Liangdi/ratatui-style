## [Unreleased]

### 🚀 Features

- *(presets)* New workspace member `ratatui-style-presets` — ready-to-use CSS
  themes & utilities built on the engine: default theme + base component
  classes, Tailwind-style atomic utilities, ratatui widget-type defaults, and
  Catppuccin/Nord/Dracula palettes. All themes fill one canonical semantic-token
  vocabulary so swapping the base stylesheet restyles a whole UI. Per-preset
  feature flags, `&'static Stylesheet` accessors, and a `Preset`/`merge`/
  `PresetBuilder` composition API. Includes a preset gallery example
  (`02_gallery`) that browses every preset and can restyle the whole frame with
  the active palette.

## [0.1.1] - 2026-06-13

### 🚀 Features

- Prepare for crates.io publishing and rewrite README
- *(style)* Composable border-style/border-color + tailwind example
- Add css!/scss! macros, RuntimeStyle, and SCSS feature
- *(examples)* Add runtime stylesheet-switching demo
- [**breaking**] DX overhaul — render bridge, diagnostics, zero-alloc compute, per-edge borders
- *(examples)* Add 00_hello_world — minimal TUI first-touch

### 🚜 Refactor

- *(style)* Unify border-merge and keyword-enum parse behind single source

### 📚 Documentation

- *(readme)* Add hello-world screenshot, stack gallery vertically

### ⚙️ Miscellaneous Tasks

- Init
- *(scripts)* Add PTY-driven TUI capture tool
