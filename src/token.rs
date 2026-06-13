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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeTokens {
    vars: HashMap<String, Token>,
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

    /// Look up a variable by name (without `--`).
    pub fn get(&self, name: &str) -> Option<&Token> {
        self.vars.get(name)
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

    /// Merge another token set into this one; `other` wins on conflict.
    pub fn merge(&mut self, other: &ThemeTokens) {
        for (k, v) in &other.vars {
            self.vars.insert(k.clone(), v.clone());
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
pub fn resolve_strict(color: &Color, tokens: &ThemeTokens) -> Result<RColor> {
    resolve_inner(color, tokens, 0)
}

/// Lenient variant used by the cascade: unresolved variables degrade to
/// `Reset` rather than failing the whole render.
pub fn resolve(color: &Color, tokens: &ThemeTokens) -> RColor {
    resolve_strict(color, tokens).unwrap_or(RColor::Reset)
}

fn resolve_inner(color: &Color, tokens: &ThemeTokens, depth: u8) -> Result<RColor> {
    if depth > 32 {
        return Err(CssError::circular_variable(
            "var() reference chain too deep (depth > 32)",
        ));
    }
    match color {
        Color::Literal(c) => Ok(*c),
        Color::Reset => Ok(RColor::Reset),
        Color::Inherit => Ok(RColor::Reset),
        Color::Var { name, fallback } => match tokens.get_color(name) {
            Some(referent) => resolve_inner(referent, tokens, depth + 1),
            None => match fallback {
                Some(fb) => resolve_inner(fb, tokens, depth + 1),
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
pub fn resolve_length_strict(length: &Length, tokens: &ThemeTokens) -> Result<Length> {
    resolve_length_inner(length, tokens, 0)
}

/// Lenient variant used by the cascade: unresolved/mistyped length variables
/// degrade to [`Length::Auto`] rather than failing the whole render.
pub fn resolve_length(length: &Length, tokens: &ThemeTokens) -> Length {
    resolve_length_strict(length, tokens).unwrap_or(Length::Auto)
}

fn resolve_length_inner(length: &Length, tokens: &ThemeTokens, depth: u8) -> Result<Length> {
    if depth > 32 {
        return Err(CssError::circular_variable(
            "var() reference chain too deep (depth > 32)",
        ));
    }
    match length {
        Length::Auto | Length::Cells(_) | Length::Percent(_) | Length::Min(_) | Length::Max(_) => {
            Ok(length.clone())
        }
        Length::Var { name, fallback } => match tokens.get_length(name) {
            Some(referent) => resolve_length_inner(referent, tokens, depth + 1),
            None => match fallback {
                Some(fb) => resolve_length_inner(fb, tokens, depth + 1),
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
}
