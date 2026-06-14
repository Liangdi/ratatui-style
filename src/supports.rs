//! `@supports` query support — conditional rules based on terminal/engine
//! capability.
//!
//! Where [`@media`](crate::media) gates rules on the *current* viewport/state
//! (width, color mode), `@supports` gates rules on *capability*: does the
//! terminal advertise truecolor, does it have color at all, and does the
//! engine recognize a given CSS property. A [`SupportsQuery`] is parsed from
//! the text between `@supports` and the block's opening `{`; each element rule
//! inside is tagged with it, and the cascade skips a tagged rule unless
//! [`SupportsQuery::matches`] the active [`MediaContext`] (capability
//! conditions) and the engine's known-property set (property conditions).
//!
//! # Reusing `MediaContext`
//!
//! There is no separate "supports context". Capability conditions
//! ([`Truecolor`], [`Color`], [`Monochrome`], [`NoColor`]) evaluate against the
//! *same* [`MediaContext`] that `@media` uses — `truecolor` and `no_color` are
//! already capability flags there. Property conditions ([`Property`],
//! [`PropertyValue`]) evaluate against [`stylesheet::is_known_property`].
//!
//! [`Truecolor`]: SupportsCondition::Truecolor
//! [`Color`]: SupportsCondition::Color
//! [`Monochrome`]: SupportsCondition::Monochrome
//! [`NoColor`]: SupportsCondition::NoColor
//! [`Property`]: SupportsCondition::Property
//! [`PropertyValue`]: SupportsCondition::PropertyValue
//! [`stylesheet::is_known_property`]: crate::stylesheet::is_known_property
//!
//! # Matching model
//!
//! A query is a comma-separated list (logical OR) of [`SupportsAlternative`]s.
//! Each alternative is a conjunction (logical AND) of [`SupportsTerm`]s, where
//! each term is a [`SupportsCondition`] optionally prefixed by `not` (per-term
//! negation, following CSS4 feature negation — the same algebra as
//! [`MediaQuery`](crate::media::MediaQuery)). Precedence, tightest first:
//!
//! `not` (per-term) > `and` (terms within an alternative) > `,` (alternatives / OR)
//!
//! So `not (truecolor) and (border-style)` is ONE alternative with TWO terms:
//! `[¬(truecolor), (border-style)]` — it matches iff the terminal is NOT
//! truecolor AND the engine knows `border-style`. A leading `not` binds to the
//! immediately following feature only, not to the whole alternative.
//!
//! A query with **no alternatives** (e.g. a bare `@supports {}` with no query
//! text) matches anything — a no-op gate — mirroring [`MediaQuery`].
//!
//! Default-context caution: [`MediaContext::default()`] is all-zero / all-false.
//! A capability-gated rule with any condition will NOT match a default context
//! (e.g. `(truecolor)` vs `truecolor = false` is false) — the same caveat as
//! `@media`.

use crate::error::{CssError, Result};
use crate::media::MediaContext;

/// One `@supports` query: one or more [`SupportsAlternative`]s joined by comma
/// (OR).
///
/// The query matches if **any** alternative matches. An `alternatives` list
/// with zero entries (e.g. a bare `@supports {}` with no query text) matches
/// everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportsQuery {
    /// Comma-separated alternatives; the query matches if ANY matches. An
    /// empty list matches everything (a no-op gate).
    pub alternatives: Vec<SupportsAlternative>,
}

/// One `and`-conjunction of [`SupportsTerm`]s (no whole-alternative negation).
///
/// `matches` is true iff **every** term holds (each term already accounts for
/// its own per-term `not`). Negation lives on individual terms, not on the
/// alternative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupportsAlternative {
    /// `and`-joined terms; ALL must hold (each term: condition XOR `negated`).
    pub terms: Vec<SupportsTerm>,
}

/// One condition in a supports alternative, optionally negated (`not (feat)`).
///
/// Following CSS4 feature negation, `not` applies to the immediately following
/// feature only — it is per-term, not per-alternative. A term matches `ctx`
/// iff `cond.matches(ctx) != negated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportsTerm {
    /// A leading `not` negates this single condition.
    pub negated: bool,
    /// The underlying capability/property condition.
    pub cond: SupportsCondition,
}

