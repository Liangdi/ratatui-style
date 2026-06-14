//! `@media` query support — conditional rules based on terminal size and color
//! capability.
//!
//! A [`MediaQuery`] is parsed from the text between `@media` and the block's
//! opening `{` (e.g. `(min-width: 80) and (max-height: 40)`). Each element rule
//! inside an `@media` block is tagged with the parsed query; the cascade skips a
//! tagged rule unless [`MediaQuery::matches`] the current [`MediaContext`].
//!
//! # Matching model
//!
//! A query is a comma-separated list (logical OR) of [`MediaAlternative`]s. Each
//! alternative is an optional `not` prefix applied to a conjunction (logical AND)
//! of [`MediaCondition`]s. Precedence, tightest first:
//!
//! `not` (whole-alternative) > `and` (conditions) > `,` (alternatives / OR)
//!
//! So `(min-width: 80), not (color) and (max-height: 40)` parses as two
//! alternatives: `(min-width: 80)` OR `not ((color) and (max-height: 40))`.
//!
//! A query with **no alternatives** (e.g. a bare `@media {}` with no query text)
//! matches anything — a no-op gate — preserving the historically lenient behavior.
//!
//! Media types (`screen`, `all`, `print`, `only`) are accepted syntactically and
//! **ignored**: terminal apps are always "screen". A bare `@media print { }` is
//! treated like `@media all { }` (matches everything).
//!
//! Default-context caution: [`MediaContext::default()`] is all-zero / all-false,
//! which means "no terminal info". A media-gated rule with any condition will
//! NOT match a default context (e.g. `min-width: 80` vs `cols = 0` is false).
//! This is by design: a host that never supplies media info should not have
//! media-gated rules silently apply.

use crate::error::{CssError, Result};

/// What the host knows about the current terminal, supplied per render.
///
/// Defaults (all zero/false) mean "no media info" — media-gated rules with any
/// condition will NOT match against a default context (e.g. `min-width: 80` vs
/// `cols = 0` is false).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaContext {
    /// Terminal width in cells.
    pub cols: u16,
    /// Terminal height in cells.
    pub rows: u16,
    /// Whether the terminal supports 24-bit color.
    pub truecolor: bool,
    /// Whether color is disabled (e.g. `$NO_COLOR` is set).
    pub no_color: bool,
}

/// One `@media` query: one or more [`MediaAlternative`]s joined by comma (OR).
///
/// The query matches if **any** alternative matches. An `alternatives` list with
/// zero entries (e.g. a bare `@media {}` with no query text) matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaQuery {
    /// Comma-separated alternatives; the query matches if ANY matches. An empty
    /// list matches everything (a no-op gate).
    pub alternatives: Vec<MediaAlternative>,
}

/// One `and`-conjunction of [`MediaCondition`]s, optionally negated as a whole.
///
/// `matches` is true iff `(all conditions hold) XOR negated`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaAlternative {
    /// A `not` prefix negates the whole alternative.
    pub negated: bool,
    /// Conditions joined by `and`; all must hold (before negation).
    pub conditions: Vec<MediaCondition>,
}

/// A single media feature condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCondition {
    /// `(min-width: n)` — terminal width ≥ `n` cells.
    MinWidth(u16),
    /// `(max-width: n)` — terminal width ≤ `n` cells.
    MaxWidth(u16),
    /// `(width: n)` — terminal width exactly `n` cells.
    Width(u16),
    /// `(min-height: n)` — terminal height ≥ `n` cells.
    MinHeight(u16),
    /// `(max-height: n)` — terminal height ≤ `n` cells.
    MaxHeight(u16),
    /// `(height: n)` — terminal height exactly `n` cells.
    Height(u16),
    /// `(color)` — terminal has color (not `$NO_COLOR`).
    Color,
    /// `(monochrome)` — terminal has color disabled.
    Monochrome,
    /// `(truecolor)` — ratatui-style extension: terminal supports 24-bit color.
    Truecolor,
}

