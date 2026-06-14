//! Stylesheet — a collection of `Rule`s plus a token table, with a small CSS
//! text parser and a builder API.
//!
//! A stylesheet is parsed once and queried many times by the cascade engine.

use crate::box_model::{BorderStyleValue, BoxEdgesValue, Length};
use crate::color::Color;
use crate::error::{CssError, Loc, Result};
use crate::media::MediaQuery;
use crate::selector::Selector;
use crate::style::{Align, CssStyle, FontStyle, TextDecoration, Weight};
use crate::supports::SupportsQuery;
use crate::token::{ThemeTokens, Token};
use ratatui::widgets::Borders;

/// Cascade origin. Later origins override earlier ones at equal specificity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Origin {
    /// Built-in defaults.
    #[default]
    UserAgent,
    /// Application-wide theme.
    Theme,
    /// End-user configuration (e.g. a config file).
    User,
    /// Inline declaration on the element itself — highest priority.
    Inline,
}

/// A flattened rule: one selector + one declaration block + provenance.
#[derive(Debug, Clone)]
pub struct RuleEntry {
    pub selector: Selector,
    pub style: CssStyle,
    pub origin: Origin,
    /// Insertion order within the stylesheet — the CSS "source order" tiebreaker.
    pub order: usize,
    /// The `@media` query gating this rule, if it came from inside an `@media`
    /// block. `None` for top-level (unconditional) rules. The cascade skips a
    /// rule whose query does not match the active
    /// [`MediaContext`](crate::media::MediaContext).
    pub media: Option<MediaQuery>,
    /// The `@supports` query gating this rule, if it came from inside an
    /// `@supports` block. `None` for top-level (unconditional) rules. The
    /// cascade skips a rule whose query does not match the active
    /// [`MediaContext`](crate::media::MediaContext) (for capability conditions)
    /// or the engine's known-property set (for property conditions).
    pub supports: Option<SupportsQuery>,
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    rules: Vec<RuleEntry>,
    tokens: ThemeTokens,
    /// Whether any rule carries an [`Selector::ancestor`] chain. Set lazily in
    /// `add`/`add_rule`/`extend`; gates whether the cascade maintains an
    /// ancestor-identity stack. Stays `false` for the common (combinator-free)
    /// case so that path pays zero added cost.
    has_combinators: bool,
    /// Invalidation generation counter. Bumped (wrapping) at the START of every
    /// mutation that can change compute output (`add`, `add_rule`, `extend`,
    /// `tokens_mut`). A [`ComputeCache`](crate::cache::ComputeCache) detects a
    /// generation mismatch on lookup and clears entirely, so a stylesheet edit
    /// after a cached walk automatically invalidates every prior entry.
    ///
    /// Parsing methods (`parse`/`parse_with_origin`) build a fresh `Stylesheet`,
    /// so they leave this at 0 — a freshly parsed sheet is uncached relative to
    /// any pre-existing cache.
    generation_gen: u64,
}

impl Stylesheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a token table (CSS custom properties).
    pub fn with_tokens(tokens: ThemeTokens) -> Self {
        Self { rules: Vec::new(), tokens, has_combinators: false, generation_gen: 0 }
    }

    pub fn tokens(&self) -> &ThemeTokens {
        &self.tokens
    }
    /// Hand out a mutable borrow of the token table. Bumps [`generation`](Self::generation)
    /// first because the caller can mutate tokens through the returned `&mut`,
    /// and any token change can alter a downstream `var()` resolution — so the
    /// conservative bump invalidates the whole [`ComputeCache`](crate::cache::ComputeCache).
    pub fn tokens_mut(&mut self) -> &mut ThemeTokens {
        self.bump_gen();
        &mut self.tokens
    }
    pub fn rules(&self) -> &[RuleEntry] {
        &self.rules
    }

    /// Whether this stylesheet contains any rule with a combinator (`A B` or
    /// `A > B`). When `false`, the cascade skips the ancestor-identity stack
    /// entirely — the combinator-free hot path stays allocation-free relative
    /// to the pre-combinator baseline.
    pub fn has_combinators(&self) -> bool {
        self.has_combinators
    }

    /// The invalidation generation. Bumped on every mutation that can change
    /// compute output; a [`ComputeCache`](crate::cache::ComputeCache) clears
    /// itself when it sees a generation different from the one it was populated
    /// under.
    pub fn generation(&self) -> u64 {
        self.generation_gen
    }

    /// Bump the generation counter (wrapping). Called at the START of every
    /// cache-relevant mutation so a concurrent [`ComputeCache`](crate::cache::ComputeCache)
    /// sees a mismatch on its next access.
    fn bump_gen(&mut self) {
        self.generation_gen = self.generation_gen.wrapping_add(1);
    }

    /// Add a rule from a selector string (may be a comma list) + style.
    pub fn add(&mut self, selectors: &str, style: CssStyle, origin: Origin) -> Result<&mut Self> {
        self.bump_gen();
        let order_base = self.rules.len();
        for sel in Selector::parse_list(selectors)? {
            if sel.ancestor.is_some() {
                self.has_combinators = true;
            }
            self.rules.push(RuleEntry {
                selector: sel,
                style: style.clone(),
                origin,
                order: order_base,
                media: None,
                supports: None,
            });
        }
        self.sort_rules();
        Ok(self)
    }

    /// Add a single pre-parsed rule.
    pub fn add_rule(&mut self, selector: Selector, style: CssStyle, origin: Origin) -> &mut Self {
        self.bump_gen();
        if selector.ancestor.is_some() {
            self.has_combinators = true;
        }
        let order = self.rules.len();
        self.rules.push(RuleEntry { selector, style, origin, order, media: None, supports: None });
        self.sort_rules();
        self
    }

    /// Merge another stylesheet's rules and tokens into this one.
    pub fn extend(&mut self, other: &Stylesheet) {
        self.bump_gen();
        self.tokens.merge(&other.tokens);
        self.has_combinators |= other.has_combinators;
        let offset = self.rules.len();
        for r in &other.rules {
            self.rules.push(RuleEntry { order: offset + r.order, ..r.clone() });
        }
        self.sort_rules();
    }

    /// Sort the rules in place by the cascade key `(origin, specificity, order)`.
    ///
    /// The cascade folds declarations in ascending priority order (later =
    /// higher priority), so keeping rules pre-sorted by that key lets
    /// [`Stylesheet::compute_with`](crate::cascade::Stylesheet::compute_with)
    /// iterate rules in priority order directly and skip the per-`compute`
    /// sort. `order` is unique per rule (it's assigned at push time as
    /// `self.rules.len()`), so the sort is deterministic even on (origin,
    /// specificity) ties.
    ///
    /// This runs at the end of every mutation (`add`/`add_rule`/`extend`).
    /// Mutations are rare (parse-time) relative to `compute` (per-frame), so
    /// the sort cost is amortized away from the hot path.
    fn sort_rules(&mut self) {
        self.rules
            .sort_unstable_by_key(|r| (r.origin, r.selector.specificity(), r.order));
    }

    /// Parse a CSS text document (lenient — the default).
    ///
    /// Supports `selector { prop: value; … }` blocks, comma selector lists,
    /// the universal `*`, `:root { --name: color; }` for tokens, `/* … */`
    /// comments, and `@media (query) { … }` conditional blocks. Declarations use
    /// [`apply_decl`]'s property names.
    ///
    /// **`@media` support:**
    /// - Element rules inside an `@media` block are tagged with the parsed
    ///   [`MediaQuery`] and only apply when the active
    ///   [`MediaContext`](crate::media::MediaContext) matches. The block body is
    ///   itself a CSS fragment parsed recursively.
    /// - **`:root` inside `@media`** declares a **media-gated override** — the
    ///   token is resolved against the active [`MediaContext`] at compute time
    ///   (the most-specific matching query wins, ties broken by source order,
    ///   with the default `:root` value as fallback). See
    ///   [`ThemeTokens::get_color_with`](crate::token::ThemeTokens::get_color_with).
    /// - **Nested `@media` inside `@media`** is supported: the inner query is
    ///   combined with the enclosing query via logical AND (see
    ///   [`MediaQuery::and`]), so the inner block's rules are gated on BOTH
    ///   queries holding. Negation inside a nested `@media` is approximate — see
    ///   the `and` docs.
    ///
    /// **Lenient behavior** (unchanged from earlier versions): unknown
    /// properties are silently ignored (forward-compat), and `var(--name)`
    /// references that are undefined at parse time are kept as-is and only
    /// degrade to `Reset` during the cascade. Use [`Stylesheet::parse_strict`]
    /// to turn both into hard errors.
    ///
    /// Parse errors now carry a 1-based [`Loc`] (line:column) pointing into the
    /// source text.
    pub fn parse(css: &str) -> Result<Self> {
        Self::parse_impl(css, /* strict */ false)
    }

    /// Parse a CSS text document in **strict** mode.
    ///
    /// Behaves like [`Stylesheet::parse`] except that two additional conditions
    /// become hard errors:
    ///
    /// 1. **Unknown property** — any declaration whose property name is not
    ///    recognized by [`apply_decl`] (and is not a `--`-prefixed custom
    ///    property). The error is [`CssErrorKind::UnknownProperty`] and is
    ///    tagged with the exact line:column of the property.
    /// 2. **Undefined variable** — any `var(--name)` with **no fallback** whose
    ///    name is not declared in this stylesheet's token table (either via a
    ///    `:root { --name: … }` block or an inline `--name:` declaration). The
    ///    error is [`CssErrorKind::UndefinedVariable`]. (It is reported with
    ///    `loc = None`; see the module note on var locations.) This covers both
    ///    color `var()` references and length `var()` references (`width`/`height`).
    ///
    /// A `var(--name, fallback)` always passes, since it has a fallback.
    ///
    /// [`CssErrorKind::UnknownProperty`]: crate::CssErrorKind::UnknownProperty
    /// [`CssErrorKind::UndefinedVariable`]: crate::CssErrorKind::UndefinedVariable
    pub fn parse_strict(css: &str) -> Result<Self> {
        Self::parse_impl(css, /* strict */ true)
    }

    fn parse_impl(css: &str, strict: bool) -> Result<Self> {
        // Position-preserving comment strip: the cleaned text is the same length
        // as `css` and has identical line structure, so a byte offset in
        // `cleaned` maps 1:1 to the same offset (and thus the same line:column)
        // in the original `css`.
        let cleaned = strip_comments(css);
        let mut sheet = Stylesheet::new();
        // Top-level rules carry no media query and no supports query.
        parse_rule_loop(&cleaned, cleaned.as_str(), 0, strict, None, None, &mut sheet)?;

        if strict {
            // Any `var(--name)` with no fallback whose name is not declared
            // anywhere in the token table (default map OR a media-gated
            // override) is an error. We scan every rule's declared colors …
            for rule in &sheet.rules {
                for color in color_refs(&rule.style) {
                    if let Some(Color::Var { name, fallback: None }) = color {
                        if !sheet.tokens.is_defined(name) {
                            return Err(CssError::undefined_variable(name.as_str()));
                        }
                    }
                }
                // … and its declared lengths (width/height Length::Var). A var
                // WITH a fallback is fine even if undefined; only a var with no
                // fallback and an unknown name is flagged — mirroring the color
                // strict check above.
                for length in length_refs(&rule.style) {
                    if let Some(Length::Var { name, fallback: None }) = length {
                        if !sheet.tokens.is_defined(name) {
                            return Err(CssError::undefined_variable(name.as_str()));
                        }
                    }
                }
                // … and its declared box-edges (padding/margin BoxEdgesValue::Var) …
                for edges in box_edges_refs(&rule.style) {
                    if let Some(BoxEdgesValue::Var { name, fallback: None }) = edges {
                        if !sheet.tokens.is_defined(name) {
                            return Err(CssError::undefined_variable(name.as_str()));
                        }
                    }
                }
                // … and its declared border style (BorderStyleValue::Var).
                if let Some(BorderStyleValue::Var { name, fallback: None }) =
                    border_style_ref(&rule.style)
                {
                    if !sheet.tokens.is_defined(name) {
                        return Err(CssError::undefined_variable(name.as_str()));
                    }
                }
            }
        }

        Ok(sheet)
    }

    /// Parse a CSS text document, tagging every rule with `origin`.
    ///
    /// Same parsing as [`parse`](Self::parse), but overrides the default
    /// [`Origin::User`] on each rule with `origin`. Used by the
    /// [`css!`](crate::css) macro (origin [`Origin::Theme`]) so that embedded
    /// rules can be overridden at runtime by [`Origin::User`] rules — see
    /// [`RuntimeStyle`](crate::RuntimeStyle).
    pub fn parse_with_origin(css: &str, origin: Origin) -> Result<Self> {
        let mut sheet = Self::parse(css)?;
        for rule in &mut sheet.rules {
            rule.origin = origin;
        }
        Ok(sheet)
    }
}