impl SupportsTerm {
    /// True iff `cond` holds against `ctx`, XOR `negated`.
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        self.cond.matches(ctx) != self.negated
    }
}

/// A capability or property-support condition for `@supports`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportsCondition {
    /// `(truecolor)` — terminal advertises 24-bit color.
    Truecolor,
    /// `(color)` — terminal has color capability (i.e. NOT no-color).
    Color,
    /// `(monochrome)` — terminal is monochrome (no color).
    Monochrome,
    /// `(no-color)` — alias for [`Monochrome`](Self::Monochrome).
    NoColor,
    /// `(prop)` — the engine recognizes this CSS property name.
    Property(String),
    /// `(prop: value)` — the engine recognizes `prop` (value is loosely
    /// checked: accepted as known iff the property is known; v1 does not
    /// validate the value against the property's grammar).
    PropertyValue(String, String),
}

impl SupportsQuery {
    /// True iff **any** alternative matches `ctx`. A query with no alternatives
    /// matches anything (a no-op gate).
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        if self.alternatives.is_empty() {
            return true;
        }
        self.alternatives.iter().any(|a| a.matches(ctx))
    }

    /// Parse the text BETWEEN `@supports` and the block's opening `{`.
    ///
    /// Grammar (case-insensitive), precedence tightest first:
    ///
    /// `not` (per-term) > `and` (terms within an alternative) > `,` (alternatives / OR)
    ///
    /// - The text is split on top-level commas into one [`SupportsAlternative`]
    ///   per part (OR).
    /// - Each part is zero or more terms joined by `and`. Each term is an
    ///   optional leading `not` (per-term negation) followed by a
    ///   `(condition)` clause. So `not (a) and (b)` → two terms `[¬a, b]`.
    /// - Each condition is one of: `(truecolor)` / `(color)` / `(monochrome)`
    ///   / `(no-color)` / `(prop)` / `(prop: value)`. A property condition's
    ///   name is any identifier; the engine's known-property set decides
    ///   whether it matches at evaluation time (so parsing never errors on an
    ///   unknown property name — only on structural problems).
    ///
    /// Unknown / malformed *structure* surfaces as a [`CssError`].
    pub fn parse(s: &str) -> Result<SupportsQuery> {
        let lower = s.to_ascii_lowercase();

        if lower.trim().is_empty() {
            return Ok(SupportsQuery { alternatives: Vec::new() });
        }

        let mut alternatives = Vec::new();
        for part in split_top_level_commas(&lower) {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return Err(CssError::invalid_selector(
                    "supports query: empty alternative (stray comma?)",
                ));
            }
            alternatives.push(parse_alternative(trimmed)?);
        }

        Ok(SupportsQuery { alternatives })
    }
}

impl SupportsAlternative {
    /// True iff **every** term holds against `ctx`. Each term already accounts
    /// for its own per-term `not`.
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        self.terms.iter().all(|t| t.matches(ctx))
    }
}

/// Parse one comma-separated alternative (already trimmed, lowercased).
///
/// Parses the remainder as `and`-joined terms — each term being an optional
/// leading `not` (per-term negation) followed by a `(condition)`.
fn parse_alternative(part: &str) -> Result<SupportsAlternative> {
    let bytes = part.as_bytes();
    let mut i = 0usize;

    let mut terms = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        // Per-term `not` (CSS4 feature negation): a `not` immediately before a
        // `(feature)` negates that single feature.
        let mut negated = false;
        if let Some(consumed) = consume_keyword(bytes, i, "not") {
            negated = true;
            i = consumed;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }

        if i >= bytes.len() {
            return Err(CssError::invalid_selector(
                "supports query: `not` at end of alternative (nothing to negate)",
            ));
        }
        if bytes[i] != b'(' {
            return Err(CssError::invalid_selector(format!(
                "supports query: expected `(` near `{}`",
                &part[i.min(part.len())..]
            )));
        }
        let close = match part[i..].find(')') {
            Some(rel) => i + rel,
            None => {
                return Err(CssError::invalid_selector(
                    "supports query: unbalanced parens (missing `)`)",
                ));
            }
        };
        let inner = &part[i + 1..close];
        let cond = parse_condition(inner)?;
        terms.push(SupportsTerm { negated, cond });
        i = close + 1;

        // After a term, expect either end-of-string or ` and `.
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
            "supports query: expected `and` between terms near `{}`",
            &part[i..]
        )));
    }

    Ok(SupportsAlternative { terms })
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