impl MediaQuery {
    /// True iff **any** alternative matches `ctx`. A query with no alternatives
    /// matches anything (a no-op gate).
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        if self.alternatives.is_empty() {
            return true;
        }
        self.alternatives.iter().any(|a| a.matches(ctx))
    }

    /// Combine two queries with logical AND: the result matches iff **both**
    /// `self` and `other` match.
    ///
    /// This is used to resolve nested `@media` blocks (`@media (a) { @media (b)
    /// { … } }` is equivalent to a single `@media (a) and (b) { … }`).
    ///
    /// # Semantics
    ///
    /// A query is an OR of [`MediaAlternative`]s, and each alternative is an AND
    /// of [`MediaCondition`]s. To form `self ∧ other` we take the **cross
    /// product** of the two alternatives lists: for each `a1` in `self` and each
    /// `a2` in `other`, the combined alternative has
    /// `conditions = a1.conditions ++ a2.conditions`. So `(a),(b)` AND-ed with
    /// `(c)` yields two alternatives `(a,c)` and `(b,c)` — each must be fully
    /// satisfied for the OR to match, which is exactly AND-of-OR semantics.
    ///
    /// Match-all short-circuits: if either side has **zero alternatives** (the
    /// match-all gate) the other side is returned unchanged (cloned). If both
    /// are empty the result is empty (still match-all).
    ///
    /// # Negation limitation (approximate for `not`)
    ///
    /// A [`MediaAlternative`] carries a single `negated` flag for the *whole*
    /// conjunction, so a precise `(¬a1) ∧ a2` (one side negated, the other not)
    /// cannot be represented as one alternative. For the cross product:
    ///
    /// - If **neither** side is negated, the combined alternative is exact:
    ///   `{ negated: false, conditions: a1.conditions ++ a2.conditions }`.
    /// - If **either** side is negated, the precise AND-semantics cannot be
    ///   represented, so the combined alternative is an **approximation**:
    ///   `negated = a1.negated || a2.negated` with the concatenated conditions.
    ///
    /// The common case (no `not` inside nested `@media`) is exact. If you need
    /// a `not` in a nested context, prefer writing it as a single flat
    /// `@media` with the conditions spelled out rather than nesting.
    pub fn and(&self, other: &MediaQuery) -> MediaQuery {
        // Match-all short-circuits: an empty alternatives list means "matches
        // everything", so X AND all == X.
        if self.alternatives.is_empty() {
            return other.clone();
        }
        if other.alternatives.is_empty() {
            return self.clone();
        }

        let mut combined = Vec::with_capacity(self.alternatives.len() * other.alternatives.len());
        for a1 in &self.alternatives {
            for a2 in &other.alternatives {
                // Concatenate conditions. The negation handling is the
                // documented approximation for the negated case.
                let mut conditions = Vec::with_capacity(a1.conditions.len() + a2.conditions.len());
                conditions.extend(a1.conditions.iter().cloned());
                conditions.extend(a2.conditions.iter().cloned());
                combined.push(MediaAlternative {
                    negated: a1.negated || a2.negated,
                    conditions,
                });
            }
        }
        MediaQuery { alternatives: combined }
    }

    /// The specificity of `self` against `media`: the maximum condition-count
    /// among the alternatives that **match** under `media`. Returns `None` if
    /// the query does not match `media` at all.
    ///
    /// A match-all query (zero alternatives) has specificity `0`. Used by the
    /// media-token resolution scan to rank competing overrides: the override
    /// backed by the most-conditioned matching query wins (ties broken by source
    /// order).
    pub(crate) fn matching_specificity(&self, media: &MediaContext) -> Option<usize> {
        if self.alternatives.is_empty() {
            // Matches everything (specificity 0).
            return Some(0);
        }
        let mut best: Option<usize> = None;
        for a in &self.alternatives {
            if a.matches(media) {
                let n = a.conditions.len();
                best = Some(match best {
                    Some(b) if b >= n => b,
                    _ => n,
                });
            }
        }
        best
    }

    /// Parse the text BETWEEN `@media` and the block's opening `{`.
    ///
    /// Grammar (case-insensitive), precedence tightest first:
    ///
    /// `not` (whole-alternative) > `and` (conditions) > `,` (alternatives / OR)
    ///
    /// - The text is split on top-level commas into one [`MediaAlternative`]
    ///   per part (OR).
    /// - Each part may begin with an optional leading `not` (negates the whole
    ///   alternative) and an optional media-type keyword sequence (`only`,
    ///   `screen`, `all`, `print`, possibly `only screen`) which is accepted and
    ///   **ignored** — terminal apps are always "screen". If a media type is
    ///   present it may be followed by `and`, which is also consumed.
    /// - The remainder is zero or more `(condition)` clauses joined by `and`.
    ///
    /// Unknown / malformed features surface as a [`CssError`] so strict stays
    /// honest; the stylesheet parser propagates it.
    pub fn parse(s: &str) -> Result<MediaQuery> {
        // Lowercase once; tokens are case-insensitive.
        let lower = s.to_ascii_lowercase();

        // A wholly empty/whitespace query → zero alternatives (match-all gate).
        // This is distinct from a stray comma, which yields an empty PART among
        // non-empty ones and is a structural error.
        if lower.trim().is_empty() {
            return Ok(MediaQuery { alternatives: Vec::new() });
        }

        // Split on top-level commas (respecting paren depth — commas inside
        // parens don't occur in this grammar, but guard against future
        // extensions like `(prefers-color-scheme: dark, light)`).
        let mut alternatives = Vec::new();
        for part in split_top_level_commas(&lower) {
            let trimmed = part.trim();
            // An empty part between/around commas (e.g. trailing `, `) is a
            // structural error rather than a silent match-all alternative.
            if trimmed.is_empty() {
                return Err(CssError::invalid_selector(
                    "media query: empty alternative (stray comma?)",
                ));
            }
            alternatives.push(parse_alternative(trimmed)?);
        }

        Ok(MediaQuery { alternatives })
    }
}

