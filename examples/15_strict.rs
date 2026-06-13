//! Strict mode catches errors — `Stylesheet::parse_strict` reports typos.
//!
//! Lenient parse (`Stylesheet::parse`) silently drops unknown properties and
//! undefined variables, which is fine for live apps but can hide mistakes
//! during development. Strict parse (`Stylesheet::parse_strict`) returns a
//! `CssError` with `Loc` (line:column) so you can fix CSS typos early.
//!
//! ```sh
//! cargo run -p ratatui-style --example 15_strict
//! ```

use ratatui::style::Style;
use ratatui_style::{OwnedNode, Stylesheet};

fn show(label: &str, s: Style) {
    println!("  {label:34} fg={:<14?} bg={:<14?} mod={:?}", s.fg, s.bg, s.add_modifier);
}

fn main() {
    let css_with_errors = r##"
        :root {
            --accent: #00d4ff;
        }

        /* Typo: "colour" instead of "color" */
        Button   { colour: red; }

        /* Undefined variable with no fallback */
        Text     { background: var(--nope); }

        /* This one is valid */
        #save    { color: yellow; }
        "##;

    println!("=== Lenient parse (Stylesheet::parse) ===");
    println!("Typos and undefined vars are silently dropped:\n");

    match Stylesheet::parse(css_with_errors) {
        Ok(sheet) => {
            println!("✓ Parsed successfully (errors ignored)\n");

            // Show what actually resolved (typos dropped).
            let cases = [
                ("Button (typo'd colour ignored)", OwnedNode::new("Button")),
                ("Text (undefined var ignored)", OwnedNode::new("Text")),
                ("#save (valid)", OwnedNode::new("Button").with_id("save")),
            ];

            println!("Resolved styles:");
            for (label, node) in cases {
                let computed = sheet.compute(&node, None);
                show(label, computed.to_style());
            }

            println!("\nNote: Button's `colour: red` was dropped entirely — no color applied.");
            println!("Text's `var(--nope)` degraded to Reset (transparent background).");
        }
        Err(e) => {
            println!("✗ Unexpected error: {e}");
        }
    }

    println!("\n\n=== Strict parse (Stylesheet::parse_strict) ===");
    println!("Typos and undefined vars produce errors with line:column:\n");

    match Stylesheet::parse_strict(css_with_errors) {
        Ok(_) => {
            println!("✓ Parsed successfully (should not happen — CSS has errors)");
        }
        Err(e) => {
            println!("✗ CssError caught:");
            println!("    Kind: {:?}", e.kind);
            if let Some(loc) = e.loc {
                println!("    Loc:  line {}, column {}", loc.line, loc.column);
            } else {
                println!("    Loc:  <none>");
            }
            println!("    Full error: {e}\n");

            println!("Strict mode surfaces typos early during development.");
            println!("Use lenient mode (default `parse`) in production to degrade");
            println!("gracefully when server-provided CSS has minor issues.");
        }
    }

    println!("\n=== Recommendation ===");
    println!("• Development: use `parse_strict` to catch typos");
    println!("• Production: use `parse` to degrade gracefully");
}