/// Parse the inner content of one `(condition)` (already lowercased, no
/// surrounding parens).
///
/// Recognized bare keywords: `truecolor`, `color`, `monochrome`, `no-color`.
/// Anything else (with or without a `: value`) is a property condition — the
/// property name is stored verbatim and matched against the engine's
/// known-property set at evaluation time, so an unknown property name parses
/// fine and simply fails to match.
fn parse_condition(inner: &str) -> Result<SupportsCondition> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return Err(CssError::invalid_selector(
            "supports query: empty condition `()`",
        ));
    }
    // A colon splits `(prop: value)` from a bare `(prop)` / `(keyword)`.
    if let Some(colon) = trimmed.find(':') {
        let name = trimmed[..colon].trim();
        let value = trimmed[colon + 1..].trim().to_string();
        if name.is_empty() {
            return Err(CssError::invalid_selector(
                "supports query: `(: value)` has no property name",
            ));
        }
        // A keyword with a value is nonsensical — but tolerate it as a property
        // named after the keyword (e.g. `(color: red)` would be treated as a
        // property named "color", which is_known_property recognizes, so it
        // matches). This mirrors the spec's `supports(prop: value)` form.
        Ok(SupportsCondition::PropertyValue(name.to_string(), value))
    } else {
        match trimmed {
            "truecolor" => Ok(SupportsCondition::Truecolor),
            "color" => Ok(SupportsCondition::Color),
            "monochrome" => Ok(SupportsCondition::Monochrome),
            "no-color" | "nocolor" => Ok(SupportsCondition::NoColor),
            other => Ok(SupportsCondition::Property(other.to_string())),
        }
    }
}

impl SupportsCondition {
    /// True iff this single condition holds.
    ///
    /// Capability conditions (`truecolor`/`color`/`monochrome`/`no-color`)
    /// evaluate against the [`MediaContext`] flags; property conditions
    /// (`prop`/`prop: value`) evaluate against the engine's known-property
    /// set via [`is_known_property`]. For `(prop: value)` the value is
    /// ignored in v1 — only the property name is checked.
    ///
    /// [`is_known_property`]: crate::stylesheet::is_known_property
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        match self {
            SupportsCondition::Truecolor => ctx.truecolor,
            SupportsCondition::Color => !ctx.no_color,
            SupportsCondition::Monochrome | SupportsCondition::NoColor => ctx.no_color,
            SupportsCondition::Property(p) | SupportsCondition::PropertyValue(p, _) => {
                crate::stylesheet::is_known_property(p)
            }
        }
    }
}

impl std::fmt::Display for SupportsQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, a) in self.alternatives.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            std::fmt::Display::fmt(a, f)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for SupportsAlternative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, t) in self.terms.iter().enumerate() {
            if i > 0 {
                write!(f, " and ")?;
            }
            std::fmt::Display::fmt(t, f)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for SupportsTerm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negated {
            write!(f, "not ")?;
        }
        std::fmt::Display::fmt(&self.cond, f)
    }
}