/// The per-rule parsing loop, extracted so an `@media` / `@supports` block body
/// can be parsed by re-entering it with a `media` / `supports` tag applied to
/// every rule it adds.
///
/// - `cleaned` — the full comment-stripped source (used for `line_col` offsets).
/// - `rest` — the slice of `cleaned` being parsed in this call.
/// - `rest_off` — the byte offset of `rest` within `cleaned`.
/// - `media` — when `Some`, every element rule parsed here is tagged with this
///   query (it came from inside an `@media` block). Top-level calls pass `None`.
/// - `supports` — when `Some`, every element rule parsed here is tagged with
///   this query (it came from inside an `@supports` block). Top-level calls
///   pass `None`. If BOTH `media` and `supports` are `Some`, the inner rule
///   carries both tags — it applies only when BOTH match.
///
/// # Offset correctness (the key invariant)
///
/// The depth-aware close-brace scan returns a **relative** offset into `rest`.
/// Because `rest_off` still tracks the absolute byte offset of `rest` within
/// `cleaned`, `close_off = rest_off + close_rel` maps 1:1 through `line_col` to
/// the correct line:column. The ONLY change from the old flat-`find('}')` logic
/// is *how* `close_rel` is computed (depth scan vs first-`}`); the offset
/// arithmetic is identical. For flat rules (no inner braces), the depth scan
/// returns the same offset as the old `find('}')`, so every existing loc test
/// reports the same line numbers.
fn parse_rule_loop(
    cleaned: &str,
    rest_in: &str,
    rest_off_in: usize,
    strict: bool,
    media: Option<&MediaQuery>,
    supports: Option<&SupportsQuery>,
    sheet: &mut Stylesheet,
) -> Result<()> {
    let mut rest = rest_in;
    let mut rest_off = rest_off_in;

    while let Some(rel) = rest.find('{') {
        let brace_off = rest_off + rel;
        // selector_part occupies cleaned[..brace_off] up to this rule; its
        // trim_start lands at the first non-whitespace char of the selector.
        let selector_part = rest[..rel].trim();
        rest = &rest[rel + 1..];
        rest_off = brace_off + 1;

        // Depth-aware close-brace scan: find the `}` that closes the `{` we just
        // passed. For flat rules (no inner braces) this is the first `}` —
        // identical to the old behavior. For `@media { … { … } … }` it skips
        // inner balanced braces.
        let close_rel = match find_matching_brace(rest) {
            Some(off) => off,
            None => {
                return Err(
                    CssError::invalid_selector("missing closing `}`")
                        .at(line_col(cleaned, brace_off).line, 1),
                );
            }
        };
        let close_off = rest_off + close_rel;
        let body = &rest[..close_rel];
        let body_offset = rest_off;
        rest = &rest[close_rel + 1..];
        rest_off = close_off + 1;

        if selector_part.is_empty() {
            let loc = line_col(cleaned, brace_off);
            return Err(
                CssError::invalid_selector("rule with no selector").at(loc.line, loc.column),
            );
        }
        // Offset of the selector's first non-whitespace char (for loc).
        let sel_off = brace_off - selector_part.len();

        // `@media (query) { … }` — parse the query and recurse on the body.
        // Case-insensitive match on the leading `@media` keyword. The remainder
        // (trimmed) is the query string.
        let lowered_head = selector_part
            .get(..5)
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_media_at =
            lowered_head == "@medi" && selector_part.len() >= 6 && selector_part.as_bytes()[5] == b'a';
        if is_media_at {
            let query_str = selector_part[6..].trim();
            let inner_query = MediaQuery::parse(query_str).map_err(|mut e| {
                if e.loc.is_none() {
                    let loc = line_col(cleaned, sel_off);
                    e = e.at(loc.line, loc.column);
                }
                e
            })?;
            // Nested @media: AND-combine the inner query with the enclosing
            // query. A top-level @media (media == None) tags rules with the
            // inner query directly; a nested @media combines outer ∧ inner via
            // [`MediaQuery::and`] so rules inside are gated on BOTH holding.
            // `combined` is owned here so we can hand a stable `&MediaQuery` to
            // the recursive call for the inner body.
            let combined = match media {
                Some(outer) => outer.and(&inner_query),
                None => inner_query,
            };
            // Recurse: parse the body fragment with the combined query tagging
            // its rules (and :root inserts landing in `media_vars` under it).
            // The `supports` tag is threaded through unchanged so an outer
            // `@supports` continues to gate rules inside a nested `@media`.
            // The body lives in
            // `cleaned[body_offset..body_offset + body.len()]`, so offsets stay
            // correct.
            parse_rule_loop(cleaned, body, body_offset, strict, Some(&combined), supports, sheet)?;
            continue;
        }

        // `@supports (query) { … }` — parse the query and recurse on the body.
        // Case-insensitive match on the leading `@supports` keyword. The
        // remainder (trimmed) is the query string. `@supports` blocks nest
        // braces the same way as `@media`, so the depth-aware `close_rel` scan
        // already found the correct outer `}`.
        let lowered_head_9 = selector_part
            .get(..9)
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_supports_at = lowered_head_9 == "@supports";
        if is_supports_at {
            let query_str = selector_part[9..].trim();
            let inner_query = SupportsQuery::parse(query_str).map_err(|mut e| {
                if e.loc.is_none() {
                    let loc = line_col(cleaned, sel_off);
                    e = e.at(loc.line, loc.column);
                }
                e
            })?;
            // Unlike `@media`, there is no AND-combinator across nested
            // `@supports` blocks: the inner query is stored verbatim per-rule.
            // But an OUTER `@supports` must continue to gate the rules inside:
            // we use the inner query when there is no enclosing supports query,
            // otherwise the enclosing one takes precedence (the inner rule
            // carries only one `supports` tag — the outermost `@supports` in
            // effect). This mirrors how a top-level `@supports` tags rules
            // directly. An inner `@supports` inside an outer `@supports` is
            // therefore subsumed by the outer tag for v1 (documented
            // limitation). The `media` tag is threaded through unchanged so an
            // outer `@media` continues to gate rules inside a `@supports`.
            let effective_supports = supports.unwrap_or(&inner_query);
            parse_rule_loop(cleaned, body, body_offset, strict, media, Some(effective_supports), sheet)?;
            continue;
        }

        // `:root { --x: … }` declares tokens. When inside an `@media` block
        // (`media` is `Some`), the token is a media-gated override stored against
        // the enclosing query and only participates when the query matches the
        // active `MediaContext`. At the top level (`media == None`) it goes into
        // the default map as before.
        let is_root = selector_part.split(',').all(|s| s.trim() == ":root");
        if is_root {
            for decl in split_declarations(body, body_offset) {
                if let Some(name) = decl.prop.strip_prefix("--") {
                    let loc = line_col(cleaned, decl.value_offset);
                    let token =
                        parse_token_value(decl.value).map_err(|e| e.at(loc.line, loc.column))?;
                    match media {
                        Some(q) => sheet.tokens.insert_media(q.clone(), name.trim(), token),
                        None => sheet.tokens.insert(name.trim(), token),
                    }
                }
            }
            continue;
        }

        let mut style = CssStyle::new();
        for decl in split_declarations(body, body_offset) {
            let prop = decl.prop.trim();
            let value = decl.value.trim();
            if prop.is_empty() {
                continue;
            }
            if let Some(name) = prop.strip_prefix("--") {
                let loc = line_col(cleaned, decl.value_offset);
                let token =
                    parse_token_value(value).map_err(|e| e.at(loc.line, loc.column))?;
                sheet.tokens.insert(name, token);
            } else {
                if strict && !is_known_property(prop) {
                    let loc = line_col(cleaned, decl.prop_offset);
                    return Err(CssError::unknown_property(prop).at(loc.line, loc.column));
                }
                let loc = line_col(cleaned, decl.value_offset);
                apply_decl(&mut style, prop, value).map_err(|e| e.at(loc.line, loc.column))?;
            }
        }

        // Parse the selector list and push each variant, tagging with the media
        // and supports queries when present.
        let sels = match Selector::parse_list(selector_part) {
            Ok(v) => v,
            Err(mut e) => {
                if e.loc.is_none() {
                    let loc = line_col(cleaned, sel_off);
                    e = e.at(loc.line, loc.column);
                }
                return Err(e);
            }
        };
        let order_base = sheet.rules.len();
        for sel in sels {
            if sel.ancestor.is_some() {
                sheet.has_combinators = true;
            }
            sheet.rules.push(RuleEntry {
                selector: sel,
                style: style.clone(),
                origin: Origin::User,
                order: order_base,
                media: media.cloned(),
                supports: supports.cloned(),
            });
        }
        sheet.sort_rules();
    }

    Ok(())
}

