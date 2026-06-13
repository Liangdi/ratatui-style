//! Compile-time CSS embedding via the [`css!`](crate::css) macro.
//!
//! A CSS file is `include_str!`-ed into the binary and lazily parsed into a
//! [`Stylesheet`](crate::Stylesheet) on first access. The embedded rules carry
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

/// Embed a CSS file at compile time and lazily parse it into a
/// [`std::sync::LazyLock<Stylesheet>`](crate::Stylesheet).
///
/// The path is resolved relative to the source file where the macro is invoked
/// (via `include_str!`). The parsed rules are tagged
/// [`Origin::Theme`](crate::Origin::Theme), making them overridable by
/// user-level rules at runtime (see [`RuntimeStyle`](crate::RuntimeStyle)).
///
/// Bind the result to a `static`; dereference (`&*THEME`) for a
/// `&'static Stylesheet`.
///
/// # Panics
///
/// Panics on first access if the embedded CSS fails to parse. Since the CSS is
/// fixed at compile time, a parse error indicates a bug in the CSS file.
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
}
