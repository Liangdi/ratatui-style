## [0.2.0] - 2026-06-15

A breaking release: the P3 roadmap lands (structural pseudo-classes, descendant/child
combinators, `@media` queries) alongside a serde cross-format overhaul and cascade
performance work.

### 🚀 Features

- *(selector)* Structural pseudo-classes — `:nth-child`, `:nth-last-child`,
  `:first-child`, `:last-child`, `:only-child`, with a full `an+b` parser
  (`odd`/`even`/`2n+1`/`-n+2`/…). New public types `Pseudo`, `NthExpr`.
- *(selector)* Descendant (`A B`) and child (`A > B`) combinators. New public
  `Combinator` enum. Combinators resolve through `CascadeContext`, which now
  maintains an ancestor-identity stack (zero cost on stylesheets with no
  combinators, gated by `Stylesheet::has_combinators()`).
- *(media)* `@media` queries — `@media (min-width: 80) and (max-height: 40) { … }`
  blocks with terminal-size (`min/max-width/height`, `width`, `height`) and
  color-capability (`color`, `monochrome`, `truecolor`) conditions. New module
  `media` with `MediaContext` / `MediaQuery` / `MediaCondition`. Driven via
  `Stylesheet::compute_with_media`, `CascadeContext::with_media`, and
  `RuntimeStyle::with_media`.
- *(serde)* Deserialize from **any** serde format — JSON **and** TOML **and**
  YAML now round-trip (including bare TOML integers for length/padding). The
  custom `Deserialize` impls were rewritten from `serde_json::Value`-coupled to
  format-agnostic `Visitor`s.兑现 design.md §1/§2 的 "JSON/TOML/YAML" 承诺.
- *(bench)* Criterion benchmark suite (`benches/cascade.rs`) covering the hot
  path — `compute` vs `compute_with`, `OwnedNode` vs `NodeRef`, parent
  inheritance, `var()` resolution, `CascadeContext` tree walk.

### 🚜 Refactor

- *(cascade)* Rules are now pre-sorted by `(origin, specificity, order)` at
  insertion time; the per-`compute` sort is eliminated (the design-doc claim is
  now true).
- *(cascade)* `render_computed` no longer computes `apply_margin` twice.
- *(box-model)* `BorderSpec::edges_to_keyword` no longer `Box::leak`s — uses a
  bounded `const` table over the 16 `Borders` bitsets.
- *(box-model)* `BoxEdges::parse` now rejects shorthand with more than 4 values
  (previously silently wrapped).
- *(color)* Deduplicated the twin `split_top_comma` (`color`/`box_model`).
- *(lib)* `#![forbid(unsafe_code)]` — the no-`unsafe` guarantee is now a
  compile-time one.

### 🐛 Bug Fixes

- *(build)* Examples `01_values` and `14_data_driven` now declare
  `required-features = ["serde"]`, fixing `cargo build --no-default-features`.

### [breaking]

- `Selector.pseudos` changed type `Vec<PseudoClass>` → `Vec<Pseudo>` (state
  flags are now `Pseudo::State(PseudoClass)`).
- `Selector` gained an `ancestor: Option<(Combinator, Box<Selector>)>` field.
- `RuleEntry` gained a `media: Option<MediaQuery>` field.
- One-shot `compute` / `compute_with` use a default (empty) `MediaContext` —
  `@media`-gated rules do not apply via these entry points; use
  `compute_with_media`, `CascadeContext::with_media`, or `RuntimeStyle::with_media`.
- Combinator selectors (`A B`, `A > B`) require `CascadeContext` to resolve
  (the one-shot path has no ancestor-identity information).

### 📚 Documentation

- *(design)* Synced `design.md` to the implementation: `font-weight ≥ 600`
  (was ≥700), removed the unimplemented `opacity` row, corrected the
  `border-radius` note, marked P3 implemented.
- *(examples)* Removed a redundant `use serde_json;` flagged by clippy.

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
