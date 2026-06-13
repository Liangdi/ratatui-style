//! # ratatui-style-presets
//!
//! Ready-to-use CSS themes & utilities for [`ratatui-style`] — the styling
//! layer pre-filled, so third-party apps get a sensible look **out of the
//! box** and can swap looks by changing one stylesheet.
//!
//! Each preset is embedded at compile time and exposed as a `&'static
//! Stylesheet` parsed with [`Origin::Theme`](ratatui_style::Origin::Theme), so
//! a downstream app overrides any of it with its own `Origin::User` rules at
//! equal specificity — no merge plumbing required.
//!
//! ## Themes & swappability
//!
//! Every theme fills the **same** canonical semantic tokens (see
//! [`SEMANTIC_TOKENS`]): `--bg`, `--text`, `--accent`, `--success`, …
//! [`default_theme()`] additionally ships base component classes (`Button`,
//! `Panel`, `Text`, `List`, …) that reference those tokens through `var()`.
//! The `widgets` preset does the same for ratatui widget type names. So:
//!
//! - pick a palette (default / catppuccin / nord / dracula) → restyle everything,
//! - layer `widgets` / `tailwind` on top → styled widgets / atomic utilities,
//! - override anything with your own `Origin::User` rules.
//!
//! ## Feature flags
//!
//! | Feature   | Preset                                            |
//! |-----------|---------------------------------------------------|
//! | _(none)_  | [`default_theme()`] — always available            |
//! | `tailwind`| [`tailwind()`] — atomic utility classes           |
//! | `widgets` | [`widgets()`] — ratatui widget type defaults      |
//! | `catppuccin` | [`catppuccin()`] — Catppuccin palette          |
//! | `nord`    | [`nord()`] — Nord palette                         |
//! | `dracula` | [`dracula()`] — Dracula palette                   |
//!
//! ```toml
//! [dependencies]
//! ratatui-style-presets = { version = "0.1", features = ["widgets", "catppuccin"] }
//! ```
//!
//! ## Combine presets
//!
//! [`merge()`] stacks presets into one owned stylesheet (later ones override
//! earlier at equal specificity), and [`PresetBuilder`] does it fluently:
//!
//! ```no_run
//! use ratatui_style_presets::{merge, Preset, PresetBuilder};
//!
//! // Stack presets into one sheet (later ones override earlier at equal
//! // specificity). Add `Widgets` / `Catppuccin` / … when those features are on:
//! let sheet = merge(&[Preset::Default]);
//!
//! // Same thing, fluently:
//! let sheet = PresetBuilder::new()
//!     .with(Preset::Default)
//!     .build();
//! ```
//!
//! [`ratatui-style`]: https://docs.rs/ratatui-style

#![forbid(unsafe_code)]

use ratatui_style::Stylesheet;

/// The canonical, theme-agnostic semantic color-token names every shipped
/// theme reproduces. A theme that sets exactly these tokens can be dropped in
/// as the base stylesheet and the rest of a UI restyles for free.
///
/// `var()` currently supports **color** tokens (not length/padding), so this
/// vocabulary is intentionally color-only.
pub const SEMANTIC_TOKENS: &[&str] = &[
    "--bg",
    "--surface",
    "--surface-2",
    "--border",
    "--text",
    "--text-muted",
    "--accent",
    "--accent-fg",
    "--success",
    "--warning",
    "--danger",
    "--info",
];

/// Parse an embedded preset CSS file once, lazily, as `Origin::Theme`.
///
/// Uses absolute paths so it expands correctly inside any `cfg`-gated module.
macro_rules! preset_static {
    ($name:ident, $path:literal) => {
        static $name: ::std::sync::LazyLock<::ratatui_style::Stylesheet> =
            ::std::sync::LazyLock::new(|| {
                ::ratatui_style::Stylesheet::parse_with_origin(
                    ::std::include_str!($path),
                    ::ratatui_style::Origin::Theme,
                )
                .expect(::std::concat!("embedded preset CSS must parse: ", $path))
            });
    };
}

preset_static!(DEFAULT, "../css/default.css");

/// The default theme + base component classes (`Button`, `Panel`, `Text`,
/// `List`, `Badge`, …). Always available — no feature flag needed.
///
/// Defines the canonical [`SEMANTIC_TOKENS`] that the other themes reproduce.
pub fn default_theme() -> &'static Stylesheet {
    &DEFAULT
}

// ---------------------------------------------------------------------------
// Feature-gated presets. The `static` (and its `include_str!`) are themselves
// `#[cfg]`-gated, so an unused preset is never parsed and its CSS file need
// not exist unless the feature is enabled.
// ---------------------------------------------------------------------------

