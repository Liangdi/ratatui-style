//! P4/P5 — The opt-in compute cache (access-order LRU + generation invalidation).
//!
//! Computing a node's style walks every matching rule, overlays them, resolves
//! `var()` and inheritance. For a stable tree rendered every frame, that's
//! wasted work. Attach a [`ComputeCache`] and a repeat compute of an identical
//! (node, parent, media) signature is a hash hit.
//!
//! There are two ways to wire the cache:
//!
//! - **`CascadeContext::with_cache(n)`** — the tree-walking API keeps the cache
//!   across `enter` calls within one context's lifetime. Best for a render loop
//!   whose stylesheet is stable for the frame.
//! - **`Stylesheet::compute_cached(...)`** — the one-shot API. The host owns the
//!   `ComputeCache` and carries it across frames, surviving even stylesheet
//!   mutations, because the cache self-invalidates on a generation mismatch.
//!
//! This example uses the **one-shot** path so it can also show generation
//! invalidation: a `CascadeContext` borrows the sheet immutably for its whole
//! life, so to mutate the sheet you must first drop the context. Carrying a
//! standalone `ComputeCache` (which the one-shot API takes by `&mut`) sidesteps
//! that — exactly how a real host holds the cache across frames.
//!
//! ```sh
//! cargo run -p ratatui-style --example 20_cache
//! ```

use ratatui_style::media::MediaContext;
use ratatui_style::{ComputeCache, ComputeScratch, CssStyle, Origin, OwnedNode, Stylesheet};

/// Walk a fixed 5-node tree (Root → Panel → 3 Rows) through the one-shot
/// cached path, returning the number of distinct signatures the cache holds.
fn walk(sheet: &Stylesheet, cache: &mut ComputeCache, scratch: &mut ComputeScratch) -> usize {
    let media = MediaContext::default();
    let mut sigs: Vec<u64> = Vec::new();

    // Root: no parent → parent_sig = None.
    let (root, root_sig) = sheet.compute_cached(&OwnedNode::new("Root"), None, None, &media, scratch, cache);
    sigs.push(root_sig);

    // Panel: parent = root's computed style (its signature is the parent_sig).
    let (panel, panel_sig) =
        sheet.compute_cached(&OwnedNode::new("Panel"), Some(&root), Some(root_sig), &media, scratch, cache);
    sigs.push(panel_sig);

    // Three rows under the panel — distinct classes ⇒ distinct signatures.
    for i in 0..3 {
        let (_row, row_sig) = sheet.compute_cached(
            &OwnedNode::new("Row").with_classes([format!("r{i}")]),
            Some(&panel),
            Some(panel_sig),
            &media,
            scratch,
            cache,
        );
        sigs.push(row_sig);
    }
    // Distinct signatures = distinct cache slots.
    sigs.sort_unstable();
    sigs.dedup();
    sigs.len()
}

fn main() {
    let mut sheet = Stylesheet::new();
    sheet
        .add("Panel", CssStyle::new().color("#cdd6f4").background("#313244"), Origin::User)
        .unwrap();
    for i in 0..3 {
        sheet
            .add(
                &format!("Row.r{i}"),
                CssStyle::new().color(if i % 2 == 0 { "#89b4fa" } else { "#a6e3a1" }),
                Origin::User,
            )
            .unwrap();
    }
    let mut scratch = ComputeScratch::new();

    println!("Capacity 8 cache — the 5-node tree produces 5 distinct signatures:\n");
    {
        let mut cache = ComputeCache::new(8);
        walk(&sheet, &mut cache, &mut scratch);
        println!("  after first  walk → cache.len() = {}  (all misses, filled)", cache.len());
        walk(&sheet, &mut cache, &mut scratch);
        println!("  after second walk → cache.len() = {}  (all hits, no growth)", cache.len());
    }

    println!("\nCapacity 2 cache — LRU evicts to stay ≤ 2:\n");
    {
        let mut cache = ComputeCache::new(2);
        walk(&sheet, &mut cache, &mut scratch);
        println!("  walked 5 nodes with cap 2 → cache.len() = {}  (bounded)", cache.len());
    }

    println!("\nGeneration invalidation — mutating the sheet clears the cache:\n");
    {
        let mut cache = ComputeCache::new(8);
        walk(&sheet, &mut cache, &mut scratch);
        println!("  generation = {}, cache.len() = {}", sheet.generation(), cache.len());

        // Any mutation bumps the generation. The cache detects the mismatch on
        // its NEXT access and self-clears — no manual invalidation needed.
        sheet
            .add("Footer", CssStyle::new().color("#6c7086"), Origin::User)
            .unwrap();
        println!("  after sheet.add(...) → generation = {} (bumped)", sheet.generation());

        walk(&sheet, &mut cache, &mut scratch);
        println!("  next walk rebuilt cold → cache.len() = {}", cache.len());
    }

    println!();
    println!("In a render loop: keep one ComputeCache + ComputeScratch across");
    println!("frames, call compute_cached(...) each frame, and the steady-state");
    println!("per-frame work shrinks to hash hits for any node whose");
    println!("(identity, parent, media) is unchanged since last frame.");
}