impl MediaAlternative {
    /// True iff `(all conditions hold against ctx) XOR negated`.
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        let all_hold = self.conditions.iter().all(|c| c.matches(ctx));
        all_hold != self.negated
    }
}

/// Parse one comma-separated alternative (already trimmed, lowercased).
///
/// Handles the optional leading `not` and media-type keywords, then splits the
/// remainder on `and` into conditions.
fn parse_alternative(part: &str) -> Result<MediaAlternative> {
    let bytes = part.as_bytes();
    let mut i = 0usize;
    let mut negated = false;

    // Skip leading whitespace.
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    // Optional leading `not` (whole word). CSS media queries put `not` at the
    // alternative level (it negates the whole comma-separated part), so this is
    // distinct from the (unsupported) per-condition `not`.
    if let Some(consumed) = consume_keyword(bytes, i, "not") {
        i = consumed;
        negated = true;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }

    // Consume an optional media-type keyword sequence: `only`, `screen`, `all`,
    // `print`, possibly `only screen`. These are IGNORED (terminal apps are
    // always "screen"). Also consume a trailing `and` if present so the
    // remainder is the condition list.
    loop {
        let prev_i = i;
        for kw in ["only", "screen", "all", "print"] {
            if let Some(consumed) = consume_keyword(bytes, i, kw) {
                i = consumed;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                break;
            }
        }
        if i == prev_i {
            break;
        }
    }
    // After consuming media types, consume a single following `and` if present
    // (e.g. `screen and (...)`). This `and` separates the media type from the
    // feature conditions, not conditions from each other.
    if let Some(consumed) = consume_keyword(bytes, i, "and") {
        i = consumed;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }

    // The remainder is the condition list: zero or more `(cond)` joined by
    // `and`. If nothing remains (e.g. bare `screen` or `not screen`), the
    // alternative has zero conditions → matches everything (before negation).
    let mut conditions = Vec::new();
    let mut seen_any = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'(' {
            return Err(CssError::invalid_selector(format!(
                "media query: expected `(` near `{}`",
                &part[i.min(part.len())..]
            )));
        }
        // Find the matching `)`.
        let close = match part[i..].find(')') {
            Some(rel) => i + rel,
            None => {
                return Err(CssError::invalid_selector(
                    "media query: unbalanced parens (missing `)`)",
                ));
            }
        };
        let inner = &part[i + 1..close];
        conditions.push(parse_condition(inner)?);
        i = close + 1;
        seen_any = true;

        // After a condition, expect either end-of-string or ` and `.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if let Some(consumed) = consume_keyword(bytes, i, "and") {
            i = consumed;
            continue;
        }
        return Err(CssError::invalid_selector(format!(
            "media query: expected `and` between conditions near `{}`",
            &part[i..]
        )));
    }

    let _ = seen_any;
    Ok(MediaAlternative { negated, conditions })
}

/// If `bytes[i..]` begins with `kw` as a whole word (followed by whitespace, a
/// `(`, a `,`, or end-of-input), return the index just past the keyword; else
/// `None`. Whole-word match prevents `notable` from matching `not`.
fn consume_keyword(bytes: &[u8], i: usize, kw: &str) -> Option<usize> {
    let kw_bytes = kw.as_bytes();
    if i + kw_bytes.len() > bytes.len() {
        return None;
    }
    if &bytes[i..i + kw_bytes.len()] != kw_bytes {
        return None;
    }
    let after = i + kw_bytes.len();
    // Whole-word boundary: next char must be whitespace, `(`, `,`, or end.
    if after < bytes.len()
        && !bytes[after].is_ascii_whitespace()
        && bytes[after] != b'('
        && bytes[after] != b','
    {
        return None;
    }
    Some(after)
}