#[cfg(feature = "tailwind")]
preset_static!(TAILWIND, "../css/tailwind.css");
/// Tailwind-style atomic utility classes (`.bg-*`, `.text-*`, `.p-*`,
/// `.rounded` …). Requires feature `tailwind`.
#[cfg(feature = "tailwind")]
pub fn tailwind() -> &'static Stylesheet {
    &TAILWIND
}

#[cfg(feature = "widgets")]
preset_static!(WIDGETS, "../css/widgets.css");
/// Default styles for ratatui widget type names (`Table`, `List`, `Tabs`,
/// `Gauge`, …), referencing the canonical semantic tokens. Requires feature
/// `widgets`.
#[cfg(feature = "widgets")]
pub fn widgets() -> &'static Stylesheet {
    &WIDGETS
}

#[cfg(feature = "catppuccin")]
preset_static!(CATPPUCCIN, "../css/catppuccin.css");
/// The Catppuccin (Mocha) palette, filling the canonical semantic tokens.
/// Requires feature `catppuccin`.
#[cfg(feature = "catppuccin")]
pub fn catppuccin() -> &'static Stylesheet {
    &CATPPUCCIN
}

#[cfg(feature = "nord")]
preset_static!(NORD, "../css/nord.css");
/// The Nord palette, filling the canonical semantic tokens. Requires feature
/// `nord`.
#[cfg(feature = "nord")]
pub fn nord() -> &'static Stylesheet {
    &NORD
}

#[cfg(feature = "dracula")]
preset_static!(DRACULA, "../css/dracula.css");
/// The Dracula palette, filling the canonical semantic tokens. Requires
/// feature `dracula`.
#[cfg(feature = "dracula")]
pub fn dracula() -> &'static Stylesheet {
    &DRACULA
}

// ---------------------------------------------------------------------------
// Selection + composition API.
// ---------------------------------------------------------------------------

/// A selectable preset stylesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    /// The default theme + base component classes (always available).
    Default,
    /// Tailwind-style atomic utilities.
    #[cfg(feature = "tailwind")]
    Tailwind,
    /// ratatui widget type defaults.
    #[cfg(feature = "widgets")]
    Widgets,
    /// Catppuccin (Mocha) palette.
    #[cfg(feature = "catppuccin")]
    Catppuccin,
    /// Nord palette.
    #[cfg(feature = "nord")]
    Nord,
    /// Dracula palette.
    #[cfg(feature = "dracula")]
    Dracula,
}

impl Preset {
    /// The `&'static Stylesheet` for this preset (parsed as `Origin::Theme`).
    pub fn stylesheet(&self) -> &'static Stylesheet {
        match self {
            Preset::Default => default_theme(),
            #[cfg(feature = "tailwind")]
            Preset::Tailwind => tailwind(),
            #[cfg(feature = "widgets")]
            Preset::Widgets => widgets(),
            #[cfg(feature = "catppuccin")]
            Preset::Catppuccin => catppuccin(),
            #[cfg(feature = "nord")]
            Preset::Nord => nord(),
            #[cfg(feature = "dracula")]
            Preset::Dracula => dracula(),
        }
    }
}

/// Stack several presets into one owned stylesheet, in order.
///
/// Later presets override earlier ones at equal specificity (all rules are
/// `Origin::Theme`). The result is a fresh owned [`Stylesheet`] you can pass to
/// `RuntimeStyle::from_owned` or compute against directly.
///
/// Note: presets can't add a `.merge()` method onto [`Stylesheet`] directly
/// (orphan rule — `Stylesheet` is foreign), so this free function + the
/// [`PresetBuilder`] are the composition entry points.
pub fn merge(presets: &[Preset]) -> Stylesheet {
    let mut sheet = Stylesheet::new();
    for p in presets {
        sheet.extend(p.stylesheet());
    }
    sheet
}

/// A fluent builder over [`merge()`]: stack presets, then [`build()`](Self::build)
/// into one owned stylesheet.
#[derive(Debug, Default)]
pub struct PresetBuilder {
    presets: Vec<Preset>,
}

impl PresetBuilder {
    /// Start an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a preset; it layers on top of whatever was added before.
    #[must_use]
    pub fn with(mut self, preset: Preset) -> Self {
        self.presets.push(preset);
        self
    }

