//! Compile-time CSS embedding via the [`css!`](crate::css) macro.
//!
//! The macro has two forms:
//!
//! - **File form** — `css!("theme.css")`: a CSS file is `include_str!`-ed into
//!   the binary and lazily parsed into a
//!   [`Stylesheet`](crate::Stylesheet). The path is relative to the source file
//!   where the macro is invoked.
//! - **Inline form** — `css!(inline "Button { color: red; }")`: the CSS source
//!   is an inline string literal, with no file involved. Handy for small
//!   embedded themes that don't warrant a separate `.css` artifact.
//!
//! In both forms the parsed rules carry
//! [`Origin::Theme`](crate::Origin::Theme), so they can be overridden at
//! runtime by [`Origin::User`](crate::Origin::User) rules — see
//! [`RuntimeStyle`](crate::RuntimeStyle).
//!
//! The macro expands to a [`std::sync::LazyLock`] value, so it is meant to be
//! bound to a `static`. `LazyLock` derefs to `Stylesheet`, so you call methods
//! directly on the static; and because the data lives in the static, `&*THEME`
//! yields a `&'static Stylesheet` for APIs that need one (such as
//! [`RuntimeStyle::new`](crate::RuntimeStyle::new)).
//!
//! ```rust,ignore
//! use std::sync::LazyLock;
//! use ratatui_style::{css, OwnedNode, Stylesheet};
//!
//! static THEME: LazyLock<Stylesheet> = css!("theme.css");
//! // or, inline:
//! static THEME: LazyLock<Stylesheet> = css!(inline "Button { color: red; }");
//!
//! fn main() {
//!     let node = OwnedNode::new("Button").with_classes(["primary"]);
//!     let computed = THEME.compute(&node, None);   // auto-deref through LazyLock
//! }
//! ```
//!
//! ## SCSS / Sass
//!
//! This crate deliberately does **not** depend on a Sass compiler. To embed
//! `.scss`, compile it to CSS in your own `build.rs` and point `css!` at the
//! generated file:
//!
//! ```rust,ignore
//! // Cargo.toml:  [build-dependencies] grass = "0.13"
//! // build.rs:
//! //   let css = grass::from_path("styles/theme.scss", &Default::default())?;
//! //   std::fs::write(format!("{}/theme.css", OUT_DIR), css)?;
//!
//! use std::sync::LazyLock;
//! use ratatui_style::{css, Stylesheet};
//!
//! static THEME: LazyLock<Stylesheet> = css!(concat!(env!("OUT_DIR"), "/theme.css"));
//! ```

/// Embed CSS at compile time and lazily parse it into a
/// [`std::sync::LazyLock<Stylesheet>`](crate::Stylesheet).
///
/// Two forms:
///
/// - **File** — `css!("theme.css")`: the path is resolved relative to the
///   source file where the macro is invoked (via `include_str!`).
/// - **Inline** — `css!(inline "Button { color: red; }")`: the CSS source is an
///   inline string literal, with no file involved.
///
/// In both forms the parsed rules are tagged
/// [`Origin::Theme`](crate::Origin::Theme), making them overridable by
/// user-level rules at runtime (see [`RuntimeStyle`](crate::RuntimeStyle)).
///
/// Bind the result to a `static`; dereference (`&*THEME`) for a
/// `&'static Stylesheet`.
///
/// # Panics
///
/// Panics on first access if the CSS fails to parse. Since the CSS is fixed at
/// compile time, a parse error indicates a bug in the CSS source.
#[macro_export]
macro_rules! css {
    ($path:literal) => {
        std::sync::LazyLock::new(|| {
            $crate::Stylesheet::parse_with_origin(
                include_str!($path),
                $crate::Origin::Theme,
            )
            .expect(concat!("embedded CSS parse failed: ", $path))
        })
    };
    (inline $body:literal) => {
        std::sync::LazyLock::new(|| {
            $crate::Stylesheet::parse_with_origin($body, $crate::Origin::Theme)
                .expect("embedded inline CSS parse failed")
        })
    };
}

#[cfg(test)]
mod tests {
    use crate::node::OwnedNode;
    use crate::stylesheet::Origin;
    use crate::{Color, Stylesheet};
    use ratatui::style::Color as RC;
    use std::sync::LazyLock;

    // The crate-internal path to a #[macro_export] macro is `crate::css`. This
    // binds the inline form to a static, mirroring real downstream usage.
    static INLINE_THEME: LazyLock<Stylesheet> = crate::css!(inline "Button { color: red; }");

    #[test]
    fn inline_macro_produces_computable_stylesheet() {
        let node = OwnedNode::new("Button");
        let computed = INLINE_THEME.compute(&node, None);
        assert_eq!(computed.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn inline_macro_tags_origin_theme() {
        // The inline form, like the file form, must tag rules Origin::Theme so
        // they are overridable by User rules at runtime.
        let mut sheet = Stylesheet::new();
        // User rule should override the Theme rule from the inline stylesheet.
        sheet.add("Button", crate::style::CssStyle::new().color(RC::Blue), Origin::User).unwrap();
        // Build a runtime view: theme (inline) then user rule on top.
        // Simplest check: the inline stylesheet's single rule is Theme origin.
        let rules = INLINE_THEME.rules();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].origin, Origin::Theme);
        assert_eq!(rules[0].selector.type_name.as_deref(), Some("Button"));
    }
}