/// Find the matching `}` for the most recent `{`, scanning `rest` byte by byte
/// with brace-depth tracking.
///
/// The caller has already consumed up to just past the opening `{`, so `rest`
/// begins at the first byte of the rule body. We scan forward: every `{`
/// increases depth, every `}` either (at depth 0) is the match we return, or
/// (depth > 0) decreases depth. For flat rules with no inner braces, depth never
/// rises above 0 and the first `}` is returned — identical to the old
/// `rest.find('}')`. Returns `None` if the input runs out before the matching
/// close brace.
///
/// This is byte-based and UTF-8-safe because `{` and `}` are single-byte ASCII
/// and never appear as continuation bytes in a multi-byte sequence.
fn find_matching_brace(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut depth: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// All `color`-carrying fields of a [`CssStyle`], for var scanning.
fn color_refs(style: &CssStyle) -> [Option<&Color>; 3] {
    [style.color.as_ref(), style.background.as_ref(), style.underline_color.as_ref()]
}

/// All `length`-carrying fields of a [`CssStyle`], for var scanning.
fn length_refs(style: &CssStyle) -> [Option<&Length>; 2] {
    [style.width.as_ref(), style.height.as_ref()]
}

/// All `box-edges`-carrying fields of a [`CssStyle`] (padding/margin), for var
/// scanning. Like the color/length scans, used by strict-mode parsing.
fn box_edges_refs(style: &CssStyle) -> [Option<&BoxEdgesValue>; 2] {
    [style.padding.as_ref(), style.margin.as_ref()]
}

/// The `border-style` field of a [`CssStyle`], for var scanning.
fn border_style_ref(style: &CssStyle) -> Option<&BorderStyleValue> {
    style.border.as_ref().map(|b| &b.style)
}

/// Parse a custom-property value into a [`Token`], trying [`Color`], then
/// [`Length`], then [`BoxEdges`], then [`BorderStyle`].
///
/// Color and length *literal* syntaxes don't overlap (`#fff`/`rgb()`/named
/// colors vs. `10`/`50%`/`auto`/`min(n)`), so for concrete values whichever
/// parser accepts the value wins. A bare `var(--other)` is the one case that is
/// syntactically valid in every grammar — its type is not knowable at parse
/// time, so it is stored as [`Token::Var`] and resolved by following the chain
/// via [`ThemeTokens::get_color`] / [`ThemeTokens::get_length`] /
/// [`ThemeTokens::get_box_edges`] / [`ThemeTokens::get_border_style`] at
/// cascade time.
///
/// A bare integer (`10`) is accepted by both `Length` and `BoxEdges`; the order
/// below (Color → Length → BoxEdges → BorderStyle) means bare integers become
/// `Token::Length`, preserving the existing length-token behavior. A multi-value
/// box shorthand (`1 2`, `1 2 3 4`) fails the Length parse and falls through to
/// `BoxEdges`. A border-style keyword (`rounded`, `single`, …) that is not a
/// color or length lands on `BorderStyle`.
///
/// If all parsers reject the value, the error reported is the final parse
/// failure (BorderStyle, which tends to be descriptive for garbage).
fn parse_token_value(value: &str) -> Result<Token> {
    use crate::box_model::{BorderStyleValue, BoxEdgesValue};
    // A bare var() reference defers its type until resolution.
    if let Ok(Color::Var { name, fallback: None }) = Color::parse(value) {
        return Ok(Token::Var { name });
    }
    if let Ok(c) = Color::parse(value) {
        return Ok(Token::Color(c));
    }
    if let Ok(l) = Length::parse(value) {
        return Ok(Token::Length(l));
    }
    // BoxEdgesValue::parse returns Edges for concrete shorthands and Var for a
    // var() — the var() case was already caught above, so here we only accept
    // a concrete Edges.
    if let Ok(BoxEdgesValue::Edges(e)) = BoxEdgesValue::parse(value) {
        return Ok(Token::BoxEdges(e));
    }
    // BorderStyleValue::parse returns Fixed for keywords; the var() case was
    // caught above. A non-keyword errors out (the terminal fallback).
    match BorderStyleValue::parse(value) {
        Ok(BorderStyleValue::Fixed(b)) => Ok(Token::BorderStyle(b)),
        Ok(_) => {
            // Unreachable: a bare var() was already returned above. Treat as
            // a parse error to avoid silently storing an untyped Var.
            Err(CssError::invalid_length(format!("invalid token value: {value}")))
        }
        Err(e) => Err(e),
    }
}

/// Apply one `prop: value` declaration to a [`CssStyle`] (text form).
///
/// **Lenient by design**: unknown properties are silently ignored (the `_`
/// match arm) for forward-compatibility. The strict check lives in the parser
/// ([`Stylesheet::parse_strict`]) via [`is_known_property`] so that this public
/// helper stays non-breaking for ad-hoc callers.
pub fn apply_decl(style: &mut CssStyle, prop: &str, value: &str) -> Result<()> {
    let prop = prop.trim().to_ascii_lowercase();
    match prop.as_str() {
        "color" => style.color = Some(Color::parse(value)?),
        "background" | "background-color" => style.background = Some(Color::parse(value)?),
        "font-weight" => style.weight = Some(Weight::parse(value)?),
        "font-style" => style.font_style = Some(FontStyle::parse(value)?),
        "text-decoration" => style.decoration = Some(TextDecoration::parse(value)?),
        "underline-color" => style.underline_color = Some(Color::parse(value)?),
        "padding" => style.padding = Some(crate::box_model::BoxEdgesValue::parse(value)?),
        "margin" => style.margin = Some(crate::box_model::BoxEdgesValue::parse(value)?),
        "border" => {
            let mut spec = crate::box_model::BorderSpec::parse_shorthand(value)?;
            // The full shorthand declares a complete border → force all edges.
            spec.edges = Some(ratatui::widgets::Borders::ALL);
            style.border = Some(spec);
        }
        "border-style" => {
            let parsed = crate::box_model::BorderStyleValue::parse(value)?;
            style.border_mut().style = parsed;
        }
        "border-color" => {
            style.border_mut().color = Some(Color::parse(value)?);
        }
        // Per-edge declarations. Each parses the same `<style> [color]` value
        // grammar as `border`, then merges the resolved style/color *and* ORs
        // the corresponding edge(s) into the spec. This lets `.border-top` and
        // `.border-bottom` compose into a top+bottom set (edges accumulate via
        // `BorderSpec::merge` during the cascade), while a full `border: …`
        // still wins on style/color when it has higher priority.
        "border-top" => apply_per_edge(style, value, Borders::TOP)?,
        "border-right" => apply_per_edge(style, value, Borders::RIGHT)?,
        "border-bottom" => apply_per_edge(style, value, Borders::BOTTOM)?,
        "border-left" => apply_per_edge(style, value, Borders::LEFT)?,
        "border-x" => {
            apply_per_edge(style, value, Borders::LEFT | Borders::RIGHT)?
        }
        "border-y" => {
            apply_per_edge(style, value, Borders::TOP | Borders::BOTTOM)?
        }
        "text-align" => style.text_align = Some(Align::parse(value)?),
        "width" => style.width = Some(crate::box_model::Length::parse(value)?),
        "height" => style.height = Some(crate::box_model::Length::parse(value)?),
        _ => { /* unknown property → ignored (forward-compat) */ }
    }
    Ok(())
}

/// Apply a per-edge border declaration (`border-top`, `border-x`, …) to `style`.
///
/// The value follows the same `<style> [color]` grammar as the `border`
/// shorthand. The resolved style and color are merged onto the existing border
/// spec (so `.border-top: rounded` + a later `.border-color: red` still
/// compose), and the given edge set is OR-accumulated into `spec.edges` —
/// enabling `.border-top` and `.border-bottom` to add up to TOP | BOTTOM.
fn apply_per_edge(style: &mut CssStyle, value: &str, edges: Borders) -> Result<()> {
    // parse_shorthand yields style/color with edges=ALL; we override edges to
    // just the declared subset before merging, so the per-edge declaration
    // never widens the set to all four sides.
    let mut parsed = crate::box_model::BorderSpec::parse_shorthand(value)?;
    parsed.edges = Some(edges);
    style.border_mut().merge(&parsed);
    Ok(())
}

/// Whether `prop` is a property the text parser understands.
///
/// This is the **single source of truth** for the property name set; both
/// [`apply_decl`] (implicitly, via its match) and [`Stylesheet::parse_strict`]
/// (explicitly, via this predicate) consult the same list. A `--`-prefixed name
/// is a custom property and never counts as unknown.
///
/// Also consulted by the `@supports` matcher for `(prop)` / `(prop: value)`
/// conditions — see [`SupportsCondition::matches`](crate::supports::SupportsCondition::matches).
pub(crate) fn is_known_property(prop: &str) -> bool {
    let p = prop.trim().to_ascii_lowercase();
    matches!(
        p.as_str(),
        "color"
            | "background"
            | "background-color"
            | "font-weight"
            | "font-style"
            | "text-decoration"
            | "underline-color"
            | "padding"
            | "margin"
            | "border"
            | "border-style"
            | "border-color"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-x"
            | "border-y"
            | "text-align"
            | "width"
            | "height"
    )
}

/// Replace `/* … */` comments with spaces, **keeping the input length and line
/// structure identical** so byte offsets map 1:1 back to the source.
///
/// Every character inside the comment (including the opening `/*` and closing
/// `*/`) becomes a space — **except** newlines, which are preserved. This keeps
/// `cleaned.len() == css.len()` and `cleaned.lines().count() == css.lines().count()`,
/// which is what makes [`line_col`] correct.
fn strip_comments(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Replace the opening `/*`.
            out.push(' ');
            out.push(' ');
            i += 2;
            // Blank out everything up to and including `*/`, preserving `\n`.
            while i < bytes.len() {
                if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    break;
                }
                let b = bytes[i];
                out.push(if b == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            // Unterminated comment: the loop above runs off the end, which
            // preserves length — acceptable (no dedicated error).
        } else {
            // Push the original char (handles multi-byte UTF-8 safely because we
            // copy bytes that aren't part of a comment delimiter verbatim).
            let ch = css[i..].chars().next().expect("non-empty slice");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Convert a byte offset in `src` to a 1-based `Loc { line, column }`.
///
/// Column is counted in **bytes** from the start of the line (1-based). This is
/// the conventional choice for diagnostics and matches how editors report
/// columns for ASCII CSS; multi-byte characters will report a byte column.
fn line_col(src: &str, byte: usize) -> Loc {
    let byte = byte.min(src.len());
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for (i, b) in src.bytes().enumerate() {
        if i >= byte {
            break;
        }
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Loc::new(line, col)
}

/// One declaration parsed out of a rule body.
struct Decl<'a> {
    prop: &'a str,
    value: &'a str,
    /// Byte offset (within the original source) of the start of `prop`.
    prop_offset: usize,
    /// Byte offset (within the original source) of the start of `value`.
    value_offset: usize,
}

/// Split a rule body into declarations on `;`, honoring nested parentheses (so
/// `var(--x, rgb(1,2,3))` survives intact). `body_offset` is the byte offset of
/// `body` within the original source, used to compute per-declaration offsets.
fn split_declarations<'a>(body: &'a str, body_offset: usize) -> Vec<Decl<'a>> {
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    let mut start = 0usize;
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => {
                push_decl(&body[start..i], body_offset + start, &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_decl(&body[start..], body_offset + start, &mut out);
    out
}

fn push_decl<'a>(chunk: &'a str, chunk_offset: usize, out: &mut Vec<Decl<'a>>) {
    // Find the trimmed prop/value, but compute their offsets relative to the
    // (already offset-adjusted) chunk start.
    let leading = chunk.len() - chunk.trim_start().len();
    let trimmed = &chunk[leading..];
    let trailing = trimmed.len() - trimmed.trim_end().len();
    let core = &trimmed[..trimmed.len() - trailing];
    if core.is_empty() {
        return;
    }
    let core_offset = chunk_offset + leading;
    if let Some(colon) = core.find(':') {
        let prop = &core[..colon];
        let value = &core[colon + 1..];
        out.push(Decl {
            prop,
            // value starts after the colon; trim leading whitespace for the
            // value *offset* so the loc points at the value, not the gap.
            value_offset: core_offset + colon + 1 + (value.len() - value.trim_start().len()),
            prop_offset: core_offset,
            value: value.trim(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CssErrorKind;
    use crate::media::MediaContext;
    use crate::node::OwnedNode;
    use ratatui::style::Color as RColor;

    #[test]
    fn parse_text_stylesheet() {
        let css = r#"
            :root {
                --accent: #00d4ff;
            }
            /* a comment */
            Button.primary {
                color: var(--accent);
                background: blue;
                font-weight: bold;
            }
            #save:focus { text-decoration: underline; }
        "#;
        let sheet = Stylesheet::parse(css).unwrap();
        assert_eq!(sheet.tokens().get_color("accent"), Some(&Color::literal(RColor::Rgb(0, 212, 255))));
        // The `var()` reference is kept verbatim in the rule; resolved at compute time.
        let primary = sheet
            .rules()
            .iter()
            .find(|r| r.selector.classes.iter().any(|c| c == "primary"))
            .unwrap();
        assert_eq!(primary.style.color, Some(Color::var("accent")));
        assert!(sheet.rules().iter().any(|r| r.selector.id.as_deref() == Some("save")));
    }

    #[test]
    fn add_flattens_comma_list() {
        let mut sheet = Stylesheet::new();
        sheet.add("Text, .muted, #title", CssStyle::new(), Origin::User).unwrap();
        assert_eq!(sheet.rules().len(), 3);
    }

    #[test]
    fn parse_with_origin_sets_theme() {
        let sheet = Stylesheet::parse_with_origin("Button { color: red; }", Origin::Theme).unwrap();
        assert_eq!(sheet.rules()[0].origin, Origin::Theme);
    }

    #[test]
    fn border_style_and_color_compose_through_cascade() {
        // Two atomic utility classes — one for the border type, one for its
        // color — compose on a single element. This is the Tailwind idiom
        // (`rounded border-slate-700`) and only works because `border` cascades
        // at the sub-field level.
        let sheet = Stylesheet::parse(
            r#"
            .rounded          { border-style: rounded; }
            .border-slate-700 { border-color: #334155; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["rounded", "border-slate-700"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(
            border.style,
            crate::box_model::BorderStyleValue::Fixed(crate::box_model::BorderStyle::Rounded)
        );
        assert_eq!(border.color, Some(Color::literal(RColor::Rgb(0x33, 0x41, 0x55))));
    }

    // -------------------------------------------------------------------------
    // Per-edge borders
    // -------------------------------------------------------------------------

    #[test]
    fn border_bottom_single_edge() {
        // A single per-edge declaration resolves to exactly that edge.
        let sheet = Stylesheet::parse(".b { border-bottom: rounded; }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["b"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(
            border.style,
            crate::box_model::BorderStyleValue::Fixed(crate::box_model::BorderStyle::Rounded)
        );
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::BOTTOM));
        assert_eq!(border.borders(), ratatui::widgets::Borders::BOTTOM);
    }

    #[test]
    fn per_edge_cascade_accumulates_top_and_bottom() {
        // Two atomic per-edge classes compose into TOP | BOTTOM — the same
        // Tailwind-style composition idiom as `.rounded` + `.border-slate-700`.
        let sheet = Stylesheet::parse(
            r#"
            .bt { border-top: single; }
            .bb { border-bottom: single; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["bt", "bb"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::TOP | ratatui::widgets::Borders::BOTTOM));
    }

    #[test]
    fn per_edge_rendered_to_borders() {
        // A `border-bottom` rule computes to a block whose borders() yields only
        // BOTTOM — i.e. only the bottom edge is drawn.
        let sheet = Stylesheet::parse(".x { border-bottom: rounded red; }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.borders(), ratatui::widgets::Borders::BOTTOM);
        // color composes too.
        assert_eq!(border.color, Some(Color::literal(RColor::Red)));
    }

    #[test]
    fn per_edge_x_and_y_aliases() {
        // border-x → LEFT|RIGHT, border-y → TOP|BOTTOM.
        let sheet = Stylesheet::parse(
            r#"
            .bx { border-x: single; }
            .by { border-y: single; }
            "#,
        )
        .unwrap();
        let bx = sheet.compute(&OwnedNode::new("Div").with_classes(["bx"]), None);
        let by = sheet.compute(&OwnedNode::new("Div").with_classes(["by"]), None);
        assert_eq!(
            bx.style.border.as_ref().unwrap().edges,
            Some(ratatui::widgets::Borders::LEFT | ratatui::widgets::Borders::RIGHT)
        );
        assert_eq!(
            by.style.border.as_ref().unwrap().edges,
            Some(ratatui::widgets::Borders::TOP | ratatui::widgets::Borders::BOTTOM)
        );
    }

    #[test]
    fn full_border_shorthand_keeps_all_edges_under_compose() {
        // Regression guard: a full `border: rounded` shorthand (edges=ALL)
        // composed with a per-edge class stays ALL (never narrows).
        let sheet = Stylesheet::parse(
            r#"
            .full { border: rounded; }
            .bot  { border-bottom: single; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["full", "bot"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::ALL));
    }

    #[test]
    fn strict_mode_accepts_per_edge_properties() {
        // The new property names must be recognized in strict mode.
        Stylesheet::parse_strict(".x { border-bottom: rounded; }")
            .expect("border-bottom is a known property in strict mode");
    }

    // -------------------------------------------------------------------------
    // New: location tracking
    // -------------------------------------------------------------------------

    #[test]
    fn located_color_error() {
        // Line 3 holds the bad color.
        let css = "Button {\n    color: red;\n    background: #zzz;\n}\n";
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(loc.line, 3, "line should point at the bad color's line");
        // sanity: it is indeed an invalid color error.
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn comment_positions_preserved() {
        // A multi-line comment must not shift the line of the error below it.
        let css = "/* a\n   multi-line\n   comment */\nButton {\n    color: #nope;\n}\n";
        // cleaned must be the same length as the source.
        let cleaned = strip_comments(css);
        assert_eq!(cleaned.len(), css.len(), "strip_comments is length-preserving");
        // The bad color is on line 5 of the original.
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(loc.line, 5);
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn strip_comments_keeps_length_and_newlines() {
        let css = "a { color: red; /* x\ny */ color: blue; }";
        let cleaned = strip_comments(css);
        assert_eq!(cleaned.len(), css.len());
        assert_eq!(cleaned.matches('\n').count(), css.matches('\n').count());
        // the comment body is now spaces (plus the preserved newline).
        assert!(!cleaned.contains("/*"));
        assert!(!cleaned.contains("*/"));
    }

    // -------------------------------------------------------------------------
    // New: strict mode
    // -------------------------------------------------------------------------

    #[test]
    fn strict_unknown_property() {
        let err = Stylesheet::parse_strict("Foo { colr: red; }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UnknownProperty(ref p) if p == "colr"));
        // loc points at the property (line 1, somewhere on that line).
        let loc = err.loc.expect("unknown property has a location");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn strict_known_property_ok() {
        Stylesheet::parse_strict("Foo { color: red; }").expect("known property parses in strict mode");
    }

    #[test]
    fn strict_undefined_var() {
        let err = Stylesheet::parse_strict("Foo { color: var(--nope); }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_defined_var_ok() {
        Stylesheet::parse_strict(":root{--x:red;}\nFoo{color:var(--x);}").expect("defined var is fine");
    }

    #[test]
    fn strict_var_with_fallback_ok() {
        // A fallback makes even an undefined var acceptable in strict mode.
        Stylesheet::parse_strict("Foo { color: var(--nope, #fff); }")
            .expect("var with fallback does not error in strict mode");
    }

    #[test]
    fn lenient_parse_still_ignores_unknown() {
        // The default `parse` must NOT error on an unknown property.
        Stylesheet::parse("Foo { colr: red; }").expect("lenient parse ignores unknown property");
        // …nor on an undefined var.
        Stylesheet::parse("Foo { color: var(--nope); }").expect("lenient parse keeps undefined var");
    }

    // -------------------------------------------------------------------------
    // Length var() tokens (:root parsing) and strict coverage
    // -------------------------------------------------------------------------

    #[test]
    fn root_parses_length_token() {
        let sheet = Stylesheet::parse(":root{--w:22;--c:#fff;}").unwrap();
        assert_eq!(sheet.tokens().get_length("w"), Some(&crate::box_model::Length::Cells(22)));
        assert_eq!(
            sheet.tokens().get_color("c"),
            Some(&Color::literal(RColor::Rgb(0xff, 0xff, 0xff)))
        );
    }

    #[test]
    fn root_parses_length_percent_token() {
        let sheet = Stylesheet::parse(":root{--half:50%}").unwrap();
        assert_eq!(sheet.tokens().get_length("half"), Some(&crate::box_model::Length::Percent(50)));
    }

    #[test]
    fn root_rejects_garbage_token_value() {
        // Neither a valid color nor a valid length → error.
        assert!(Stylesheet::parse(":root{--x: banana;}").is_err());
    }

    #[test]
    fn strict_undefined_length_var() {
        let err = Stylesheet::parse_strict(".x{width:var(--nope)}").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_defined_length_var_ok() {
        Stylesheet::parse_strict(":root{--w:10}.x{width:var(--w)}")
            .expect("defined length var is fine in strict mode");
    }

    // -------------------------------------------------------------------------
    // @media query blocks
    // -------------------------------------------------------------------------

    #[test]
    fn media_block_tags_one_rule() {
        let sheet =
            Stylesheet::parse("@media (min-width: 80) { Button { color: red; } }").unwrap();
        let button_rules: Vec<_> = sheet
            .rules()
            .iter()
            .filter(|r| r.selector.type_name.as_deref() == Some("Button"))
            .collect();
        assert_eq!(button_rules.len(), 1, "exactly one Button rule");
        let media = button_rules[0].media.as_ref().expect("rule is media-gated");
        assert_eq!(
            media.alternatives,
            vec![crate::media::MediaAlternative {
                terms: vec![crate::media::MediaTerm {
                    negated: false,
                    cond: crate::media::MediaCondition::MinWidth(80),
                }],
            }]
        );
    }

    #[test]
    fn media_block_depth_aware_two_rules_tagged() {
        // TWO element rules inside one @media block. The depth-aware brace scan
        // must find the OUTER `}` (after both rules), not the first inner `}`.
        let sheet = Stylesheet::parse(
            "@media (min-width: 80) { Button { color: red; } .x { padding: 1; } }",
        )
        .unwrap();
        // Both inner rules are tagged with the query.
        let tagged = sheet.rules().iter().filter(|r| r.media.is_some()).count();
        assert_eq!(tagged, 2, "both inner rules carry the media query");
        for r in sheet.rules() {
            if let Some(m) = &r.media {
                assert_eq!(
                    m.alternatives,
                    vec![crate::media::MediaAlternative {
                        terms: vec![crate::media::MediaTerm {
                            negated: false,
                            cond: crate::media::MediaCondition::MinWidth(80),
                        }],
                    }]
                );
            }
        }
    }

    #[test]
    fn media_block_trailing_top_level_rule_untagged() {
        // Regression: after the @media block, a trailing top-level rule must
        // parse with media: None. This exercises that the depth-aware scan
        // correctly resumes `rest` after the matching outer `}`.
        let sheet = Stylesheet::parse(
            "@media (min-width: 80) { Button { color: red; } } Text { color: blue; }",
        )
        .unwrap();
        let text_rule = sheet
            .rules()
            .iter()
            .find(|r| r.selector.type_name.as_deref() == Some("Text"))
            .expect("trailing Text rule parsed");
        assert!(text_rule.media.is_none(), "trailing top-level rule is NOT media-gated");
    }

    #[test]
    fn media_block_invalid_color_loc_points_at_value() {
        // The error loc must point at the #zzz line (inside the @media block),
        // NOT at the @media line. This guards offset correctness through the
        // depth-aware brace scan + recursion.
        let css = "@media (min-width:1){\n Button {\n  background: #zzz;\n }\n}";
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(
            loc.line, 3,
            "loc must point at the #zzz line (line 3), got line {}",
            loc.line
        );
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn root_inside_media_inserts_media_gated_token() {
        // P4-3: :root inside @media now declares a MEDIA-GATED override, not a
        // global one. The default `get_color` (no media ctx) does NOT see it;
        // `get_color_with` against a matching context does.
        let sheet = Stylesheet::parse("@media (min-width:1){ :root { --x: #fff; } }").unwrap();
        // Default map: the override is NOT present.
        assert_eq!(
            sheet.tokens().get_color("x"),
            None,
            "media-gated :root must NOT land in the default token map"
        );
        // Matching context: resolves to the override.
        let large = MediaContext { cols: 80, ..Default::default() };
        assert_eq!(
            sheet.tokens().get_color_with("x", &large),
            Some(Color::literal(RColor::Rgb(0xff, 0xff, 0xff))),
            "media-gated :root resolves under a matching context"
        );
        // Non-matching context: no value.
        let small = MediaContext { cols: 0, ..Default::default() };
        assert_eq!(sheet.tokens().get_color_with("x", &small), None);
    }

    #[test]
    fn nested_media_combines_queries() {
        // P5-4: nested @media ANDs the inner query with the enclosing query.
        // @media (min-width: 80) { @media (color) { Button { color: red } } }
        // → Button rule gated on (min-width:80) AND (color).
        let css = "@media (min-width: 80) { @media (color) { Button { color: red; } } }";
        let sheet = Stylesheet::parse(css).unwrap();
        let button_rules: Vec<_> =
            sheet.rules().iter().filter(|r| r.selector.type_name.as_deref() == Some("Button")).collect();
        assert_eq!(button_rules.len(), 1, "exactly one Button rule");
        let media = button_rules[0].media.as_ref().expect("Button rule is media-gated");
        // One alternative, two concatenated conditions.
        assert_eq!(
            media.alternatives,
            vec![crate::media::MediaAlternative {
                terms: vec![
                    crate::media::MediaTerm {
                        negated: false,
                        cond: crate::media::MediaCondition::MinWidth(80),
                    },
                    crate::media::MediaTerm {
                        negated: false,
                        cond: crate::media::MediaCondition::Color,
                    },
                ],
            }],
            "nested @media AND-combines the queries"
        );
        // Drive a real cascade: under {cols:100, color} the rule applies; under
        // {cols:100, no_color} it does not; under {cols:60, color} it does not.
        use crate::cascade::ComputeScratch;
        let node = OwnedNode::new("Button");
        let mut scratch = ComputeScratch::new();
        let both = MediaContext { cols: 100, no_color: false, ..Default::default() };
        let computed = sheet.compute_with_media(&node, None, &mut scratch, &both);
        assert_eq!(
            computed.style.color,
            Some(Color::literal(RColor::Red)),
            "both conditions hold → rule applies"
        );
        let color_off = MediaContext { cols: 100, no_color: true, ..Default::default() };
        let computed = sheet.compute_with_media(&node, None, &mut scratch, &color_off);
        assert_eq!(
            computed.style.color, None,
            "color=false → combined query fails → rule does NOT apply"
        );
        let width_off = MediaContext { cols: 60, no_color: false, ..Default::default() };
        let computed = sheet.compute_with_media(&node, None, &mut scratch, &width_off);
        assert_eq!(
            computed.style.color, None,
            "min-width fails → combined query fails → rule does NOT apply"
        );
    }

    #[test]
    fn nested_media_three_deep() {
        // @media (a){@media (b){@media (c){ X{…} }}} → tagged with (a∧b∧c).
        let css = "@media (min-width: 80) {\n  @media (color) {\n    @media (truecolor) {\n      X { color: red; }\n    }\n  }\n}";
        let sheet = Stylesheet::parse(css).unwrap();
        let x_rules: Vec<_> =
            sheet.rules().iter().filter(|r| r.selector.type_name.as_deref() == Some("X")).collect();
        assert_eq!(x_rules.len(), 1);
        let media = x_rules[0].media.as_ref().expect("X rule is media-gated");
        assert_eq!(
            media.alternatives,
            vec![crate::media::MediaAlternative {
                terms: vec![
                    crate::media::MediaTerm {
                        negated: false,
                        cond: crate::media::MediaCondition::MinWidth(80),
                    },
                    crate::media::MediaTerm {
                        negated: false,
                        cond: crate::media::MediaCondition::Color,
                    },
                    crate::media::MediaTerm {
                        negated: false,
                        cond: crate::media::MediaCondition::Truecolor,
                    },
                ],
            }],
            "three-deep nesting AND-combines all three queries"
        );
        // Only all-three ctx matches.
        let all = MediaContext {
            cols: 100,
            no_color: false,
            truecolor: true,
            ..Default::default()
        };
        assert!(media.matches(&all), "all three hold → matches");
        let missing_truecolor = MediaContext {
            cols: 100,
            no_color: false,
            truecolor: false,
            ..Default::default()
        };
        assert!(
            !media.matches(&missing_truecolor),
            "missing truecolor → no match"
        );
    }

    #[test]
    fn nested_media_root_inserts_under_combined_query() {
        // :root inside a nested @media lands in media_vars under the COMBINED
        // query, not just the inner one.
        let css = "@media (min-width: 80) { @media (color) { :root { --x: red; } } }";
        let sheet = Stylesheet::parse(css).unwrap();
        // Default map: not present.
        assert_eq!(sheet.tokens().get_color("x"), None);
        // Only (min-width:80) matches but not (color) → combined query fails → no value.
        let width_only = MediaContext { cols: 100, no_color: true, ..Default::default() };
        assert_eq!(sheet.tokens().get_color_with("x", &width_only), None);
        // Both → resolves.
        let both = MediaContext { cols: 100, no_color: false, ..Default::default() };
        assert_eq!(
            sheet.tokens().get_color_with("x", &both),
            Some(Color::literal(RColor::Red))
        );
    }

    #[test]
    fn media_query_error_propagates() {
        // A malformed query surfaces as an error (not silently skipped).
        assert!(Stylesheet::parse("@media (foo: 1) { Button { color: red; } }").is_err());
    }

    #[test]
    fn existing_loc_tests_unchanged_by_depth_scan() {
        // Re-run the two offset-correctness guard tests inline to prove the
        // depth-aware brace change did not shift line numbers.
        let css = "Button {\n    color: red;\n    background: #zzz;\n}\n";
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(loc.line, 3);

        let css2 = "/* a\n   multi-line\n   comment */\nButton {\n    color: #nope;\n}\n";
        let err2 = Stylesheet::parse(css2).unwrap_err();
        let loc2 = err2.loc.expect("error has a location");
        assert_eq!(loc2.line, 5);
    }

    // ---------------------------------------------------------------------
    // Media-gated :root tokens (P4-3)
    // ---------------------------------------------------------------------

    #[test]
    fn parses_root_inside_media_into_media_gated_override() {
        // :root { --accent: red } @media (min-width: 80) { :root { --accent: blue } }
        // → default has red, a media entry has blue.
        let css = ":root { --accent: red } @media (min-width: 80) { :root { --accent: blue } }";
        let sheet = Stylesheet::parse(css).unwrap();
        // Default map = red.
        assert_eq!(
            sheet.tokens().get_color("accent"),
            Some(&Color::literal(RColor::Red))
        );
        // Matching context = blue.
        let large = MediaContext { cols: 100, ..Default::default() };
        assert_eq!(
            sheet.tokens().get_color_with("accent", &large),
            Some(Color::literal(RColor::Blue))
        );
        // Non-matching = red (default fallback).
        let small = MediaContext { cols: 60, ..Default::default() };
        assert_eq!(
            sheet.tokens().get_color_with("accent", &small),
            Some(Color::literal(RColor::Red))
        );
    }

    #[test]
    fn strict_accepts_var_defined_only_in_media() {
        // var(--x) used in a rule, --x defined only inside an @media block →
        // strict parse must NOT error (it's defined somewhere).
        let css = "@media (min-width: 80) { :root { --x: red; } }\n.a { color: var(--x); }";
        Stylesheet::parse_strict(css).expect("var defined only in @media is 'defined'");
    }

    #[test]
    fn strict_still_errors_on_truly_undefined_var() {
        // Regression: a var with no definition anywhere still errors in strict.
        Stylesheet::parse_strict(".a { color: var(--nope); }").unwrap_err();
        // And a width-length var that's undefined.
        Stylesheet::parse_strict(".a { width: var(--nope); }").unwrap_err();
    }

    // ---------------------------------------------------------------------
    // Generation counter (P4-4 cache invalidation backbone)
    // ---------------------------------------------------------------------

    #[test]
    fn generation_starts_at_zero() {
        // Every constructor path yields a fresh gen of 0.
        assert_eq!(Stylesheet::new().generation(), 0);
        assert_eq!(Stylesheet::with_tokens(ThemeTokens::new()).generation(), 0);
        // A freshly parsed sheet is also 0 — parse builds via `Stylesheet::new`.
        let parsed = Stylesheet::parse("Button { color: red; }").unwrap();
        assert_eq!(parsed.generation(), 0);
        let with_origin = Stylesheet::parse_with_origin("Button { color: red; }", Origin::Theme).unwrap();
        assert_eq!(with_origin.generation(), 0);
    }

    #[test]
    fn generation_bumps_on_add() {
        let mut s = Stylesheet::new();
        let g0 = s.generation();
        s.add("Button", CssStyle::new().color(RColor::Red), Origin::User)
            .unwrap();
        assert_ne!(s.generation(), g0);
        // Each subsequent add bumps again.
        let g1 = s.generation();
        s.add("Text", CssStyle::new(), Origin::User).unwrap();
        assert_ne!(s.generation(), g1);
    }

    #[test]
    fn generation_bumps_on_add_rule() {
        let mut s = Stylesheet::new();
        let g0 = s.generation();
        let sel = Selector::parse_compound("Button").unwrap();
        s.add_rule(sel, CssStyle::new(), Origin::User);
        assert_ne!(s.generation(), g0);
    }

    #[test]
    fn generation_bumps_on_extend() {
        let mut a = Stylesheet::new();
        let other = Stylesheet::parse("Text { color: blue; }").unwrap();
        let g0 = a.generation();
        a.extend(&other);
        assert_ne!(a.generation(), g0);
    }

    #[test]
    fn generation_bumps_on_tokens_mut() {
        let mut s = Stylesheet::new();
        let g0 = s.generation();
        let _ = s.tokens_mut();
        assert_ne!(s.generation(), g0, "tokens_mut must bump (covers token changes)");
    }

    // ---------------------------------------------------------------------
    // @supports query blocks
    // ---------------------------------------------------------------------

    #[test]
    fn supports_block_tags_one_rule() {
        let sheet =
            Stylesheet::parse("@supports (truecolor) { Button { color: red; } }").unwrap();
        let button_rules: Vec<_> = sheet
            .rules()
            .iter()
            .filter(|r| r.selector.type_name.as_deref() == Some("Button"))
            .collect();
        assert_eq!(button_rules.len(), 1, "exactly one Button rule");
        let supports = button_rules[0].supports.as_ref().expect("rule is supports-gated");
        assert_eq!(
            supports.alternatives,
            vec![crate::supports::SupportsAlternative {
                terms: vec![crate::supports::SupportsTerm {
                    negated: false,
                    cond: crate::supports::SupportsCondition::Truecolor,
                }],
            }]
        );
        // Untagged for media.
        assert!(button_rules[0].media.is_none());
    }

    #[test]
    fn supports_block_depth_aware_two_rules_tagged() {
        // TWO element rules inside one @supports block. The depth-aware brace
        // scan must find the OUTER `}` (after both rules), not the first inner.
        let sheet = Stylesheet::parse(
            "@supports (truecolor) { Button { color: red; } .x { padding: 1; } }",
        )
        .unwrap();
        let tagged = sheet.rules().iter().filter(|r| r.supports.is_some()).count();
        assert_eq!(tagged, 2, "both inner rules carry the supports query");
    }

    #[test]
    fn supports_block_trailing_top_level_rule_untagged() {
        // After the @supports block, a trailing top-level rule must parse with
        // supports: None. This exercises that the depth-aware scan correctly
        // resumes `rest` after the matching outer `}`.
        let sheet = Stylesheet::parse(
            "@supports (truecolor) { Button { color: red; } } Text { color: blue; }",
        )
        .unwrap();
        let text_rule = sheet
            .rules()
            .iter()
            .find(|r| r.selector.type_name.as_deref() == Some("Text"))
            .expect("trailing Text rule parsed");
        assert!(text_rule.supports.is_none(), "trailing top-level rule is NOT supports-gated");
    }

    #[test]
    fn supports_block_invalid_color_loc_points_at_value() {
        // The error loc must point at the #zzz line (inside the @supports
        // block), NOT at the @supports line. Guards offset correctness through
        // the depth-aware brace scan + recursion.
        let css = "@supports (truecolor){\n Button {\n  background: #zzz;\n }\n}";
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(
            loc.line, 3,
            "loc must point at the #zzz line (line 3), got line {}",
            loc.line
        );
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn supports_nested_inside_media_tags_both() {
        // @media (min-width: 80) { @supports (truecolor) { Button { … } } }
        // → Button rule carries BOTH a media and a supports tag.
        let css = "@media (min-width: 80) { @supports (truecolor) { Button { color: red; } } }";
        let sheet = Stylesheet::parse(css).unwrap();
        let button_rules: Vec<_> =
            sheet.rules().iter().filter(|r| r.selector.type_name.as_deref() == Some("Button")).collect();
        assert_eq!(button_rules.len(), 1, "exactly one Button rule");
        let r = &button_rules[0];
        let media = r.media.as_ref().expect("rule is media-gated");
        assert_eq!(
            media.alternatives,
            vec![crate::media::MediaAlternative {
                terms: vec![crate::media::MediaTerm {
                    negated: false,
                    cond: crate::media::MediaCondition::MinWidth(80),
                }],
            }]
        );
        let supports = r.supports.as_ref().expect("rule is supports-gated");
        assert_eq!(
            supports.alternatives,
            vec![crate::supports::SupportsAlternative {
                terms: vec![crate::supports::SupportsTerm {
                    negated: false,
                    cond: crate::supports::SupportsCondition::Truecolor,
                }],
            }]
        );
    }

    #[test]
    fn media_nested_inside_supports_tags_both() {
        // @supports (truecolor) { @media (min-width: 80) { Button { … } } }
        // → Button rule carries BOTH a supports and a media tag.
        let css = "@supports (truecolor) { @media (min-width: 80) { Button { color: red; } } }";
        let sheet = Stylesheet::parse(css).unwrap();
        let button_rules: Vec<_> =
            sheet.rules().iter().filter(|r| r.selector.type_name.as_deref() == Some("Button")).collect();
        assert_eq!(button_rules.len(), 1);
        let r = &button_rules[0];
        assert!(r.media.is_some(), "rule carries media tag");
        assert!(r.supports.is_some(), "rule carries supports tag");
        let supports = r.supports.as_ref().unwrap();
        assert_eq!(
            supports.alternatives[0].terms[0].cond,
            crate::supports::SupportsCondition::Truecolor
        );
    }

    #[test]
    fn supports_query_error_propagates() {
        // A malformed query structure surfaces as an error (not silently
        // skipped). Unbalanced parens.
        assert!(Stylesheet::parse("@supports (truecolor { Button { color: red; } }").is_err());
    }

    #[test]
    fn supports_property_condition_tagged() {
        // A property-support condition tags the rule.
        let sheet =
            Stylesheet::parse("@supports (border-style) { .x { border-style: rounded; } }").unwrap();
        let x_rules: Vec<_> =
            sheet.rules().iter().filter(|r| r.selector.classes.iter().any(|c| c == "x")).collect();
        assert_eq!(x_rules.len(), 1);
        let supports = x_rules[0].supports.as_ref().expect("supports-gated");
        assert_eq!(
            supports.alternatives[0].terms[0].cond,
            crate::supports::SupportsCondition::Property("border-style".into())
        );
    }

    #[test]
    fn supports_case_insensitive_keyword() {
        // @SUPPORTS should parse case-insensitively.
        let sheet = Stylesheet::parse("@SUPPORTS (truecolor) { Button { color: red; } }").unwrap();
        let button = sheet
            .rules()
            .iter()
            .find(|r| r.selector.type_name.as_deref() == Some("Button"))
            .unwrap();
        assert!(button.supports.is_some());
    }

    // ---------------------------------------------------------------------
    // var() in padding / margin / border-style (P6-4)
    // ---------------------------------------------------------------------

    #[test]
    fn parse_padding_var_reference() {
        // `padding: var(--pad)` parses into a BoxEdgesValue::Var that survives
        // until cascade resolution.
        let sheet = Stylesheet::parse(":root{--pad:1} .x { padding: var(--pad); }").unwrap();
        let rule = sheet
            .rules()
            .iter()
            .find(|r| r.selector.classes.iter().any(|c| c == "x"))
            .unwrap();
        assert_eq!(
            rule.style.padding,
            Some(crate::box_model::BoxEdgesValue::var("pad"))
        );
    }

    #[test]
    fn parse_border_style_var_reference() {
        let sheet = Stylesheet::parse(":root{--bs:rounded} .x { border-style: var(--bs); }").unwrap();
        let rule = sheet
            .rules()
            .iter()
            .find(|r| r.selector.classes.iter().any(|c| c == "x"))
            .unwrap();
        let border = rule.style.border.as_ref().expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyleValue::var("bs"));
    }

    #[test]
    fn parse_border_shorthand_with_var_style() {
        // `border: var(--bs) #f00` — var is the style, #f00 the color.
        let sheet = Stylesheet::parse(":root{--bs:rounded} .x { border: var(--bs) #f00; }").unwrap();
        let rule = sheet
            .rules()
            .iter()
            .find(|r| r.selector.classes.iter().any(|c| c == "x"))
            .unwrap();
        let border = rule.style.border.as_ref().expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyleValue::var("bs"));
        assert_eq!(border.color, Some(Color::literal(RColor::Rgb(0xff, 0, 0))));
    }

    #[test]
    fn strict_undefined_padding_var_errors() {
        let err = Stylesheet::parse_strict(".x { padding: var(--nope); }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_padding_var_with_fallback_ok() {
        Stylesheet::parse_strict(".x { padding: var(--nope, 3); }")
            .expect("padding var with fallback is OK in strict mode");
    }

    #[test]
    fn strict_undefined_border_style_var_errors() {
        let err = Stylesheet::parse_strict(".x { border-style: var(--nope); }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_defined_box_edges_var_ok() {
        Stylesheet::parse_strict(":root{--pad:1} .x { padding: var(--pad); }")
            .expect("defined box-edges var is fine in strict mode");
    }

    #[test]
    fn strict_defined_border_style_var_ok() {
        Stylesheet::parse_strict(":root{--bs:rounded} .x { border-style: var(--bs); }")
            .expect("defined border-style var is fine in strict mode");
    }
}
