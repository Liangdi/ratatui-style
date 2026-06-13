## [0.1.2] - 2026-06-13

### 🚀 Features

- *(presets)* Add ratatui-style-presets crate
- *(examples)* Add sizing, data-driven, and strict-mode demos
- *(presets)* Add 02_gallery example, replace showcase

### 🐛 Bug Fixes

- *(box-model)* Honor var() fallback for Length, matching Color
- *(cascade)* Resolve var() in border colors

### 🚜 Refactor

- *(examples)* Use zero-alloc NodeRef/compute_with in draw loops

### 📚 Documentation

- *(readme)* Sync English README with Chinese version

### ⚙️ Miscellaneous Tasks

- Add CHANGELOG
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
- Release ratatui-style version 0.1.1
