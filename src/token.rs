//! CSS custom properties — the `var()` resolution target.
//!
//! A [`ThemeTokens`] table maps variable names (without the `--` prefix) to
//! [`Token`] values. `var(--name)` references in a [`crate::style::CssStyle`]
//! are resolved against this table during the cascade (see `cascade.rs`).

use std::collections::HashMap;

use ratatui::style::Color as RColor;

use crate::box_model::Length;
use crate::color::Color;
use crate::error::{CssError, Result};
use crate::media::{MediaContext, MediaQuery};

/// A CSS custom-property value. Currently supports [`Color`] and [`Length`]
/// (the latter for `width`/`height`). The color fields — `color`,
/// `background`, `underline-color`, and the `color` nested inside a `border`
/// spec — are all `var()`-driven and resolved during the cascade. By contrast
/// `padding`/`margin` and a border's *style*/*edges* cannot yet be driven by
/// `var()` (their `BoxEdges`/`BorderStyle`/`Borders` representations don't
/// carry a `Var` variant).
///
/// [`Token::Var`] covers the case where a custom property is itself a bare
/// `var(--other)` reference: its ultimate type (color vs length) is not knowable
/// at parse time, so it is stored untyped and resolved by following the chain
/// via [`ThemeTokens::get_color`] / [`ThemeTokens::get_length`].
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Color(Color),
    Length(Length),
    /// A bare `var(--other)` reference whose type is determined by what `--other`
    /// ultimately resolves to. Both `get_color` and `get_length` follow the chain.
    Var { name: String },
}

impl From<Color> for Token {
    fn from(c: Color) -> Self {
        Token::Color(c)
    }
}

impl From<Length> for Token {
    fn from(l: Length) -> Self {
        Token::Length(l)
    }
}

/// Parse a CSS string into a `Token`. Mirrors [`Color::from(&str)`]: a valid
/// color expression becomes `Token::Color`; anything else (including a valid
/// length like `"50%"`) degrades to a reset color. This keeps the ergonomic
/// `tokens_mut().insert("accent", "#00d4ff")` form working for the common
/// color case; for a length token, pass a [`Length`] explicitly.
impl From<&str> for Token {
    fn from(s: &str) -> Self {
        Token::Color(Color::from(s))
    }
}

impl From<String> for Token {
    fn from(s: String) -> Self {
        Token::from(s.as_str())
    }
}

/// A map of CSS custom-property names to [`Token`] values.
///
/// The default (media-agnostic) map lives in `vars`. Media-gated overrides —
/// declared via `:root { --x: … }` *inside* an `@media` block — live in
/// `media_vars`, an ordered list of `(query, map)` pairs in source order. The
/// media-aware getters ([`get_color_with`](Self::get_color_with) /
/// [`get_length_with`](Self::get_length_with)) consult `media_vars` and pick
/// the **most specific** matching query that binds `name` — specificity being
/// the number of conditions in the matching alternative (a 2-condition query
/// beats a 1-condition one), with ties broken by source order (later wins).
/// If no override matches/binds, the default `vars` is the fallback.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeTokens {
    vars: HashMap<String, Token>,
    media_vars: Vec<(MediaQuery, HashMap<String, Token>)>,
}