impl std::fmt::Display for SupportsCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportsCondition::Truecolor => write!(f, "(truecolor)"),
            SupportsCondition::Color => write!(f, "(color)"),
            SupportsCondition::Monochrome => write!(f, "(monochrome)"),
            SupportsCondition::NoColor => write!(f, "(no-color)"),
            SupportsCondition::Property(p) => write!(f, "({p})"),
            SupportsCondition::PropertyValue(p, v) => write!(f, "({p}: {v})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_truecolor() -> MediaContext {
        MediaContext { truecolor: true, ..Default::default() }
    }
    fn ctx_no_truecolor() -> MediaContext {
        MediaContext { truecolor: false, ..Default::default() }
    }
    fn ctx_no_color() -> MediaContext {
        MediaContext { no_color: true, ..Default::default() }
    }
    fn ctx_color() -> MediaContext {
        MediaContext { no_color: false, ..Default::default() }
    }

    /// Build a `SupportsAlternative` from a list of conditions (none negated).
    fn alt<I: IntoIterator<Item = SupportsCondition>>(conds: I) -> SupportsAlternative {
        SupportsAlternative {
            terms: conds
                .into_iter()
                .map(|c| SupportsTerm { negated: false, cond: c })
                .collect(),
        }
    }

    fn term(c: SupportsCondition) -> SupportsTerm {
        SupportsTerm { negated: false, cond: c }
    }
    fn not_term(c: SupportsCondition) -> SupportsTerm {
        SupportsTerm { negated: true, cond: c }
    }
    fn alt_terms<I: IntoIterator<Item = SupportsTerm>>(terms: I) -> SupportsAlternative {
        SupportsAlternative { terms: terms.into_iter().collect() }
    }
    fn alt_neg(c: SupportsCondition) -> SupportsAlternative {
        SupportsAlternative { terms: vec![not_term(c)] }
    }

    // --- parse ---------------------------------------------------------------

    #[test]
    fn parse_truecolor() {
        let q = SupportsQuery::parse("(truecolor)").unwrap();
        assert_eq!(q.alternatives, vec![alt([SupportsCondition::Truecolor])]);
    }

    #[test]
    fn parse_color_and_property() {
        let q = SupportsQuery::parse("(color) and (border-style)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![alt([
                SupportsCondition::Color,
                SupportsCondition::Property("border-style".into()),
            ])]
        );
    }

    #[test]
    fn parse_not_truecolor() {
        let q = SupportsQuery::parse("not (truecolor)").unwrap();
        assert_eq!(q.alternatives, vec![alt_neg(SupportsCondition::Truecolor)]);
    }

    #[test]
    fn parse_property_value() {
        let q = SupportsQuery::parse("(border-style: rounded)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![alt([SupportsCondition::PropertyValue(
                "border-style".into(),
                "rounded".into()
            )])]
        );
    }

    #[test]
    fn parse_comma_or() {
        let q = SupportsQuery::parse("(truecolor), (color)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![
                alt([SupportsCondition::Truecolor]),
                alt([SupportsCondition::Color]),
            ]
        );
    }

    #[test]
    fn parse_per_term_not() {
        // `not (truecolor) and (color)` → one alternative, two terms (first negated).
        let q = SupportsQuery::parse("not (truecolor) and (color)").unwrap();
        assert_eq!(
            q.alternatives,
            vec![alt_terms([
                not_term(SupportsCondition::Truecolor),
                term(SupportsCondition::Color),
            ])]
        );
    }

    #[test]
    fn parse_monochrome_and_no_color_aliases() {
        assert_eq!(
            SupportsQuery::parse("(monochrome)").unwrap().alternatives,
            vec![alt([SupportsCondition::Monochrome])]
        );
        assert_eq!(
            SupportsQuery::parse("(no-color)").unwrap().alternatives,
            vec![alt([SupportsCondition::NoColor])]
        );
        // nocolor (no hyphen) also tolerated as the alias.
        assert_eq!(
            SupportsQuery::parse("(nocolor)").unwrap().alternatives,
            vec![alt([SupportsCondition::NoColor])]
        );
    }

    #[test]
    fn parse_empty_query_matches_all() {
        let q = SupportsQuery::parse("").unwrap();
        assert!(q.alternatives.is_empty());
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn parse_not_is_whole_word() {
        // `notable` must NOT be parsed as `not`.
        assert!(SupportsQuery::parse("notable").is_err());
    }

    #[test]
    fn parse_empty_alternative_errors() {
        assert!(SupportsQuery::parse("(truecolor),").is_err());
        assert!(SupportsQuery::parse(", (truecolor)").is_err());
    }

    #[test]
    fn parse_unbalanced_parens_error() {
        assert!(SupportsQuery::parse("(truecolor").is_err());
    }

    #[test]
    fn parse_not_at_end_errors() {
        assert!(SupportsQuery::parse("(truecolor) and not").is_err());
    }

    // --- matches -------------------------------------------------------------

    #[test]
    fn truecolor_matches_only_when_flag_set() {
        let q = SupportsQuery::parse("(truecolor)").unwrap();
        assert!(q.matches(&ctx_truecolor()));
        assert!(!q.matches(&ctx_no_truecolor()));
    }

    #[test]
    fn color_matches_only_when_color_capable() {
        let q = SupportsQuery::parse("(color)").unwrap();
        assert!(q.matches(&ctx_color()));
        assert!(!q.matches(&ctx_no_color()));
    }

    #[test]
    fn monochrome_matches_only_when_no_color() {
        let q = SupportsQuery::parse("(monochrome)").unwrap();
        assert!(q.matches(&ctx_no_color()));
        assert!(!q.matches(&ctx_color()));
    }

    #[test]
    fn no_color_alias_matches_monochrome() {
        let q = SupportsQuery::parse("(no-color)").unwrap();
        assert!(q.matches(&ctx_no_color()));
        assert!(!q.matches(&ctx_color()));
    }

    #[test]
    fn property_matches_known_regardless_of_ctx() {
        // `border-style` is a known engine property.
        let q = SupportsQuery::parse("(border-style)").unwrap();
        assert!(q.matches(&ctx_truecolor()));
        assert!(q.matches(&ctx_no_truecolor()));
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn unknown_property_does_not_match() {
        let q = SupportsQuery::parse("(future-thing)").unwrap();
        // Parses fine (property name stored verbatim)...
        assert!(q.alternatives.len() == 1);
        // ...but does not match (engine doesn't know `future-thing`).
        assert!(!q.matches(&MediaContext::default()));
        assert!(!q.matches(&ctx_truecolor()));
    }

    #[test]
    fn property_value_matches_known_property() {
        // Value ignored in v1; only the property name matters.
        let q = SupportsQuery::parse("(border-style: rounded)").unwrap();
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn not_inverts_condition() {
        let q = SupportsQuery::parse("not (truecolor)").unwrap();
        assert!(q.matches(&ctx_no_truecolor()), "¬truecolor matches when truecolor is off");
        assert!(!q.matches(&ctx_truecolor()), "¬truecolor does NOT match when truecolor is on");
    }

    #[test]
    fn and_matches_only_when_all_hold() {
        // (truecolor) and (border-style)
        let q = SupportsQuery::parse("(truecolor) and (border-style)").unwrap();
        // border-style is always known; truecolor gates.
        assert!(q.matches(&ctx_truecolor()));
        assert!(!q.matches(&ctx_no_truecolor()));
    }

    #[test]
    fn comma_or_matches_either() {
        // (truecolor), (monochrome) — matches a truecolor ctx OR a no-color ctx.
        let q = SupportsQuery::parse("(truecolor), (monochrome)").unwrap();
        assert!(q.matches(&ctx_truecolor()));
        assert!(q.matches(&ctx_no_color()));
        // A ctx that is neither truecolor nor no-color: fails both.
        let plain = MediaContext { truecolor: false, no_color: false, ..Default::default() };
        assert!(!q.matches(&plain));
    }

    #[test]
    fn per_term_negation_matches() {
        // `not (truecolor) and (color)` matches a color ctx WITHOUT truecolor.
        let q = SupportsQuery::parse("not (truecolor) and (color)").unwrap();
        let color_no_tc = MediaContext { truecolor: false, no_color: false, ..Default::default() };
        assert!(q.matches(&color_no_tc));
        // truecolor on → ¬truecolor false → no match.
        let color_tc = MediaContext { truecolor: true, no_color: false, ..Default::default() };
        assert!(!q.matches(&color_tc));
        // no_color → (color) false → no match.
        assert!(!q.matches(&ctx_no_color()));
    }

    // --- Display -------------------------------------------------------------

    #[test]
    fn display_roundtrip_simple() {
        let q = SupportsQuery::parse("(truecolor) and (color)").unwrap();
        assert_eq!(q.to_string(), "(truecolor) and (color)");
    }

    #[test]
    fn display_roundtrip_comma_and_not() {
        let q = SupportsQuery::parse("(truecolor), not (color)").unwrap();
        assert_eq!(q.to_string(), "(truecolor), not (color)");
    }

    #[test]
    fn display_roundtrip_property_value() {
        let q = SupportsQuery::parse("(border-style: rounded)").unwrap();
        assert_eq!(q.to_string(), "(border-style: rounded)");
    }

    #[test]
    fn display_roundtrip_negated_term() {
        let q = SupportsQuery::parse("not (truecolor) and (color)").unwrap();
        assert_eq!(q.to_string(), "not (truecolor) and (color)");
        let q2 = SupportsQuery::parse(&q.to_string()).unwrap();
        assert_eq!(q, q2);
    }
}
