# ratatui-style-presets

[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Ready-to-use CSS themes & utilities for **[ratatui-style]** — the styling layer
pre-filled, so a ratatui app gets a sensible look **out of the box** and can
swap looks by changing one stylesheet.

Each preset is embedded at compile time and exposed as a `&'static Stylesheet`
parsed with `Origin::Theme`, so a downstream app overrides any of it with its
own `Origin::User` rules at equal specificity — no merge plumbing required.

[ratatui-style]: https://crates.io/crates/ratatui-style

## Quick start

```toml
[dependencies]
ratatui-style = "0.1"
ratatui-style-presets = { version = "0.1", features = ["widgets", "catppuccin"] }
```

```rust
use ratatui_style::NodeRef;
use ratatui_style_presets::{merge, Preset};

// Stack the default theme + widget defaults + a palette into one sheet.
let sheet = merge(&[Preset::Default, Preset::Widgets, Preset::Catppuccin]);

let node = NodeRef::new("Button").classes(&["primary"]);
let computed = sheet.compute(&node, None);
let _ratatui_style = computed.to_style();
let _block       = computed.to_block();
```

## Swappability

Every theme fills the **same** canonical semantic tokens:

```
--bg --surface --surface-2 --border
--text --text-muted
--accent --accent-fg --success --warning --danger --info
```

`default_theme()` (always available) defines these **plus** base component
classes (`Button`, `Panel`, `Text`, `List`, `Badge`, …) that reference the
tokens through `var()`. The `widgets` preset does the same for ratatui widget
type names. So: pick a palette → restyle everything; layer utilities/widgets →
styled widgets; override anything with your own `Origin::User` rules.

## Feature flags

| Feature      | Preset                                       |
|--------------|----------------------------------------------|
| _(none)_     | `default_theme()` — always available         |
| `tailwind`   | `tailwind()` — atomic utility classes        |
| `widgets`    | `widgets()` — ratatui widget type defaults   |
| `catppuccin` | `catppuccin()` — Catppuccin (Mocha) palette  |
| `nord`       | `nord()` — Nord palette                      |
| `dracula`    | `dracula()` — Dracula palette                |

All features are opt-in; `default = []`.

## Combine presets

```rust
use ratatui_style_presets::{merge, Preset, PresetBuilder};

let sheet = merge(&[Preset::Default, Preset::Widgets, Preset::Catppuccin]);

// Same thing, fluently:
let sheet = PresetBuilder::new()
    .with(Preset::Default)
    .with(Preset::Widgets)
    .build();
```

## Example

```sh
cargo run -p ratatui-style-presets --example showcase
cargo run -p ratatui-style-presets --example showcase --all-features
```

## License

MIT