    /// Materialize the stacked presets into one owned stylesheet.
    pub fn build(self) -> Stylesheet {
        merge(&self.presets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_style::{NodeRef, OwnedNode, State};

    #[test]
    fn default_theme_styles_root() {
        // Root pulls --bg as background.
        let c = DEFAULT.compute(&NodeRef::new("Root"), None);
        assert!(
            c.style.background.is_some(),
            "Root should have a background from --bg"
        );
    }

    #[test]
    fn default_theme_styles_primary_button() {
        let node = NodeRef::new("Button").classes(&["primary"]);
        let c = DEFAULT.compute(&node, None);
        // primary sets background to --accent and color to --accent-fg.
        assert!(c.style.background.is_some(), "primary button needs a background");
        assert!(c.style.color.is_some(), "primary button needs a text color");
    }

    #[test]
    fn default_theme_text_variants() {
        for cls in ["title", "success", "warning", "danger", "info", "muted"] {
            // Bind the slice to a local so the NodeRef borrow outlives the call.
            let classes = [cls];
            let node = NodeRef::new("Text").classes(&classes);
            let c = DEFAULT.compute(&node, None);
            assert!(c.style.color.is_some(), "Text.{cls} should set a color");
        }
    }

    #[test]
    fn pseudo_state_disabled_applies() {
        // :disabled should kick in when the disabled state is set.
        let node = NodeRef::new("Button").classes(&["primary"]).state(State {
            disabled: true,
            ..State::empty()
        });
        let c = DEFAULT.compute(&node, None);
        assert!(c.style.color.is_some(), "disabled button should still have a color");
    }

    #[test]
    fn merge_reproduces_a_preset() {
        // Merging just Default should compute identically to the Default sheet.
        let merged = merge(&[Preset::Default]);
        let node = NodeRef::new("Button").classes(&["primary"]);
        let a = DEFAULT.compute(&node, None);
        let b = merged.compute(&node, None);
        assert_eq!(a.style.color, b.style.color);
        assert_eq!(a.style.background, b.style.background);
    }

    #[test]
    fn builder_equivalent_to_merge() {
        let node = NodeRef::new("Root");
        let via_merge = merge(&[Preset::Default]).compute(&node, None);
        let via_build = PresetBuilder::new().with(Preset::Default).build().compute(&node, None);
        assert_eq!(via_merge.style.background, via_build.style.background);
    }

    #[test]
    fn semantic_token_vocabulary_is_stable() {
        // Guard against accidentally renaming the shared tokens — every theme
        // depends on exactly these names.
        assert_eq!(SEMANTIC_TOKENS.len(), 12);
        assert!(SEMANTIC_TOKENS.contains(&"--accent"));
        assert!(SEMANTIC_TOKENS.contains(&"--text-muted"));
    }

    #[cfg(feature = "tailwind")]
    #[test]
    fn tailwind_utilities_match() {
        // Atomic utilities compose: each writes a different CSS field, so all
        // three apply. (OwnedNode lets us use runtime class strings.)
        let node = OwnedNode::new("Div").with_classes(["bg-blue-500", "text-white", "p-1"]);
        let c = TAILWIND.compute(&node, None);
        assert!(c.style.background.is_some(), "bg-blue-500 must set background");
        assert!(c.style.color.is_some(), "text-white must set color");
        assert!(c.style.padding.is_some(), "p-1 must set padding");

        // The .btn component + variant layer.
        let btn = OwnedNode::new("Div").with_classes(["btn", "primary"]);
        assert!(
            TAILWIND.compute(&btn, None).style.background.is_some(),
            ".btn.primary must set background"
        );
    }

    #[cfg(feature = "widgets")]
    #[test]
    fn widget_types_match() {
        // Type-keyed rules must apply — proves the selectors (and var() tokens)
        // aren't silently dropped by the lenient parser.
        let table = WIDGETS.compute(&OwnedNode::new("Table"), None);
        assert!(table.style.border.is_some(), "Table must have a border");

        let gauge = WIDGETS.compute(&OwnedNode::new("Gauge"), None);
        assert!(gauge.style.background.is_some(), "Gauge must set accent background");

        let tab_active = WIDGETS.compute(&OwnedNode::new("Tab").with_classes(["active"]), None);
        assert!(tab_active.style.color.is_some(), "Tab.active must set color");
    }

    /// Each theme fills the SAME canonical `--accent` token, so pairing a theme
    /// with the widget defaults must resolve `Gauge`'s `var(--accent)` background
    /// to that theme's accent color — and the four shipped themes must differ
    /// (they're distinct palettes, not duplicates).
    #[cfg(all(
        feature = "widgets",
        feature = "catppuccin",
        feature = "nord",
        feature = "dracula"
    ))]
    #[test]
    fn themes_resolve_distinct_accents() {
        fn accent(theme: Preset) -> ratatui_style::Color {
            let sheet = merge(&[theme, Preset::Widgets]);
            // Gauge { background: var(--accent); } in widgets.css.
            sheet
                .compute(&OwnedNode::new("Gauge"), None)
                .style
                .background
                .expect("theme must resolve --accent for Gauge")
        }

        let all = [
            accent(Preset::Default),
            accent(Preset::Catppuccin),
            accent(Preset::Nord),
            accent(Preset::Dracula),
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(
                    all[i], all[j],
                    "themes {i} and {j} share an accent color — expected distinct palettes"
                );
            }
        }
    }
}