/// Split `s` on top-level commas (those at paren depth 0), returning owned
/// slices of the original. Whitespace is NOT trimmed here — callers trim.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            ',' if depth == 0 => {
                parts.push(&s[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parse the inner content of one `(feature)` or `(feature: value)` condition
/// (already lowercased, no surrounding parens).
fn parse_condition(inner: &str) -> Result<MediaCondition> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Err(CssError::invalid_selector(
            "media query: empty condition `()`",
        ));
    }
    // Split on `:` if present.
    if let Some(colon) = trimmed.find(':') {
        let feature = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim();
        parse_feature_value(feature, value)
    } else {
        parse_feature_bare(trimmed)
    }
}

/// Parse a bare `(feature)` condition (no value).
fn parse_feature_bare(feature: &str) -> Result<MediaCondition> {
    match feature {
        "min-width" | "max-width" | "width" | "min-height" | "max-height" | "height" => {
            Err(CssError::invalid_selector(format!(
                "media query: `({feature})` requires a value, e.g. `({feature}: 80)`"
            )))
        }
        "color" => Ok(MediaCondition::Color),
        "monochrome" => Ok(MediaCondition::Monochrome),
        "truecolor" => Ok(MediaCondition::Truecolor),
        other => Err(CssError::invalid_selector(format!(
            "media query: unknown feature `{other}`"
        ))),
    }
}

/// Parse a `(feature: value)` condition.
fn parse_feature_value(feature: &str, value: &str) -> Result<MediaCondition> {
    match feature {
        "min-width" => Ok(MediaCondition::MinWidth(parse_u16(value, "min-width")?)),
        "max-width" => Ok(MediaCondition::MaxWidth(parse_u16(value, "max-width")?)),
        "width" => Ok(MediaCondition::Width(parse_u16(value, "width")?)),
        "min-height" => Ok(MediaCondition::MinHeight(parse_u16(value, "min-height")?)),
        "max-height" => Ok(MediaCondition::MaxHeight(parse_u16(value, "max-height")?)),
        "height" => Ok(MediaCondition::Height(parse_u16(value, "height")?)),
        "color" => {
            // `(color: 0)` → Monochrome; `(color: N)` with N>=1 → Color.
            match value {
                "0" => Ok(MediaCondition::Monochrome),
                _ => {
                    // Validate it's a non-negative number for the error path,
                    // then treat any nonzero as Color.
                    let n = parse_u16(value, "color")?;
                    if n == 0 {
                        Ok(MediaCondition::Monochrome)
                    } else {
                        Ok(MediaCondition::Color)
                    }
                }
            }
        }
        "monochrome" => {
            // `(monochrome: 0)` → Color; otherwise Monochrome.
            match value {
                "0" => Ok(MediaCondition::Color),
                _ => {
                    let n = parse_u16(value, "monochrome")?;
                    if n == 0 {
                        Ok(MediaCondition::Color)
                    } else {
                        Ok(MediaCondition::Monochrome)
                    }
                }
            }
        }
        "truecolor" => {
            // `(truecolor: 1)` → Truecolor; `(truecolor: 0)` → treat as
            // negation? For v1, keep simple: only `1` (or bare) means truecolor.
            let n = parse_u16(value, "truecolor")?;
            if n >= 1 {
                Ok(MediaCondition::Truecolor)
            } else {
                // A `(truecolor: 0)` query is nonsensical in the AND model
                // (it would need a NOT). Surface as an error for v1.
                Err(CssError::invalid_selector(
                    "media query: `(truecolor: 0)` is not supported — use a separate context",
                ))
            }
        }
        other => Err(CssError::invalid_selector(format!(
            "media query: unknown feature `{other}`"
        ))),
    }
}

/// Parse a `u16` value, rejecting negatives and non-numeric text.
fn parse_u16(value: &str, feature: &str) -> Result<u16> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CssError::invalid_selector(format!(
            "media query: `({feature}:)` has no value"
        )));
    }
    // Reject a leading `-` explicitly (u16 parse would reject it anyway, but
    // give a clearer message).
    if trimmed.starts_with('-') {
        return Err(CssError::invalid_selector(format!(
            "media query: `({feature}: {trimmed})` value must be non-negative"
        )));
    }
    trimmed.parse::<u16>().map_err(|_| {
        CssError::invalid_selector(format!(
            "media query: `({feature}: {trimmed})` value is not a number"
        ))
    })
}

