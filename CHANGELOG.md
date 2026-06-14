## [0.2.0] - 2026-06-15

A breaking release: the P3 + P4 + P5 + P6 roadmaps land — structural pseudo-classes
(incl. `:nth-of-type` and `:last/only/nth-last-of-type`), all four combinators (incl. nested
sibling chains), `@media` queries with per-term `not` + nesting + media-scoped tokens with
specificity cascade, `@supports` capability gating, `var()` for padding/margin/border-style, and
an access-order `ComputedStyle` LRU cache — alongside a serde cross-format overhaul and cascade
perf work.

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
- *(selector)* Sibling combinators — adjacent (`A + B`) and general (`A ~ B`).
  `CascadeContext` tracks previous-sibling identities per depth (gated by
  `has_combinators()`, zero cost when unused). One-level siblings; nested
  sibling chains (`A + B + C`) are not fully resolved.
- *(media)* `@media` `not` (whole-alternative negation), comma-OR, and `and`.
  `MediaQuery` is now `alternatives: Vec<MediaAlternative>` (each alternative:
  `negated` + `and`-joined conditions). New public `MediaAlternative`.
- *(token)* Media-scoped custom properties — `:root { --x }` inside `@media`
  defines a media-gated override resolved against the active `MediaContext`
  (last-matching-override wins, default fallback). New `ThemeTokens` methods
  `insert_media` / `set_media` / `get_color_with` / `get_length_with` /
  `is_defined`; resolve fns gain `_with_media` variants (old ones are
  default-media wrappers). Lifts the prior v1 ":root inside @media is global"
  limitation.
- *(cache)* Opt-in `ComputedStyle` LRU cache (new `cache` module,
  `ComputeCache`). `CascadeContext::with_cache(capacity)` memoizes compute
  results across frames; keys fold node identity + ancestor-chain signature +
  sibling identities + media. Auto-invalidated via a `Stylesheet::generation()`
  counter bumped on every mutation (`add`/`add_rule`/`extend`/`tokens_mut`).
  Off by default — the no-cache path is unchanged. **Access-order LRU**
  (a `get` hit promotes; eviction is least-recently-used, O(capacity)).
- *(selector)* Nested sibling chains — `A + B + C`, `A ~ B ~ C`, and mixed
  `A + B ~ C` now resolve (the matched sibling's own previous siblings are
  threaded through the recursion).
- *(selector)* `:nth-of-type(an+b)` and `:first-of-type` — count only same-type
  siblings. (`:last/only/nth-last-of-type` need forward-sibling info and are
  not supported; they error at parse.)
- *(media)* `@media` nesting — `@media (a) { @media (b) { … } }` AND-combines
  the queries (`MediaQuery::and`, cross-product of alternatives). `not` inside
  nested `@media` is approximate (documented); use a flat query for precision.
- *(token)* Media-token specificity cascade — when multiple media overrides
  match, the **most specific** (most conditions) wins; ties break by source
  order. Replaces the prior "last-matching-wins" rule.
- *(selector)* `:last-of-type`, `:only-of-type`, `:nth-last-of-type` — sourced
  from host-supplied `Position.of_type_count` (`:nth-of-type`/`:first-of-type`
  keep their prev-sibling fallback). `Position` gained `of_type_index` /
  `of_type_count` fields.
- *(media)* Per-term negation — `not` now negates the immediately-following
  condition (`MediaTerm { negated, cond }`), not the whole alternative.
  `MediaQuery::and` is now EXACT for nested `@media` (no approximation).
- *(supports)* `@supports` capability gating — `@supports (truecolor) { … }`,
  `(color)`, `(monochrome)`, `(prop)` / `(prop: val)` (engine property
  support). New `supports` module; reuses `MediaContext` (no new context type).
  Composes with `@media` (a rule may carry both `media` + `supports` tags).
- *(box-model)* `var()` for `padding` / `margin` (`BoxEdgesValue`) and
  `border-style` (`BorderStyleValue`). New `Token::BoxEdges` / `Token::BorderStyle`;
  resolved during the cascade (lenient → zero edges / `None` style). Builders
  keep backward compat (`.padding(1)`, `.border(BorderStyle::Rounded)`, …).

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
- *(box-model)* Gate the `length_to_css` helper behind `#[cfg(feature =
  "serde")]` — it was dead code under `--no-default-features` after the serde
  rewrite (introduced in this release cycle).

### [breaking]

- `Selector.pseudos` changed type `Vec<PseudoClass>` → `Vec<Pseudo>` (state
  flags are now `Pseudo::State(PseudoClass)`).
- `Selector` gained an `ancestor: Option<(Combinator, Box<Selector>)>` field.
- `RuleEntry` gained a `media: Option<MediaQuery>` field.
- One-shot `compute` / `compute_with` use a default (empty) `MediaContext` —
  `@media`-gated rules do not apply via these entry points; use
  `compute_with_media`, `CascadeContext::with_media`, or `RuntimeStyle::with_media`.
- Combinator selectors (`A B`, `A > B`, `A + B`, `A ~ B`) require
  `CascadeContext` to resolve (the one-shot path has no ancestor/sibling
  identity).
- `MediaQuery.conditions` → `MediaQuery.alternatives: Vec<MediaAlternative>`
  (`not`/comma/`and` support). Callers reading `query.conditions` must adapt.
- `ThemeTokens` resolve/getter calls that need media awareness use the new
  `_with_media` / `_with` variants; the cascade threads the active context
  through var resolution.

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
