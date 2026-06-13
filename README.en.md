# ratatui-style

[![crates.io](https://img.shields.io/crates/v/ratatui-style.svg)](https://crates.io/crates/ratatui-style)
[![docs.rs](https://docs.rs/ratatui-style/badge.svg)](https://docs.rs/ratatui-style)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**[中文](README.md)**

A CSS cascade engine for [ratatui](https://ratatui.rs) — selectors, specificity,
inheritance, pseudo-states, and data-driven styling. It produces **native
ratatui `Style` / `Block` / `Constraint` values**; it is never a parallel
rendering stack.

It speaks CSS property names (`color`, `background-color`, `font-weight`,
`border`, `padding`, `margin`, `text-align`, `width`, …), is `serde`-friendly
(so server-driven UIs can ship style over the wire), and implements a pragmatic
cascade: **origin × specificity × inheritance × pseudo-states**.

## Screenshots

<p align="center">
  <a href="examples/00_hello_world.rs"><b>00_hello_world</b> · minimal first render</a><br>
  <img src="screenshot/hello-world.png" alt="hello world">
</p>

<p align="center">
  <a href="examples/06_tailwind.rs"><b>06_tailwind</b> · utility-class design system</a><br>
  <img src="screenshot/tailwind-style.png" alt="tailwind style">
</p>

<p align="center">
  <a href="examples/07_scifi_hud.rs"><b>07_scifi_hud</b> · cyberpunk HUD</a><br>
  <img src="screenshot/sci-fi-hud.png" alt="sci-fi HUD">
</p>

See [Examples](#examples) below for the full list.

## Quick start

```rust
use ratatui_style::{CssStyle, Origin, OwnedNode, Stylesheet};

let mut sheet = Stylesheet::new();
sheet.add(
    "Button.primary",
    CssStyle::new().color("#fff").background("blue").bold(),
    Origin::User,
)?;

let node = OwnedNode::new("Button").with_classes(["primary"]);
let computed = sheet.compute(&node, None);

// Project onto native ratatui:
let _style   = computed.to_style();    // → ratatui::style::Style
let _block   = computed.to_block();    // → ratatui::widgets::Block
let _area    = computed.apply_margin(area); // shrinks a Rect
let _layout  = computed.constraints(); // → (Constraint, Constraint)
let _align   = computed.alignment();   // → Alignment
```

## CSS text stylesheets

```rust
use ratatui_style::Stylesheet;

let sheet = Stylesheet::parse(r#"
    :root { --accent: #00d4ff; }

    Button.primary {
        color: var(--accent);
        background: blue;
        font-weight: bold;
        border: rounded;
        padding: 0 2;
    }
    Button:focus { background: green; }
    #save:disabled { color: gray; }
"#)?;
```

## Cascade model

The cascade resolves styles per element in five steps:

1. **Collect** all rules whose selector matches the node.
2. **Sort** ascending by `(origin, specificity, source_order)`.
3. **Overlay** declarations — later rules replace earlier ones field-by-field.
4. **Inherit** — inheritable properties (`color`, `font-weight`, `font-style`,
   `text-decoration`, `underline-color`, `text-align`) flow from the parent's
   computed style into `None` fields on the child.
5. **Resolve** `var()` references against the token table.

### Origin layers

Rules are layered by origin; higher origins override lower ones at equal
specificity:

| Origin | Priority | Use for |
|---|---|---|
| `UserAgent` | lowest | Built-in defaults |
| `Theme` | | Application-wide theme |
| `User` | | End-user config / CSS text |
| `Inline` | highest | Per-element inline style |

### Specificity

`(ids, classes + pseudos, type)` — standard CSS specificity as a comparable
tuple. `*` (universal) is `(0, 0, 0)`.

## Supported CSS properties

| Property | Value type | Maps to |
|---|---|---|
| `color` | [Color](#color-syntax) | `Style::fg` |
| `background` / `background-color` | Color | `Style::bg` / `Block::style` |
| `font-weight` | `bold` / `normal` / `100`–`900` | `Modifier::BOLD` |
| `font-style` | `italic` / `normal` | `Modifier::ITALIC` |
| `text-decoration` | `underline` / `line-through` / both | `Modifier::UNDERLINED` / `CROSSED_OUT` |
| `underline-color` | Color | `Style::underline_color` |
| `border` | `none` / `single` / `rounded` / `double` / `thick` [color] | `Block::borders` + `border_type` |
| `padding` | `1` / `1 2` / `1 2 3` / `1 2 3 4` | `Block::padding` |
| `margin` | same shorthand as padding | `Rect` shrink |
| `text-align` | `left` / `center` / `right` | `Alignment` |
| `width` / `height` | `auto` / `10` / `50%` / `min(3)` / `max(5)` | `Constraint` |

## Color syntax

All color properties accept:

| Syntax | Example |
|---|---|
| Hex 3/4/6/8 | `#fff` `#fff0` `#ff8800` `#ff8800ff` |
| `rgb()` / `rgba()` | `rgb(255, 128, 0)` `rgba(0,0,0,0.5)` |
| Named CSS colors | `red` `blue` `cyan` `orange` `gold` … |
| `transparent` / `none` / `reset` | resets to terminal default |
| `inherit` | forces inheritance from parent |
| `var(--name)` | CSS custom property, with optional fallback: `var(--accent, #fff)` |

## Selectors & pseudo-classes

Compound selectors of the form `Type.class#id:pseudo…`, plus comma lists and
the `*` universal selector:

```
Button              /* type */
.primary            /* class */
#save               /* id */
Button.primary:focus  /* compound */
Text, .muted, #title  /* comma list */
*                   /* universal */
```

Pseudo-classes: `:focus` `:hover` `:disabled` `:checked` `:active`

## Inheritance & `var()`

Color, font-weight, font-style, text-decoration, underline-color, and
text-align inherit from the parent's computed style. `var(--name)` resolves
against the `:root` token table (or a `ThemeTokens` built programmatically /
from themekit).

```rust
use ratatui_style::{CssStyle, Origin, OwnedNode, Stylesheet};

let mut sheet = Stylesheet::new();
sheet.tokens_mut().insert("accent", "#00d4ff");

sheet.add("Panel", CssStyle::new().color("#cdd6f4").italic(), Origin::Theme)?;
sheet.add("Button", CssStyle::new().background("var(--accent)").bold(), Origin::User)?;
sheet.add("Button:disabled", CssStyle::new().color("gray"), Origin::User)?;

// Panel resolves its own style.
let panel = sheet.compute(&OwnedNode::new("Panel"), None);

// Text inherits color + italic from panel.
let text = sheet.compute(&OwnedNode::new("Text"), Some(&panel));

// Disabled button: :disabled rule applies, color=gray.
let btn = sheet.compute(
    &OwnedNode::new("Button").with_state(ratatui_style::State::disabled()),
    Some(&panel),
);
```

## Framework integration

Implement `StyledNode` on your node type — the engine knows nothing about your
framework:

```rust
use ratatui_style::{StyledNode, State, Position};

impl StyledNode for MyNode {
    fn type_name(&self) -> &str { &self.kind }
    fn id(&self) -> Option<&str> { self.id.as_deref() }
    fn classes(&self) -> Vec<&str> { self.classes.iter().map(String::as_str).collect() }
    fn state(&self) -> State { self.state }
    fn position(&self) -> Position { self.position.clone() }
}
```

## Features

| Feature | Default | Adds |
|---|---|---|
| `serde` | ✅ | `Serialize`/`Deserialize` for all value types — JSON property maps, config files, wire format |
| `themekit` | ❌ | `ThemeTokens::from_themekit` — bridge `ratatui-themekit` semantic slots to CSS `var()` tokens |

Disable default features for a pure, zero-dep style engine:

```toml
[dependencies]
ratatui-style = { version = "0.1", default-features = false }
```

## Examples

```sh
# Hello, World! — minimal: one CSS rule → render "Hello, World!"
cargo run --example 00_hello_world

# Interactive dashboard — all CSS, single stylesheet
cargo run --example 05_dashboard

# Cascade demo — inheritance, var(), specificity, pseudo-states
cargo run --example 03_cascade

# CSS text stylesheet parsing
cargo run --example 02_stylesheet

# Color & value parsing
cargo run --example 01_values

# css! macro — compile-time embedding + runtime override
cargo run --example 09_runtime_override

# scss! macro — compile-time SCSS embedding (requires the scss feature)
cargo run --example 10_scss_embed --features scss

# themekit bridge (requires the themekit feature)
cargo run --example 11_themekit_bridge --features themekit
```

## Position in the ecosystem

| Crate | Role | `ratatui-style` |
|---|---|---|
| `ratatui-themekit` | 15 semantic color slots + palettes | **composes** — `ThemeTokens::from_themekit` seeds CSS variables |
| `tui-theme-builder` | compile-time `Style` macro | `ratatui-style` covers the **runtime/config-driven** case |
| `lipgloss` | "CSS for terminals" (own stack) | same DX, on ratatui's buffer model |

## Status

Implemented: CSS text parser, compound selectors, specificity, cascade layers
(`UserAgent` < `Theme` < `User` < `Inline`), pseudo-states, `var()` with
fallback, inheritance, box model (`padding` / `margin` / `border`), sizing
(`width` / `height` → `Constraint`), `serde` integration, and `themekit`
bridge.

Future work: descendant/child combinators (`A B`, `A > B`), `:nth-child`,
`@media`, and a `ComputedStyle` cache.

## License

MIT
