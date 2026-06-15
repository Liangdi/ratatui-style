//! P3/P5/P6 — Structural pseudo-classes.
//!
//! These consume [`StyledNode::position`] — where an element sits among its
//! siblings. The host supplies a [`Position`] (index, sibling_count, and for the
//! of-type variants, the same-type counts). The engine does the matching:
//!
//! | Roadmap | Selector | Meaning |
//! |---------|----------|---------|
//! | P3 | `:nth-child(odd)` / `:nth-child(2n+1)` | every other sibling |
//! | P3 | `:first-child` / `:last-child` | the first / last sibling |
//! | P3 | `:only-child` | the sole sibling |
//! | P5 | `:nth-of-type(1)` / `:first-of-type` | first among same-type siblings |
//! | P6 | `:last-of-type` / `:only-of-type` / `:nth-last-of-type` | tail / sole same-type |
//!
//! A default `Position` (`sibling_count == 0`) means "no sibling info" and does
//! **not** match — the host opts in by supplying real counts.
//!
//! This example renders zebra-striped rows, with the first/last row and the
//! single section header styled by structure alone.
//!
//! ```sh
//! cargo run -p ratatui-style --example 18_structural
//! ```

use ratatui::style::Style;

use ratatui_style::{OwnedNode, Position, Stylesheet};

const SHEET: &str = r##"
:root {
    --text:  #cdd6f4;
    --row-a: #1e1e2e;
    --row-b: #313244;
    --accent: #89b4fa;
    --good: #a6e3a1;
    --warn: #fab387;
}

List      { color: var(--text); }
Row       { color: var(--text); }   /* base text color every row inherits */

/* zebra striping: odd-indexed children get the alt background */
Row:nth-child(odd)  { background: var(--row-b); }
Row:nth-child(even) { background: var(--row-a); }

/* structural edges — no class needed */
Row:first-child { color: var(--accent); font-weight: bold; }
Row:last-child  { color: var(--warn); }

/* a lone child among same-type siblings: the section header is the only Header */
Header:only-of-type    { color: var(--good); font-weight: bold; }

/* :first-of-type / :last-of-type — first Section accented, last warned */
Section:first-of-type  { color: var(--accent); }
Section:last-of-type   { color: var(--warn); }
"##;

fn show(label: &str, s: Style) {
    println!("  {label:34} fg={:<13?} bg={:<13?} mod={:?}", s.fg, s.bg, s.add_modifier);
}

/// Build a node with sibling-position info. `i`/`n` are index/count among ALL
/// siblings; `oti`/`otn` are the same-type counts (for the `-of-type` pseudos).
fn node(type_name: &str, i: usize, n: usize, oti: usize, otn: usize) -> OwnedNode {
    OwnedNode::new(type_name).with_position(
        Position::new(i, n).with_of_type(oti, otn),
    )
}

fn main() {
    let sheet = Stylesheet::parse(SHEET).expect("parse");

    // A list of 4 rows + a single header + 2 sections. The structural pseudos
    // style them purely from position — no classes on the elements themselves.
    let total = 6usize; // 4 Rows among 6 siblings (rows interleaved conceptually)

    println!("Zebra rows (nth-child) + edges (first/last-child):\n");
    // Render rows 0..4 as siblings of a 6-wide list.
    for i in 0..4 {
        let row = sheet.compute(&node("Row", i, total, i, 4), None);
        show(&format!("Row #{i}"), row.to_style());
    }

    println!("\nA lone Header among rows → :only-of-type:\n");
    let header = sheet.compute(&node("Header", 0, total, 0, 1), None);
    show("Header (only-of-type)", header.to_style());

    println!("\nSections → first-of-type / last-of-type:\n");
    // Two Sections among same-type siblings; structure alone picks accent vs warn.
    let s1 = sheet.compute(&node("Section", 0, total, 0, 2), None);
    let s2 = sheet.compute(&node("Section", 1, total, 1, 2), None);
    show("Section #1 (first-of-type)", s1.to_style());
    show("Section #2 (last-of-type)", s2.to_style());

    println!();
    println!("No classes were set on any node — every visual difference above");
    println!("comes from the node's Position (index / count / of-type counts).");
    println!("Drop the `.with_position(...)` and the structural pseudos stop");
    println!("matching (no zebra bg, no first/last coloring — only the base):");
    let no_pos = sheet.compute(&OwnedNode::new("Row"), None);
    show("Row WITHOUT position", no_pos.to_style());
}