impl MediaCondition {
    /// True iff this single condition holds against `ctx`.
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        match *self {
            MediaCondition::MinWidth(n) => ctx.cols >= n,
            MediaCondition::MaxWidth(n) => ctx.cols <= n,
            MediaCondition::Width(n) => ctx.cols == n,
            MediaCondition::MinHeight(n) => ctx.rows >= n,
            MediaCondition::MaxHeight(n) => ctx.rows <= n,
            MediaCondition::Height(n) => ctx.rows == n,
            MediaCondition::Color => !ctx.no_color,
            MediaCondition::Monochrome => ctx.no_color,
            MediaCondition::Truecolor => ctx.truecolor,
        }
    }
}

impl std::fmt::Display for MediaQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.alternatives.is_empty() {
            return write!(f, "all");
        }
        for (i, a) in self.alternatives.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            std::fmt::Display::fmt(a, f)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for MediaAlternative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negated {
            write!(f, "not ")?;
        }
        for (i, c) in self.conditions.iter().enumerate() {
            if i > 0 {
                write!(f, " and ")?;
            }
            std::fmt::Display::fmt(c, f)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for MediaCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            MediaCondition::MinWidth(n) => write!(f, "(min-width: {n})"),
            MediaCondition::MaxWidth(n) => write!(f, "(max-width: {n})"),
            MediaCondition::Width(n) => write!(f, "(width: {n})"),
            MediaCondition::MinHeight(n) => write!(f, "(min-height: {n})"),
            MediaCondition::MaxHeight(n) => write!(f, "(max-height: {n})"),
            MediaCondition::Height(n) => write!(f, "(height: {n})"),
            MediaCondition::Color => write!(f, "(color)"),
            MediaCondition::Monochrome => write!(f, "(monochrome)"),
            MediaCondition::Truecolor => write!(f, "(truecolor)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(cols: u16, rows: u16) -> MediaContext {
        MediaContext {
            cols,
            rows,
            truecolor: false,
            no_color: false,
        }
    }

    /// Build a `MediaAlternative` from a `negated` flag + a list of conditions.
    fn alt<I: IntoIterator<Item = MediaCondition>>(negated: bool, conds: I) -> MediaAlternative {
        MediaAlternative { negated, conditions: conds.into_iter().collect() }
    }

    fn no_color_ctx() -> MediaContext {
        MediaContext { no_color: true, ..Default::default() }
    }

    // --- parse ---------------------------------------------------------------

    #[test]
    fn parse_min_width() {
        let q = MediaQuery::parse("(min-width: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::MinWidth(80)])]);
    }

    #[test]
    fn parse_max_width_and_min_height() {
        let q = MediaQuery::parse("(max-width: 120) and (min-height: 24)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![alt(false, [MediaCondition::MaxWidth(120), MediaCondition::MinHeight(24)])]
        );
    }

    #[test]
    fn parse_width_exact() {
        let q = MediaQuery::parse("(width: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::Width(80)])]);
    }

    #[test]
    fn parse_color_bare() {
        let q = MediaQuery::parse("(color)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::Color])]);
    }

    #[test]
    fn parse_monochrome_bare() {
        let q = MediaQuery::parse("(monochrome)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::Monochrome])]);
    }

    #[test]
    fn parse_truecolor_bare() {
        let q = MediaQuery::parse("(truecolor)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::Truecolor])]);
    }

    #[test]
    fn parse_leading_media_type_ignored() {
        let q = MediaQuery::parse("screen and (min-width: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::MinWidth(80)])]);

        let q2 = MediaQuery::parse("all and (max-height: 40)").unwrap();
        assert_eq!(q2.alternatives, vec![alt(false, [MediaCondition::MaxHeight(40)])]);
    }

    #[test]
    fn parse_empty_query_matches_all() {
        let q = MediaQuery::parse("").unwrap();
        assert!(q.alternatives.is_empty());
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn parse_uppercase_features() {
        // Case-insensitive: features get lowercased internally.
        let q = MediaQuery::parse("(MIN-WIDTH: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(false, [MediaCondition::MinWidth(80)])]);
    }

    // --- parse: not / comma / and --------------------------------------------

    #[test]
    fn parse_comma_or_two_alternatives() {
        let q = MediaQuery::parse("(min-width: 80), (max-width: 120)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![
                alt(false, [MediaCondition::MinWidth(80)]),
                alt(false, [MediaCondition::MaxWidth(120)]),
            ]
        );
    }

    #[test]
    fn parse_not_prefix_single() {
        let q = MediaQuery::parse("not (min-width: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(true, [MediaCondition::MinWidth(80)])]);
    }

    #[test]
    fn parse_comma_three_alternatives() {
        let q = MediaQuery::parse("(min-width: 80), (max-width: 120), (color)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![
                alt(false, [MediaCondition::MinWidth(80)]),
                alt(false, [MediaCondition::MaxWidth(120)]),
                alt(false, [MediaCondition::Color]),
            ]
        );
    }

    #[test]
    fn parse_not_screen_media_type_ignored() {
        // `not` negates the alternative; media type `screen` is ignored.
        let q = MediaQuery::parse("not screen and (min-width: 80)").unwrap();
        assert_eq!(q.alternatives, vec![alt(true, [MediaCondition::MinWidth(80)])]);
    }

    #[test]
    fn parse_comma_with_not_second_alt() {
        let q = MediaQuery::parse("(min-width: 80), not (color)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![
                alt(false, [MediaCondition::MinWidth(80)]),
                alt(true, [MediaCondition::Color]),
            ]
        );
    }

    #[test]
    fn parse_and_chain_one_alternative_regression() {
        // Existing AND behavior: one alternative, two conditions.
        let q = MediaQuery::parse("(min-width: 80) and (max-height: 40)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![alt(
                false,
                [MediaCondition::MinWidth(80), MediaCondition::MaxHeight(40)]
            )]
        );
    }

    #[test]
    fn parse_not_is_whole_word() {
        // `notable` must NOT be parsed as `not`.
        assert!(MediaQuery::parse("notable").is_err());
    }

    #[test]
    fn parse_bare_media_type_matches_all() {
        // Bare media type, no conditions → one alternative with zero conditions
        // (matches everything before negation).
        let q = MediaQuery::parse("screen").unwrap();
        assert_eq!(q.alternatives, vec![MediaAlternative { negated: false, conditions: vec![] }]);
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn parse_empty_alternative_errors() {
        // Stray trailing comma → empty alternative is a structural error.
        assert!(MediaQuery::parse("(min-width: 80),").is_err());
        assert!(MediaQuery::parse(", (min-width: 80)").is_err());
    }

    // --- parse errors --------------------------------------------------------

    #[test]
    fn parse_unknown_feature_errors() {
        assert!(MediaQuery::parse("(foo: 1)").is_err());
    }

    #[test]
    fn parse_non_numeric_width_errors() {
        assert!(MediaQuery::parse("(min-width: wide)").is_err());
    }

    #[test]
    fn parse_unbalanced_parens_error() {
        assert!(MediaQuery::parse("(min-width: 80").is_err());
    }

    #[test]
    fn parse_missing_value_errors() {
        assert!(MediaQuery::parse("(min-width)").is_err());
    }

    #[test]
    fn parse_negative_value_errors() {
        assert!(MediaQuery::parse("(min-width: -5)").is_err());
    }

    // --- matches -------------------------------------------------------------

    #[test]
    fn min_width_matches() {
        let q = MediaQuery::parse("(min-width: 80)").unwrap();
        assert!(q.matches(&ctx(100, 24)));
        assert!(q.matches(&ctx(80, 24)));
        assert!(!q.matches(&ctx(60, 24)));
    }

    #[test]
    fn max_width_and_min_height_matches() {
        let q = MediaQuery::parse("(max-width: 120) and (min-height: 24)").unwrap();
        // Both hold.
        assert!(q.matches(&ctx(100, 24)));
        assert!(q.matches(&ctx(120, 30)));
        // One fails.
        assert!(!q.matches(&ctx(200, 24))); // width too big
        assert!(!q.matches(&ctx(100, 10))); // height too small
        assert!(!q.matches(&ctx(200, 10))); // both fail
    }

    #[test]
    fn truecolor_only_when_flag_set() {
        let q = MediaQuery::parse("(truecolor)").unwrap();
        assert!(!q.matches(&MediaContext { truecolor: false, ..Default::default() }));
        assert!(q.matches(&MediaContext { truecolor: true, ..Default::default() }));
    }

    #[test]
    fn monochrome_only_when_no_color() {
        let q = MediaQuery::parse("(monochrome)").unwrap();
        assert!(!q.matches(&MediaContext { no_color: false, ..Default::default() }));
        assert!(q.matches(&MediaContext { no_color: true, ..Default::default() }));
    }

    #[test]
    fn color_inverts_monochrome() {
        let color_q = MediaQuery::parse("(color)").unwrap();
        assert!(color_q.matches(&MediaContext { no_color: false, ..Default::default() }));
        assert!(!color_q.matches(&MediaContext { no_color: true, ..Default::default() }));
    }

    #[test]
    fn default_context_does_not_match_gated_query() {
        // A default (all-zero) context must NOT satisfy min-width: 80.
        let q = MediaQuery::parse("(min-width: 80)").unwrap();
        assert!(!q.matches(&MediaContext::default()));
    }

    // --- matches: not / comma / and -----------------------------------------

    #[test]
    fn comma_or_matches_either_alternative() {
        // (min-width: 100), (max-width: 50)
        let q = MediaQuery::parse("(min-width: 100), (max-width: 50)").unwrap();
        // First alt: cols >= 100.
        assert!(q.matches(&ctx(100, 24)));
        assert!(q.matches(&ctx(150, 24)));
        // Second alt: cols <= 50.
        assert!(q.matches(&ctx(40, 24)));
        assert!(q.matches(&ctx(50, 24)));
        // Neither: cols = 70.
        assert!(!q.matches(&ctx(70, 24)));
    }

    #[test]
    fn not_prefix_inverts_single_condition() {
        // not (min-width: 80): matches when cols < 80.
        let q = MediaQuery::parse("not (min-width: 80)").unwrap();
        assert!(q.matches(&ctx(60, 24))); // condition false → negated true
        assert!(q.matches(&ctx(79, 24)));
        assert!(!q.matches(&ctx(100, 24))); // condition true → negated false
        assert!(!q.matches(&ctx(80, 24)));
    }

    #[test]
    fn and_chain_matches_only_when_all_hold() {
        // (min-width: 80) and (max-width: 120)
        let q = MediaQuery::parse("(min-width: 80) and (max-width: 120)").unwrap();
        assert!(q.matches(&ctx(100, 24)));
        assert!(q.matches(&ctx(80, 24)));
        assert!(q.matches(&ctx(120, 24)));
        assert!(!q.matches(&ctx(60, 24))); // below min
        assert!(!q.matches(&ctx(200, 24))); // above max
    }

    #[test]
    fn comma_with_not_second_alt() {
        // (min-width: 200), not (color) against a no_color ctx → second alt.
        let q = MediaQuery::parse("(min-width: 200), not (color)").unwrap();
        // no_color ctx: (color) is false, so `not (color)` is true → matches.
        assert!(q.matches(&no_color_ctx()));
        // A color ctx with cols < 200: (min-width: 200) false, (color) true so
        // `not (color)` false → neither alt matches.
        assert!(!q.matches(&ctx(100, 24)));
        // Color ctx with cols >= 200: first alt matches.
        assert!(q.matches(&ctx(200, 24)));
    }

    #[test]
    fn not_all_conditions_in_one_alternative() {
        // not ((min-width: 80) and (color)): negates the whole conjunction.
        let q = MediaQuery::parse("not (min-width: 80) and (color)").unwrap();
        // cols=100 + color → conjunction true → negated false.
        assert!(!q.matches(&ctx(100, 24)));
        // cols=60 + color → conjunction false → negated true.
        assert!(q.matches(&ctx(60, 24)));
        // no_color: (color) false → conjunction false → negated true.
        assert!(q.matches(&no_color_ctx()));
    }

    // --- Display -------------------------------------------------------------

    #[test]
    fn display_roundtrip() {
        let q = MediaQuery::parse("(min-width: 80) and (color)").unwrap();
        assert_eq!(q.to_string(), "(min-width: 80) and (color)");
    }

    #[test]
    fn display_roundtrip_comma_and_not() {
        let q = MediaQuery::parse("(min-width: 80), not (color)").unwrap();
        assert_eq!(q.to_string(), "(min-width: 80), not (color)");
    }

    // --- and (nested @media combinator) -------------------------------------

    #[test]
    fn media_query_and_concatenates_conditions() {
        // (min-width: 80).and((color)) → one alternative with BOTH conditions;
        // matches only when both hold.
        let q1 = MediaQuery::parse("(min-width: 80)").unwrap();
        let q2 = MediaQuery::parse("(color)").unwrap();
        let combined = q1.and(&q2);
        assert_eq!(
            combined.alternatives,
            vec![alt(false, [MediaCondition::MinWidth(80), MediaCondition::Color])],
            "AND of two single-condition queries concatenates conditions"
        );
        // Matches only when both hold.
        let both = MediaContext { cols: 100, no_color: false, ..Default::default() };
        let width_only = MediaContext { cols: 100, no_color: true, ..Default::default() };
        let color_only = MediaContext { cols: 60, no_color: false, ..Default::default() };
        let neither = MediaContext { cols: 60, no_color: true, ..Default::default() };
        assert!(combined.matches(&both), "both hold → matches");
        assert!(!combined.matches(&width_only), "color missing → no match");
        assert!(!combined.matches(&color_only), "width missing → no match");
        assert!(!combined.matches(&neither), "neither → no match");
    }

    #[test]
    fn media_query_and_cross_product() {
        // (a),(b) AND (c) → two alternatives (a,c),(b,c).
        let q1 = MediaQuery::parse("(min-width: 80), (max-width: 40)").unwrap();
        let q2 = MediaQuery::parse("(color)").unwrap();
        let combined = q1.and(&q2);
        assert_eq!(
            combined.alternatives,
            vec![
                alt(false, [MediaCondition::MinWidth(80), MediaCondition::Color]),
                alt(false, [MediaCondition::MaxWidth(40), MediaCondition::Color]),
            ],
            "OR cross-product with AND concatenates per-alternative"
        );
        // cols:100, color → first alt matches.
        let large_color = MediaContext { cols: 100, no_color: false, ..Default::default() };
        assert!(combined.matches(&large_color));
        // cols:30, color → second alt matches.
        let small_color = MediaContext { cols: 30, no_color: false, ..Default::default() };
        assert!(combined.matches(&small_color));
        // cols:50, color → neither alt (50 is between 40 and 80).
        let mid_color = MediaContext { cols: 50, no_color: false, ..Default::default() };
        assert!(!combined.matches(&mid_color));
        // cols:100, no color → first alt's color condition fails; second alt's
        // width fails too.
        let large_mono = MediaContext { cols: 100, no_color: true, ..Default::default() };
        assert!(!combined.matches(&large_mono));
    }

    #[test]
    fn media_query_and_empty_short_circuit() {
        // empty AND other == other; other AND empty == other.
        let empty = MediaQuery::default();
        let other = MediaQuery::parse("(min-width: 80)").unwrap();
        assert_eq!(empty.and(&other), other, "match-all AND other == other");
        assert_eq!(other.and(&empty), other, "other AND match-all == other");
        // Both empty → empty.
        assert_eq!(empty.and(&MediaQuery::default()), MediaQuery::default());
    }

    #[test]
    fn media_query_and_negation_is_approximate() {
        // (not (min-width: 80)).and((color)): the documented v1 approximation
        // propagates negation onto the combined alternative. So the combined
        // alternative is `not ((min-width: 80) and (color))`.
        //
        // This is NOT the precise semantics of `(¬min-width:80) ∧ (color)` —
        // that would require a per-condition negation model we don't have. The
        // test pins the documented approximation so a future change is caught.
        let not_q = MediaQuery::parse("not (min-width: 80)").unwrap();
        let color_q = MediaQuery::parse("(color)").unwrap();
        let combined = not_q.and(&color_q);
        // One alternative, negated, with both conditions.
        assert_eq!(
            combined.alternatives,
            vec![alt(true, [MediaCondition::MinWidth(80), MediaCondition::Color])],
            "negation propagates to the combined alternative (approximation)"
        );
        // Under the approximation: `not ((min-width: 80) and (color))`.
        // cols:100, color → both hold → negated → no match.
        let large_color = MediaContext { cols: 100, no_color: false, ..Default::default() };
        assert!(
            !combined.matches(&large_color),
            "approximation: both hold → negated → no match"
        );
        // cols:60, color → min-width fails → conjunction false → negated true.
        let small_color = MediaContext { cols: 60, no_color: false, ..Default::default() };
        assert!(
            combined.matches(&small_color),
            "approximation: one fails → negated → matches"
        );
    }

    #[test]
    fn matching_specificity_returns_max_condition_count() {
        // A 2-condition matching alternative is more specific than a 1-condition
        // one in the same query (under OR).
        let q = MediaQuery::parse("(min-width: 80) and (color), (max-width: 40)").unwrap();
        // cols:100, color → first alt (2 conds) matches → specificity 2.
        let large_color = MediaContext { cols: 100, no_color: false, ..Default::default() };
        assert_eq!(q.matching_specificity(&large_color), Some(2));
        // cols:30, color → only second alt (1 cond) matches → specificity 1.
        let small_color = MediaContext { cols: 30, no_color: false, ..Default::default() };
        assert_eq!(q.matching_specificity(&small_color), Some(1));
    }

    #[test]
    fn matching_specificity_none_when_no_match() {
        let q = MediaQuery::parse("(min-width: 80)").unwrap();
        // cols:60 → no match → None.
        let small = MediaContext { cols: 60, ..Default::default() };
        assert_eq!(q.matching_specificity(&small), None);
    }

    #[test]
    fn matching_specificity_zero_for_match_all() {
        let empty = MediaQuery::default();
        assert_eq!(empty.matching_specificity(&MediaContext::default()), Some(0));
    }
}
