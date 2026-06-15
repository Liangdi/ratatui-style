//! P6 — `@supports` feature detection.
//!
//! `@supports` gates rules on **capability**, evaluated against the same
//! [`MediaContext`] as `@media` — but it asks "does this terminal *support* X?"
//! rather than "is this terminal *currently* sized X?". Conditions:
//!
//! - `(truecolor)` / `not (truecolor)` — 24-bit color capability
//! - `(color)` / `(monochrome)` / `(no-color)` — color enabled / disabled
//! - `(border-style)`, `(padding)`, … — does the engine know this property?
//! - `(border-style: rounded)` — does the engine know this property *value*?
//!
//! The canonical use: ship a vivid palette for truecolor terminals and a safe
//! 16-color fallback from the same stylesheet, decided at compute time. Here
//! the `Card`'s own `color` is set differently under each branch, so the
//! resolved foreground visibly changes between contexts.
//!
//! ```sh
//! cargo run -p ratatui-style --example 19_supports
//! ```

use ratatui::style::Style;

use ratatui_style::{ComputeScratch, MediaContext, OwnedNode, Stylesheet};

const SHEET: &str = r##"
:root {
    --text:   #cdd6f4;
    --panel:  #313244;
}

Root   { background: #1e1e2e; color: var(--text); }
Card   { background: var(--panel); }

/* Truecolor terminal: vivid pink text + double frame. */
@supports (truecolor) {
    Card  { color: #f5c2e7; border: double #f5c2e7; }
}

/* 16-color fallback: named color + plain border — no truecolor needed. */
@supports not (truecolor) {
    Card  { color: magenta; border: plain magenta; }
}

/* Monochrome (NO_COLOR): a neutral that reads without color. */
@supports (no-color) {
    Card  { color: gray; }
}

/* Property-capability gate: `border-style: rounded` only if the engine knows
 * that value (it does) — a guard pattern for engine-portable CSS. */
@supports (border-style: rounded) {
    Card { border-style: rounded; }
}
"##;

fn show(label: &str, s: Style, ctx: &str) {
    println!("  [{ctx}] {label:18} fg={:<14?} bg={:<14?}", s.fg, s.bg);
}

fn main() {
    let sheet = Stylesheet::parse(SHEET).expect("parse");
    let node = OwnedNode::new("Card");
    let mut scratch = ComputeScratch::new();

    println!("Same stylesheet, three capability contexts — note the fg:\n");

    // Truecolor terminal → pink (#f5c2e7 as Rgb).
    let truecolor = MediaContext { truecolor: true, no_color: false, ..Default::default() };
    let c1 = sheet.compute_with_media(&node, None, &mut scratch, &truecolor);
    show("Card", c1.to_style(), "truecolor ");

    // 16-color terminal → magenta (named Color::Magenta).
    let sixteen = MediaContext { truecolor: false, no_color: false, ..Default::default() };
    let c2 = sheet.compute_with_media(&node, None, &mut scratch, &sixteen);
    show("Card", c2.to_style(), "16-color ");

    // Monochrome (NO_COLOR) → gray.
    let mono = MediaContext { truecolor: false, no_color: true, ..Default::default() };
    let c3 = sheet.compute_with_media(&node, None, &mut scratch, &mono);
    show("Card", c3.to_style(), "NO_COLOR ");

    println!();
    println!("The fg flips between Rgb pink, Magenta, and Gray — one stylesheet,");
    println!("chosen at compute time from the terminal's capabilities. `@supports`");
    println!("and `@media` evaluate against the same MediaContext, so a single");
    println!("sheet adapts to both *size* (@media) and *capability* (@supports).");
}
