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
//! `MediaQuery::matches` is true iff **all** of its conditions hold against the
//! supplied context. A query with no conditions matches anything (an empty
//! `@media {}` is a no-op gate).
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

/// One `@media` query: a conjunction (AND) of [`MediaCondition`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaQuery {
    /// All conditions must hold for [`matches`](Self::matches) to be true.
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
    /// True iff **all** conditions hold against `ctx`. An empty query (no
    /// conditions) matches anything.
    pub fn matches(&self, ctx: &MediaContext) -> bool {
        self.conditions.iter().all(|c| c.matches(ctx))
    }

    /// Parse the text BETWEEN `@media` and the block's opening `{`.
    ///
    /// Accepts an optional leading media-type keyword (`screen`, `all`, `only
    /// screen` — accepted and ignored) followed by zero or more parenthesized
    /// conditions joined by `and` (case-insensitive). Each condition is either
    /// `(feature)` or `(feature: value)`.
    ///
    /// Unknown / malformed features surface as a [`CssError`] so strict stays
    /// honest; the stylesheet parser propagates it.
    pub fn parse(s: &str) -> Result<MediaQuery> {
        let mut conditions = Vec::new();

        // Lowercase the whole query once; tokens are case-insensitive. Keep the
        // original slice around for nothing — we only operate on the lowercased
        // copy below.
        let lower = s.to_ascii_lowercase();
        let bytes = lower.as_bytes();
        let mut i = 0usize;

        // Skip an optional leading media-type keyword up to the first `(` (or
        // end). Recognized keywords (`screen`, `all`, `only`) are ignored; any
        // other keyword is also tolerated here, since the meaningful content is
        // the parenthesized conditions. We do NOT honor `not`/`print` — those
        // would invert/exclude matches and are out of scope for v1, so we just
        // skip leading words and parse the conditions.
        while i < bytes.len() {
            // Skip whitespace.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            if bytes[i] == b'(' {
                // Reached the first condition — break to the condition loop.
                break;
            }
            // Consume a leading word (media-type keyword). Skip until whitespace
            // or `(`. We ignore its value for v1.
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'('
            {
                i += 1;
            }
            // Loop continues; next iteration skips whitespace then either finds
            // `(` or another keyword (e.g. the `and` between `screen` and the
            // first `(`), which we also skip.
        }

        // Now parse zero or more `(condition)` joined by `and`.
        loop {
            // Skip whitespace.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            if bytes[i] != b'(' {
                // Expected either end-of-string or a `(`. Anything else is a
                // structural error.
                return Err(CssError::invalid_selector(format!(
                    "media query: expected `(` near `{}`",
                    &s[i.min(s.len())..]
                )));
            }
            // Find the matching `)`.
            let close = match lower[i..].find(')') {
                Some(rel) => i + rel,
                None => {
                    return Err(CssError::invalid_selector(
                        "media query: unbalanced parens (missing `)`)",
                    ));
                }
            };
            // Inner content between the parens (exclusive).
            let inner = &lower[i + 1..close];
            conditions.push(parse_condition(inner)?);
            i = close + 1;

            // After a condition, expect either end-of-string or ` and `.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            // Must be the keyword `and`.
            let rest = &lower[i..];
            let and_len = "and".len();
            if rest.len() >= and_len && &rest[..and_len] == "and" {
                i += and_len;
                continue;
            }
            return Err(CssError::invalid_selector(format!(
                "media query: expected `and` between conditions near `{rest}`"
            )));
        }

        Ok(MediaQuery { conditions })
    }
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
        if self.conditions.is_empty() {
            return write!(f, "all");
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

    // --- parse ---------------------------------------------------------------

    #[test]
    fn parse_min_width() {
        let q = MediaQuery::parse("(min-width: 80)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::MinWidth(80)]);
    }

    #[test]
    fn parse_max_width_and_min_height() {
        let q = MediaQuery::parse("(max-width: 120) and (min-height: 24)").unwrap();
        assert_eq!(
            q.conditions,
            vec![MediaCondition::MaxWidth(120), MediaCondition::MinHeight(24)]
        );
    }

    #[test]
    fn parse_width_exact() {
        let q = MediaQuery::parse("(width: 80)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::Width(80)]);
    }

    #[test]
    fn parse_color_bare() {
        let q = MediaQuery::parse("(color)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::Color]);
    }

    #[test]
    fn parse_monochrome_bare() {
        let q = MediaQuery::parse("(monochrome)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::Monochrome]);
    }

    #[test]
    fn parse_truecolor_bare() {
        let q = MediaQuery::parse("(truecolor)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::Truecolor]);
    }

    #[test]
    fn parse_leading_media_type_ignored() {
        let q = MediaQuery::parse("screen and (min-width: 80)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::MinWidth(80)]);

        let q2 = MediaQuery::parse("all and (max-height: 40)").unwrap();
        assert_eq!(q2.conditions, vec![MediaCondition::MaxHeight(40)]);
    }

    #[test]
    fn parse_empty_query_matches_all() {
        let q = MediaQuery::parse("").unwrap();
        assert!(q.conditions.is_empty());
        assert!(q.matches(&MediaContext::default()));
    }

    #[test]
    fn parse_uppercase_features() {
        // Case-insensitive: features get lowercased internally.
        let q = MediaQuery::parse("(MIN-WIDTH: 80)").unwrap();
        assert_eq!(q.conditions, vec![MediaCondition::MinWidth(80)]);
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

    // --- Display -------------------------------------------------------------

    #[test]
    fn display_roundtrip() {
        let q = MediaQuery::parse("(min-width: 80) and (color)").unwrap();
        assert_eq!(q.to_string(), "(min-width: 80) and (color)");
    }
}