impl ThemeTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert/overwrite a variable. `name` is given without the `--` prefix.
    pub fn set<T: Into<Token>>(mut self, name: impl Into<String>, value: T) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    /// Insert/overwrite a variable (mutable).
    pub fn insert<T: Into<Token>>(&mut self, name: impl Into<String>, value: T) {
        self.vars.insert(name.into(), value.into());
    }

    /// Insert a media-gated override. `query` is the enclosing `@media` query;
    /// `name` is given without the `--` prefix. If an entry for an equal `query`
    /// already exists in `media_vars`, the name is inserted into that entry's
    /// map (same-name overwrites); otherwise a new `(query, map)` entry is
    /// appended, preserving source order.
    pub fn insert_media<T: Into<Token>>(
        &mut self,
        query: MediaQuery,
        name: impl Into<String>,
        value: T,
    ) {
        // Find an existing entry with the same query (by equality) and accumulate
        // into it; otherwise append a fresh entry. Equality on `MediaQuery` is
        // structural, so two textually-identical queries collapse to one map.
        let key = name.into();
        for (q, map) in &mut self.media_vars {
            if q == &query {
                map.insert(key.clone(), value.into());
                return;
            }
        }
        let mut map = HashMap::new();
        map.insert(key, value.into());
        self.media_vars.push((query, map));
    }

    /// Builder form of [`insert_media`](Self::insert_media).
    pub fn set_media<T: Into<Token>>(
        mut self,
        query: MediaQuery,
        name: impl Into<String>,
        value: T,
    ) -> Self {
        self.insert_media(query, name, value);
        self
    }

    /// Look up a variable by name (without `--`), default map only.
    pub fn get(&self, name: &str) -> Option<&Token> {
        self.vars.get(name)
    }

    /// True iff `name` is defined in the default map OR in any media-gated
    /// override. Used by strict-mode parsing: a `var(--name)` is "defined" if
    /// it is declared *anywhere*, even inside an `@media` block.
    pub fn is_defined(&self, name: &str) -> bool {
        if self.vars.contains_key(name) {
            return true;
        }
        self.media_vars
            .iter()
            .any(|(_, map)| map.contains_key(name))
    }

    /// Convenience: look up a variable as a [`Color`], if it holds one.
    ///
    /// Follows [`Token::Var`] chains: a `--a: var(--b)` reference resolves to
    /// whatever `--b` (transitively) is, and the result is returned only if the
    /// terminal value is a color.
    pub fn get_color(&self, name: &str) -> Option<&Color> {
        let mut cur = name;
        for _ in 0..32 {
            match self.vars.get(cur)? {
                Token::Color(c) => return Some(c),
                Token::Var { name: next } => cur = next,
                // A length (or anything else) is not a color.
                _ => return None,
            }
        }
        None
    }

    /// Convenience: look up a variable as a [`Length`], if it holds one.
    ///
    /// Follows [`Token::Var`] chains like [`get_color`](Self::get_color).
    pub fn get_length(&self, name: &str) -> Option<&Length> {
        let mut cur = name;
        for _ in 0..32 {
            match self.vars.get(cur)? {
                Token::Length(l) => return Some(l),
                Token::Var { name: next } => cur = next,
                // A color (or anything else) is not a length.
                _ => return None,
            }
        }
        None
    }

    /// Media-aware color lookup. Among all `media_vars` entries whose query
    /// [`MediaQuery::matches`] `media` **and** whose map binds `name` (following
    /// [`Token::Var`] chains, themselves resolved media-aware), the **most
    /// specific** one wins — specificity is the number of conditions in the
    /// matching alternative (e.g. `(min-width:80) and (color)` at 2 conditions
    /// beats `(min-width:80)` at 1), with ties broken by source order (the
    /// later entry wins). If no override matches/binds, falls back to the
    /// default [`get_color`](Self::get_color). Returns an **owned** [`Color`]
    /// because the resolved value may live in any one of several maps and there
    /// is no single stable borrow.
    pub fn get_color_with(&self, name: &str, media: &MediaContext) -> Option<Color> {
        self.resolve_color_with(name, media, 0)
    }

    /// Media-aware length lookup — analogous to [`get_color_with`](Self::get_color_with).
    /// Most-specific matching query wins (ties → source order). Returns an
    /// owned [`Length`].
    pub fn get_length_with(&self, name: &str, media: &MediaContext) -> Option<Length> {
        self.resolve_length_with(name, media, 0)
    }

    /// Recursive, depth-capped, cycle-guarded color resolver for the
    /// media-aware path. Among all matching media overrides, the most specific
    /// one that binds `name` (to a color, or to a `Var` chain) wins; ties go to
    /// the later source-order entry. Falls back to `vars` for the default.
    /// `Token::Var` chains are followed recursively through the same media
    /// context.
    fn resolve_color_with(&self, name: &str, media: &MediaContext, depth: u8) -> Option<Color> {
        if depth > 32 {
            return None;
        }
        // Pick the most-specific matching media override that binds `name` to a
        // color-compatible token (a `Color` or a `Var` chain — a `Length` is a
        // type mismatch and does not count as binding for the color path).
        if let Some(map) = self.best_media_map(name, media, /* color_path */ true) {
            // SAFETY-free: best_media_map only returns Some(map) when map[name]
            // exists and is Color-or-Var; re-fetch the token.
            match map.get(name).expect("best_media_map guarantees map[name] present") {
                Token::Color(c) => return Some(c.clone()),
                Token::Var { name: next } => {
                    return self.resolve_color_with(next, media, depth + 1);
                }
                // Unreachable: best_media_map rejects Length on the color path.
                Token::Length(_) => return None,
            }
        }
        // Default fallback.
        match self.vars.get(name)? {
            Token::Color(c) => Some(c.clone()),
            Token::Var { name: next } => self.resolve_color_with(next, media, depth + 1),
            Token::Length(_) => None,
        }
    }

    /// Recursive, depth-capped length resolver for the media-aware path.
    /// Mirrors [`resolve_color_with`](Self::resolve_color_with): most-specific
    /// matching override wins, ties → source order.
    fn resolve_length_with(&self, name: &str, media: &MediaContext, depth: u8) -> Option<Length> {
        if depth > 32 {
            return None;
        }
        if let Some(map) = self.best_media_map(name, media, /* color_path */ false) {
            match map.get(name).expect("best_media_map guarantees map[name] present") {
                Token::Length(l) => return Some(l.clone()),
                Token::Var { name: next } => {
                    return self.resolve_length_with(next, media, depth + 1);
                }
                // Unreachable: best_media_map rejects Color on the length path.
                Token::Color(_) => return None,
            }
        }
        match self.vars.get(name)? {
            Token::Length(l) => Some(l.clone()),
            Token::Var { name: next } => self.resolve_length_with(next, media, depth + 1),
            Token::Color(_) => None,
        }
    }

    /// Find the map of the **most specific** matching media override that binds
    /// `name` to a token usable on the requested path.
    ///
    /// - `color_path == true`: a binding counts only if it is a `Color` or a
    ///   `Var` (a `Length` binding is a type mismatch and is skipped).
    /// - `color_path == false`: symmetric — `Length` or `Var` counts, `Color`
    ///   is skipped.
    ///
    /// Ranking: among entries whose query [`MediaQuery::matches`] `media` AND
    /// whose map binds `name` to a path-compatible token, the winner is the one
    /// with the highest [`MediaQuery::matching_specificity`] (most conditions in
    /// the matching alternative). Ties are broken by **source order**: since we
    /// scan `media_vars` front-to-back and replace the current winner whenever
    /// the new entry's specificity is `>=` (equal-or-greater), a later
    /// equal-specificity entry takes precedence (later wins).
    ///
    /// Returns `None` if no matching override binds `name` compatibly.
    fn best_media_map(
        &self,
        name: &str,
        media: &MediaContext,
        color_path: bool,
    ) -> Option<&HashMap<String, Token>> {
        let mut best: Option<(&HashMap<String, Token>, usize)> = None;
        for (query, map) in &self.media_vars {
            // The query must match under the active context; skip if not.
            let spec = match query.matching_specificity(media) {
                Some(s) => s,
                None => continue,
            };
            // ...and bind name to a path-compatible token.
            let tok = match map.get(name) {
                Some(t) => t,
                None => continue,
            };
            let compatible = match tok {
                Token::Var { .. } => true,
                Token::Color(_) => color_path,
                Token::Length(_) => !color_path,
            };
            if !compatible {
                continue;
            }
            // Keep the entry with the highest specificity; on a tie, the LATER
            // source-order entry wins. Since we iterate front-to-back, replace
            // the current winner whenever `spec >= cur_spec` (equal → later
            // entry wins, strictly greater → more specific wins).
            match best {
                Some((_, cur_spec)) if cur_spec > spec => {}
                _ => best = Some((map, spec)),
            }
        }
        best.map(|(map, _)| map)
    }

    /// Merge another token set into this one; `other` wins on conflict (both
    /// the default map and the media-gated overrides, the latter appended in
    /// source order so other's overrides come later / win).
    pub fn merge(&mut self, other: &ThemeTokens) {
        for (k, v) in &other.vars {
            self.vars.insert(k.clone(), v.clone());
        }
        for (q, map) in &other.media_vars {
            self.media_vars.push((q.clone(), map.clone()));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    pub fn len(&self) -> usize {
        self.vars.len()
    }
}

/// Resolve a `var()` reference chain to a concrete ratatui color.
///
/// - `Literal` / `Reset` map straight through.
/// - `Var` is looked up in `tokens`; if absent, the `var()` fallback is used;
///   if there is no fallback, returns [`CssError::UndefinedVariable`].
/// - `Inherit` resolves to `Reset` (it should have been folded in by the
///   inheritance pass already).
/// - Cycles / chains deeper than 32 return [`CssError::CircularVariable`].
///
/// This is the default-media wrapper: it calls
/// [`resolve_strict_with_media`] with [`MediaContext::default`], so media-gated
/// overrides do NOT participate (the default map is still consulted). Use the
/// `_with_media` variant to gate overrides against a terminal context.
pub fn resolve_strict(color: &Color, tokens: &ThemeTokens) -> Result<RColor> {
    resolve_strict_with_media(color, tokens, &MediaContext::default())
}

/// Lenient variant used by the cascade: unresolved variables degrade to
/// `Reset` rather than failing the whole render. Default-media wrapper around
/// [`resolve_with_media`].
pub fn resolve(color: &Color, tokens: &ThemeTokens) -> RColor {
    resolve_with_media(color, tokens, &MediaContext::default())
}

/// Media-aware strict resolution: like [`resolve_strict`] but the `var()` chain
/// is resolved via [`ThemeTokens::get_color_with`] against `media`, so
/// `@media`-gated token overrides participate when their query matches.
pub fn resolve_strict_with_media(
    color: &Color,
    tokens: &ThemeTokens,
    media: &MediaContext,
) -> Result<RColor> {
    resolve_inner(color, tokens, media, 0)
}

/// Media-aware lenient resolution: like [`resolve`] but consults media-gated
/// overrides. Unresolved variables degrade to `Reset`.
pub fn resolve_with_media(color: &Color, tokens: &ThemeTokens, media: &MediaContext) -> RColor {
    resolve_strict_with_media(color, tokens, media).unwrap_or(RColor::Reset)
}

fn resolve_inner(
    color: &Color,
    tokens: &ThemeTokens,
    media: &MediaContext,
    depth: u8,
) -> Result<RColor> {
    if depth > 32 {
        return Err(CssError::circular_variable(
            "var() reference chain too deep (depth > 32)",
        ));
    }
    match color {
        Color::Literal(c) => Ok(*c),
        Color::Reset => Ok(RColor::Reset),
        Color::Inherit => Ok(RColor::Reset),
        Color::Var { name, fallback } => match tokens.get_color_with(name, media) {
            Some(referent) => resolve_inner(&referent, tokens, media, depth + 1),
            None => match fallback {
                Some(fb) => resolve_inner(fb, tokens, media, depth + 1),
                None => Err(CssError::undefined_variable(name.clone())),
            },
        },
    }
}

/// Resolve a `var()` reference chain to a concrete [`Length`].
///
/// Mirrors [`resolve_inner`] (and its lenient wrapper, [`resolve`]) but for the
/// [`Length`] path. The lenient semantics are identical: a missing variable, a
/// type mismatch (e.g. a name bound to a `Color`), or a too-deep chain all
/// degrade to [`Length::Auto`] rather than failing the whole render. The strict
/// form surfaces the error instead.
///
/// Default-media wrapper around [`resolve_length_strict_with_media`].
pub fn resolve_length_strict(length: &Length, tokens: &ThemeTokens) -> Result<Length> {
    resolve_length_strict_with_media(length, tokens, &MediaContext::default())
}

/// Lenient variant used by the cascade: unresolved/mistyped length variables
/// degrade to [`Length::Auto`] rather than failing the whole render.
/// Default-media wrapper around [`resolve_length_with_media`].
pub fn resolve_length(length: &Length, tokens: &ThemeTokens) -> Length {
    resolve_length_with_media(length, tokens, &MediaContext::default())
}

/// Media-aware strict length resolution: like [`resolve_length_strict`] but the
/// `var()` chain is resolved via [`ThemeTokens::get_length_with`] against
/// `media`.
pub fn resolve_length_strict_with_media(
    length: &Length,
    tokens: &ThemeTokens,
    media: &MediaContext,
) -> Result<Length> {
    resolve_length_inner(length, tokens, media, 0)
}

/// Media-aware lenient length resolution: like [`resolve_length`] but consults
/// media-gated overrides.
pub fn resolve_length_with_media(
    length: &Length,
    tokens: &ThemeTokens,
    media: &MediaContext,
) -> Length {
    resolve_length_strict_with_media(length, tokens, media).unwrap_or(Length::Auto)
}

fn resolve_length_inner(
    length: &Length,
    tokens: &ThemeTokens,
    media: &MediaContext,
    depth: u8,
) -> Result<Length> {
    if depth > 32 {
        return Err(CssError::circular_variable(
            "var() reference chain too deep (depth > 32)",
        ));
    }
    match length {
        Length::Auto | Length::Cells(_) | Length::Percent(_) | Length::Min(_) | Length::Max(_) => {
            Ok(length.clone())
        }
        Length::Var { name, fallback } => match tokens.get_length_with(name, media) {
            Some(referent) => resolve_length_inner(&referent, tokens, media, depth + 1),
            None => match fallback {
                Some(fb) => resolve_length_inner(fb, tokens, media, depth + 1),
                None => Err(CssError::undefined_variable(name.clone())),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_simple_var() {
        let tokens = ThemeTokens::new().set("accent", Color::literal(RColor::Blue));
        let c = Color::var("accent");
        assert_eq!(resolve_strict(&c, &tokens).unwrap(), RColor::Blue);
    }

    #[test]
    fn resolves_chain() {
        let tokens = ThemeTokens::new()
            .set("accent", Color::var("blue"))
            .set("blue", Color::literal(RColor::Blue));
        assert_eq!(resolve_strict(&Color::var("accent"), &tokens).unwrap(), RColor::Blue);
    }

    #[test]
    fn uses_fallback() {
        let tokens = ThemeTokens::new();
        let c = Color::Var { name: "missing".into(), fallback: Some(Box::new(Color::literal(RColor::Red))) };
        assert_eq!(resolve_strict(&c, &tokens).unwrap(), RColor::Red);
    }

    #[test]
    fn undefined_is_error_strict_but_reset_lenient() {
        let tokens = ThemeTokens::new();
        assert!(resolve_strict(&Color::var("nope"), &tokens).is_err());
        assert_eq!(resolve(&Color::var("nope"), &tokens), RColor::Reset);
    }

    #[test]
    fn token_table_holds_length() {
        let tokens = ThemeTokens::new().set("w", Length::Cells(22));
        assert_eq!(tokens.get_length("w"), Some(&Length::Cells(22)));
        // A length slot is not a color slot.
        assert_eq!(tokens.get_color("w"), None);
        // And vice versa.
        let tokens = ThemeTokens::new().set("c", Color::literal(RColor::Blue));
        assert_eq!(tokens.get_color("c"), Some(&Color::literal(RColor::Blue)));
        assert_eq!(tokens.get_length("c"), None);
    }

    #[test]
    fn length_var_resolves_strict() {
        let tokens = ThemeTokens::new().set("w", Length::Cells(22));
        assert_eq!(
            resolve_length_strict(&Length::Var { name: "w".into(), fallback: None }, &tokens).unwrap(),
            Length::Cells(22)
        );
    }

    #[test]
    fn length_var_chain() {
        let tokens = ThemeTokens::new()
            .set("w", Length::Var { name: "w2".into(), fallback: None })
            .set("w2", Length::Cells(10));
        assert_eq!(
            resolve_length_strict(&Length::Var { name: "w".into(), fallback: None }, &tokens).unwrap(),
            Length::Cells(10)
        );
    }

    #[test]
    fn length_var_undefined_degrades_to_auto_lenient() {
        let tokens = ThemeTokens::new();
        assert!(resolve_length_strict(&Length::Var { name: "nope".into(), fallback: None }, &tokens).is_err());
        assert_eq!(
            resolve_length(&Length::Var { name: "nope".into(), fallback: None }, &tokens),
            Length::Auto
        );
    }

    #[test]
    fn length_var_mistype_degrades_to_auto_lenient() {
        // A name bound to a Color is a type mismatch on the length path.
        let tokens = ThemeTokens::new().set("c", Color::literal(RColor::Blue));
        assert_eq!(
            resolve_length(&Length::Var { name: "c".into(), fallback: None }, &tokens),
            Length::Auto
        );
    }

    #[test]
    fn length_var_undefined_uses_fallback() {
        // An undefined name WITH a fallback resolves to the fallback
        // (recursively), mirroring the color var() path.
        let tokens = ThemeTokens::new();
        let l = Length::Var {
            name: "missing".into(),
            fallback: Some(Box::new(Length::Cells(7))),
        };
        assert_eq!(resolve_length_strict(&l, &tokens).unwrap(), Length::Cells(7));
        assert_eq!(resolve_length(&l, &tokens), Length::Cells(7));
    }

    // ---------------------------------------------------------------------
    // Media-gated overrides (P4-3)
    // ---------------------------------------------------------------------

    fn mq(s: &str) -> MediaQuery {
        MediaQuery::parse(s).unwrap()
    }
    fn ctx(cols: u16) -> MediaContext {
        MediaContext {
            cols,
            ..Default::default()
        }
    }

    #[test]
    fn get_color_with_uses_media_override_when_matching() {
        let tokens = ThemeTokens::new()
            .set("accent", Color::literal(RColor::Red))
            .set_media(
                mq("(min-width: 80)"),
                "accent",
                Color::literal(RColor::Blue),
            );
        // Matching context → override (blue).
        assert_eq!(
            tokens.get_color_with("accent", &ctx(100)),
            Some(Color::literal(RColor::Blue))
        );
        // Non-matching context → default (red).
        assert_eq!(
            tokens.get_color_with("accent", &ctx(60)),
            Some(Color::literal(RColor::Red))
        );
        // Default-only getter still returns the default (red), unaffected.
        assert_eq!(
            tokens.get_color("accent"),
            Some(&Color::literal(RColor::Red))
        );
    }

    #[test]
    fn get_color_with_falls_back_when_override_is_for_a_different_name() {
        // A media override for --other should not affect --accent.
        let tokens = ThemeTokens::new()
            .set("accent", Color::literal(RColor::Red))
            .set_media(
                mq("(min-width: 80)"),
                "other",
                Color::literal(RColor::Green),
            );
        assert_eq!(
            tokens.get_color_with("accent", &ctx(100)),
            Some(Color::literal(RColor::Red)),
            "override for --other must not shadow --accent"
        );
    }

    #[test]
    fn get_color_with_last_matching_override_wins() {
        // Two queries both match ctx{cols:100}; the later source-order entry wins.
        let tokens = ThemeTokens::new()
            .set("accent", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 50)"), "accent", Color::literal(RColor::Green))
            .set_media(mq("(min-width: 80)"), "accent", Color::literal(RColor::Blue));
        assert_eq!(
            tokens.get_color_with("accent", &ctx(100)),
            Some(Color::literal(RColor::Blue)),
            "last-matching media override wins by source order"
        );
        // At cols:60 only the first query matches → green.
        assert_eq!(
            tokens.get_color_with("accent", &ctx(60)),
            Some(Color::literal(RColor::Green))
        );
    }

    #[test]
    fn get_color_with_chains_through_media_var() {
        // --x: var(--y), both media-gated under the matching ctx.
        let tokens = ThemeTokens::new().set_media(
            mq("(min-width: 80)"),
            "x",
            Token::Var { name: "y".to_string() },
        );
        let tokens = tokens.set_media(
            mq("(min-width: 80)"),
            "y",
            Color::literal(RColor::Magenta),
        );
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Magenta)),
            "media-gated var() chain resolves through both media entries"
        );
        // Non-matching context → none (no default for x/y).
        assert_eq!(tokens.get_color_with("x", &ctx(40)), None);
    }

    #[test]
    fn get_length_with_uses_media_override() {
        let tokens = ThemeTokens::new()
            .set("w", Length::Cells(5))
            .set_media(mq("(min-width: 80)"), "w", Length::Cells(50));
        assert_eq!(tokens.get_length_with("w", &ctx(100)), Some(Length::Cells(50)));
        assert_eq!(tokens.get_length_with("w", &ctx(40)), Some(Length::Cells(5)));
        // Default-only getter unaffected.
        assert_eq!(tokens.get_length("w"), Some(&Length::Cells(5)));
    }

    #[test]
    fn insert_media_accumulates_same_query_into_one_map() {
        // Two insert_media calls with the same query string accumulate into one
        // map entry (same query reused).
        let q = mq("(min-width: 80)");
        let mut tokens = ThemeTokens::new();
        tokens.insert_media(q.clone(), "a", Color::literal(RColor::Red));
        tokens.insert_media(q.clone(), "b", Color::literal(RColor::Green));
        // Both resolve under the matching ctx.
        assert_eq!(tokens.get_color_with("a", &ctx(100)), Some(Color::literal(RColor::Red)));
        assert_eq!(tokens.get_color_with("b", &ctx(100)), Some(Color::literal(RColor::Green)));
        // Same-name within one query overwrites.
        tokens.insert_media(q, "a", Color::literal(RColor::Blue));
        assert_eq!(tokens.get_color_with("a", &ctx(100)), Some(Color::literal(RColor::Blue)));
    }

    #[test]
    fn is_defined_checks_default_and_all_media_maps() {
        let mut tokens = ThemeTokens::new();
        tokens.insert("default_only", Color::literal(RColor::Red));
        tokens.insert_media(mq("(min-width: 80)"), "media_only", Color::literal(RColor::Red));
        assert!(tokens.is_defined("default_only"));
        assert!(tokens.is_defined("media_only"));
        assert!(!tokens.is_defined("neither"));
    }

    #[test]
    fn resolve_with_media_gates_var_against_context() {
        // End-to-end: resolve() (default media) → default; resolve_with_media()
        // (matching) → override.
        let tokens = ThemeTokens::new()
            .set("accent", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80)"), "accent", Color::literal(RColor::Blue));
        // Default: red (media override not consulted).
        assert_eq!(resolve(&Color::var("accent"), &tokens), RColor::Red);
        // Matching ctx: blue.
        assert_eq!(
            resolve_with_media(&Color::var("accent"), &tokens, &ctx(100)),
            RColor::Blue
        );
        // Non-matching ctx: red (fallback to default).
        assert_eq!(
            resolve_with_media(&Color::var("accent"), &tokens, &ctx(40)),
            RColor::Red
        );
    }

    #[test]
    fn resolve_length_with_media_gates_var_against_context() {
        let tokens = ThemeTokens::new()
            .set("w", Length::Cells(5))
            .set_media(mq("(min-width: 80)"), "w", Length::Cells(50));
        assert_eq!(
            resolve_length_with_media(&Length::Var { name: "w".into(), fallback: None }, &tokens, &ctx(100)),
            Length::Cells(50)
        );
        assert_eq!(
            resolve_length_with_media(&Length::Var { name: "w".into(), fallback: None }, &tokens, &ctx(40)),
            Length::Cells(5)
        );
    }

    #[test]
    fn merge_merges_media_vars_too() {
        let other = ThemeTokens::new()
            .set("a", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80)"), "a", Color::literal(RColor::Blue));
        let mut mine = ThemeTokens::new();
        mine.merge(&other);
        assert_eq!(mine.get_color("a"), Some(&Color::literal(RColor::Red)));
        assert_eq!(mine.get_color_with("a", &ctx(100)), Some(Color::literal(RColor::Blue)));
    }

    // ---------------------------------------------------------------------
    // Media-token specificity cascade (P5-3)
    // ---------------------------------------------------------------------

    #[test]
    fn get_color_with_picks_more_specific_override() {
        // media_vars: [ ((min-width:80), {--x: red}),
        //               ((min-width:80) and (color), {--x: blue}) ].
        // Under ctx matching BOTH (cols:100, color), the 2-condition query is
        // MORE specific → --x resolves to BLUE (not red), even though red is
        // the later/equal-source entry… here red is FIRST. The point: the
        // 2-condition entry wins regardless of position.
        let tokens = ThemeTokens::new()
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80) and (color)"), "x", Color::literal(RColor::Blue));
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Blue)),
            "the 2-condition override is more specific and wins"
        );
    }

    #[test]
    fn get_color_with_specificity_tie_falls_back_to_source_order() {
        // Two matching overrides with EQUAL condition counts (1 each) → later
        // source-order wins (existing behavior preserved for ties).
        let tokens = ThemeTokens::new()
            .set_media(mq("(min-width: 50)"), "x", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Blue));
        // cols:100 matches both (both are 1-condition) → later (blue) wins.
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Blue)),
            "equal specificity → later source-order wins"
        );
    }

    #[test]
    fn get_color_with_less_specific_does_not_override_more_specific() {
        // Reverse the insertion order from the first test: the more-specific
        // (2-condition) override is inserted FIRST. It must STILL win.
        let tokens = ThemeTokens::new()
            .set_media(mq("(min-width: 80) and (color)"), "x", Color::literal(RColor::Blue))
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Red));
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Blue)),
            "more-specific wins regardless of source position"
        );
    }

    #[test]
    fn get_color_with_single_override_unchanged() {
        // Regression: a single matching override behaves exactly as before.
        let tokens = ThemeTokens::new()
            .set("x", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Blue));
        assert_eq!(tokens.get_color_with("x", &ctx(100)), Some(Color::literal(RColor::Blue)));
        // Non-matching → default fallback.
        assert_eq!(tokens.get_color_with("x", &ctx(40)), Some(Color::literal(RColor::Red)));
    }

    #[test]
    fn get_color_with_no_override_falls_back_to_default() {
        // Regression: no media override at all → default vars.
        let tokens = ThemeTokens::new().set("x", Color::literal(RColor::Red));
        assert_eq!(tokens.get_color_with("x", &ctx(100)), Some(Color::literal(RColor::Red)));
        assert_eq!(tokens.get_color_with("x", &ctx(40)), Some(Color::literal(RColor::Red)));
    }

    #[test]
    fn get_color_with_specificity_var_chain() {
        // The winning (more-specific) override binds --x to a var() chain whose
        // target is also in a (less-specific) override. The chain must resolve
        // through the same media context, picking the more-specific --y too.
        let tokens = ThemeTokens::new()
            // Less specific: --x = red (direct), --y = magenta.
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80)"), "y", Color::literal(RColor::Magenta))
            // More specific: --x = var(--y).
            .set_media(mq("(min-width: 80) and (color)"), "x", Token::Var { name: "y".into() });
        // cols:100, color → --x resolves via the 2-condition entry (var --y),
        // and --y resolves via the 1-condition entry → magenta.
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Magenta)),
            "more-specific var() chain resolves through the same media context"
        );
    }

    #[test]
    fn get_length_with_picks_more_specific_override() {
        // Length path mirrors the color path.
        let tokens = ThemeTokens::new()
            .set_media(mq("(min-width: 80)"), "w", Length::Cells(5))
            .set_media(mq("(min-width: 80) and (color)"), "w", Length::Cells(50));
        assert_eq!(
            tokens.get_length_with("w", &ctx(100)),
            Some(Length::Cells(50)),
            "more-specific length override wins"
        );
    }

    #[test]
    fn get_color_with_more_specific_skipped_when_it_binds_different_name() {
        // The more-specific query matches but does NOT bind `x` — only the
        // less-specific one binds `x`. So the less-specific entry wins (the
        // more-specific one is not a candidate for `x`).
        let tokens = ThemeTokens::new()
            .set_media(mq("(min-width: 80)"), "x", Color::literal(RColor::Red))
            .set_media(mq("(min-width: 80) and (color)"), "other", Color::literal(RColor::Blue));
        assert_eq!(
            tokens.get_color_with("x", &ctx(100)),
            Some(Color::literal(RColor::Red)),
            "a more-specific query that does not bind `x` does not shadow `x`"
        );
        // And `other` resolves under the more-specific query.
        assert_eq!(
            tokens.get_color_with("other", &ctx(100)),
            Some(Color::literal(RColor::Blue))
        );
    }
}
