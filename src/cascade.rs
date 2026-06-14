//! The cascade engine — turns a [`Stylesheet`] + a [`StyledNode`] into a fully
//! resolved [`ComputedStyle`].
//!
//! Pipeline per element:
//! 1. Collect matching rules.
//! 2. Sort ascending by `(origin, specificity, source_order)`.
//! 3. Fold declarations via [`CssStyle::overlay`] (later = higher priority).
//! 4. Fold explicit `inherit` keywords and auto-inherited properties from the
//!    parent [`ComputedStyle`].
//! 5. Resolve `var()` references against the token table.

use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::Style as RStyle,
    widgets::Block,
};

use crate::box_model::Length;
use crate::cache::{node_signature, ComputeCache};
use crate::color::Color;
use crate::media::MediaContext;
use crate::node::{Classes, StyledNode};
use crate::selector::NodeIdentity;
use crate::style::CssStyle;
use crate::stylesheet::Stylesheet;
use crate::token::{self, ThemeTokens};

/// A fully-resolved style: all `var()`s turned into literals, inheritable
/// properties filled from the parent. Project onto ratatui via the delegate
/// methods.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedStyle {
    pub style: CssStyle,
}

impl ComputedStyle {
    pub fn new(style: CssStyle) -> Self {
        Self { style }
    }
    pub fn to_style(&self) -> RStyle {
        self.style.to_style()
    }
    pub fn to_block(&self) -> Block<'_> {
        self.style.to_block()
    }
    pub fn apply_margin(&self, area: Rect) -> Rect {
        self.style.apply_margin(area)
    }
    pub fn constraints(&self) -> Option<(Constraint, Constraint)> {
        self.style.constraints()
    }
    pub fn alignment(&self) -> Option<Alignment> {
        self.style.alignment()
    }

    /// Layer a single inline declaration on top of the computed style in place.
    ///
    /// Because `Origin::Inline` is applied last in the cascade, an inline
    /// declaration *wins* over every rule regardless of specificity. This
    /// method reproduces that post-compute: it overlays `inline` via
    /// [`CssStyle::overlay`], so any field `Some` in `inline` replaces the
    /// matching computed field.
    ///
    /// Note: colors passed here should already be literals — `compute` has
    /// already resolved `var()` against the token table, and this method does
    /// not re-resolve.
    pub fn apply_inline(&mut self, inline: &CssStyle) {
        self.style.overlay(inline);
    }

    /// Consuming builder form of [`apply_inline`](Self::apply_inline): overlay
    /// an inline declaration and return `self`.
    pub fn with_inline(mut self, inline: &CssStyle) -> Self {
        self.apply_inline(inline);
        self
    }

    /// One-shot box-model projection: resolve the full `margin → block →
    /// block.inner → content style` sequence in a single call.
    ///
    /// Returns `(block, content_style, inner_area)` where:
    /// - `block` is [`ComputedStyle::to_block`] (borders/padding/background, no
    ///   margin — margin is applied to the area first);
    /// - `content_style` is [`ComputedStyle::to_style`] (the foreground
    ///   decoration to apply to the inner widget);
    /// - `inner_area` is the area left for content after margin shrink *and*
    ///   the block's padding/borders, i.e. it equals
    ///   `to_block().inner(apply_margin(area))`.
    ///
    /// This matches the hand-written sequence downstream widgets previously had
    /// to thread themselves:
    ///
    /// ```text
    /// let shrunk = computed.apply_margin(area);
    /// let block  = computed.to_block();
    /// let inner  = block.inner(shrunk);
    /// let style  = computed.to_style();
    /// ```
    ///
    /// Box-model order is `margin (outer) → border → padding → content`, so the
    /// margin shrink happens *outside* the block and `block.inner` only removes
    /// padding/borders — never the margin.
    pub fn layout(&self, area: Rect) -> (Block<'_>, RStyle, Rect) {
        let shrunk = self.apply_margin(area);
        self.layout_with_shrunk(shrunk)
    }

    /// The margin-free half of [`layout`](Self::layout): given an already
    /// margin-shrunk area, build the `(block, content_style, inner)` triple.
    ///
    /// This exists so [`render_computed`] can call `apply_margin` exactly once
    /// and reuse the result for both the block render and the inner-area
    /// computation, instead of calling it twice (once in `render_computed`,
    /// once inside `layout`). The public [`layout`](Self::layout) delegates
    /// here after shrinking, so the two share one code path.
    fn layout_with_shrunk(&self, shrunk: Rect) -> (Block<'_>, RStyle, Rect) {
        let block = self.to_block();
        let inner = block.inner(shrunk);
        let style = self.to_style();
        (block, style, inner)
    }
}

/// Render a computed style's full box model in one shot.
///
/// Resolves `(block, content_style, inner)` via [`ComputedStyle::layout`],
/// renders the `block` into the margin-shrunk area, then renders the widget
/// returned by `make(inner, content_style)` into the block's inner area. The
/// closure receives the inner `Rect` and the foreground `Style` and is expected
/// to apply the style to the widget it builds (this mirrors how most ratatui
/// widgets carry a `.style(...)`).
///
/// Use this to collapse the `margin → block → content` boilerplate into a
/// single call.
///
/// ```rust,ignore
/// use ratatui::widgets::Paragraph;
/// use ratatui_style::{ComputedStyle, render_computed};
///
/// render_computed(frame, &computed, area, |inner, style| {
///     Paragraph::new("hello").style(style)
/// });
/// ```
pub fn render_computed<W, F>(
    frame: &mut ratatui::Frame<'_>,
    computed: &ComputedStyle,
    area: Rect,
    make: F,
) where
    F: FnOnce(Rect, RStyle) -> W,
    W: ratatui::widgets::Widget,
{
    let shrunk = computed.apply_margin(area);
    let (block, style, inner) = computed.layout_with_shrunk(shrunk);
    frame.render_widget(block, shrunk);
    frame.render_widget(make(inner, style), inner);
}

/// A cascade tree-walker: holds a [`Stylesheet`] reference, a reusable
/// [`ComputeScratch`], and a parent [`ComputedStyle`] stack.
///
/// `enter(node)` computes the node's style using the current stack top (if any)
/// as its parent, pushes the result, and returns an owned copy; `leave()` pops
/// it. This lets a downstream component-tree traversal inherit styles
/// automatically without the caller manually threading `Some(&parent)` into
/// every child's `compute` call.
///
/// # Why `enter` returns an owned value
///
/// `enter` returns an *owned* [`ComputedStyle`] rather than `&ComputedStyle`.
/// Returning a borrow would lock `&mut self` for the returned value's lifetime,
/// making it impossible to nest a second `enter` for a child while holding the
/// parent's style. The owned return avoids that entirely — the caller can hold
/// the parent's computed style freely and still call `enter` for children.
///
/// # Pushed clone is stack-only memcpy
///
/// After `compute`, a [`ComputedStyle`] holds only `Literal`/`Reset`/`Copy`
/// fields — every `var()` has been resolved against the token table, so no
/// [`Color::Var`] (the only [`Color`] variant carrying a heap `String` / `Box`)
/// survives. Every other field (`BoxEdges`, `BorderSpec`, `Weight`, `Length`,
/// …) is a fixed-size, stack-resident enum/struct. The `computed.clone()` that
/// backs the internal stack is therefore a plain stack memcpy with no heap
/// allocation, and is cheap to ignore.
///
/// # Example — walking a three-level tree
///
/// ```rust,ignore
/// use ratatui_style::{CascadeContext, OwnedNode, Stylesheet};
///
/// let sheet: Stylesheet = /* … */;
/// let mut ctx = CascadeContext::new(&sheet);
///
/// // Root
/// let root = ctx.enter(&OwnedNode::new("Root"));
/// // …render root…
///
/// // Panel (child of Root)
/// let panel = ctx.enter(&OwnedNode::new("Panel"));
/// // …render panel…
///
/// // Text (child of Panel) — inherits Panel's color automatically
/// let text = ctx.enter(&OwnedNode::new("Text"));
/// // …render text…
/// ctx.leave(); // back to Panel context
///
/// ctx.leave(); // back to Root context
/// ctx.leave(); // done
/// ```
pub struct CascadeContext<'s> {
    sheet: &'s Stylesheet,
    scratch: ComputeScratch,
    stack: Vec<ComputedStyle>,
    /// Snapshot of the selector-relevant fields of each `enter`ed node, kept
    /// ONLY when `sheet.has_combinators()`. Mirrors `stack` 1:1 (pushed in
    /// `enter`, popped in `leave`) so combinator selectors can match against
    /// the ancestor chain. Empty and untouched for combinator-free stylesheets,
    /// so that path stays allocation-free.
    identity_stack: Vec<NodeIdentity>,
    /// Previous-sibling identities keyed by tree depth, kept ONLY when
    /// `sheet.has_combinators()`. `siblings[D]` is the list of prior siblings
    /// of a node at depth `D` (number of ancestors = D), within the current
    /// parent, oldest-first. Cleared at `siblings[D+1]` on `enter` (a node's
    /// children start with no prior siblings) and appended to at `siblings[D]`
    /// on `leave` (the departed node becomes a prior sibling for the next one).
    /// Empty and untouched for combinator-free stylesheets.
    siblings: Vec<Vec<NodeIdentity>>,
    /// The active terminal context used to gate `@media` rules. Defaults to
    /// all-zero / all-false (no media info), in which case media-gated rules
    /// with any condition do NOT match. Set via [`set_media`](Self::set_media)
    /// / [`with_media`](Self::with_media) before `enter`ing nodes whose rules
    /// depend on it.
    media: MediaContext,
    /// Opt-in compute cache. `None` (the default) means no caching: `enter` /
    /// `leave` behave byte-for-byte identically to the uncached baseline. When
    /// `Some`, every `enter` consults the cache and stores the result; the
    /// cache auto-invalidates on stylesheet mutation via the generation check.
    cache: Option<ComputeCache>,
    /// The signature of each `enter`ed node, mirroring `stack` 1:1. Maintained
    /// ONLY when `cache.is_some()` — used as the next child's `parent_sig` so
    /// the ancestor chain is transitively folded into each child's signature.
    sig_stack: Vec<u64>,
}

impl<'s> CascadeContext<'s> {
    /// Build a walker over `sheet` with an empty parent stack and a fresh
    /// reusable scratch buffer.
    pub fn new(sheet: &'s Stylesheet) -> Self {
        Self {
            sheet,
            scratch: ComputeScratch::new(),
            stack: Vec::new(),
            identity_stack: Vec::new(),
            siblings: Vec::new(),
            media: MediaContext::default(),
            cache: None,
            sig_stack: Vec::new(),
        }
    }

    /// Set the active [`MediaContext`] used to gate `@media` rules, returning
    /// `&mut Self` for chaining. Call before `enter`ing nodes whose rules
    /// depend on terminal size / color capability.
    pub fn set_media(&mut self, media: MediaContext) -> &mut Self {
        self.media = media;
        self
    }

    /// Consuming builder form of [`set_media`](Self::set_media).
    pub fn with_media(mut self, media: MediaContext) -> Self {
        self.media = media;
        self
    }

    /// Attach an opt-in [`ComputeCache`] with the given hard capacity, enabling
    /// memoization across `enter` calls. `capacity == 0` attaches a cache that
    /// never stores (effectively disabled, but `cache` is `Some`); the typical
    /// choice is a small bound sized to the tree's working set.
    ///
    /// Once attached, every `enter` consults the cache before computing and
    /// stores the result on a miss. The cache auto-invalidates on stylesheet
    /// mutation: a `sheet.add(...)` / `tokens_mut()` / etc. bumps the sheet's
    /// generation, which the cache detects on its next access and clears.
    ///
    /// **Combinator handling**: when the stylesheet `has_combinators()`, the
    /// cached path uses the ancestors-aware compute ([combinators match]).
    /// When it does not, the cheaper one-shot path is used. Both paths share
    /// the same cache, so caching works regardless.
    ///
    /// [combinators match]: Stylesheet::compute_cached_ancestors
    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(ComputeCache::new(capacity));
        self
    }

    /// A reference to the attached cache, if any. Useful for tests that assert
    /// on cache state (e.g. that a warm walk populated entries).
    pub fn cache(&self) -> Option<&ComputeCache> {
        self.cache.as_ref()
    }

    /// The currently active [`MediaContext`].
    pub fn media(&self) -> &MediaContext {
        &self.media
    }

    /// Compute `node`'s style using the current stack top as its parent, push
    /// the result onto the stack, and return an owned copy.
    ///
    /// When the stylesheet `has_combinators()`, this also snapshots `node`'s
    /// selector-relevant fields onto an ancestor-identity stack so descendant
    /// (`A B`) and child (`A > B`) selectors can match against the chain. The
    /// identity stack is only maintained in that case — combinator-free
    /// stylesheets pay no added cost here.
    ///
    /// `@media` rules are gated against [`media`](Self::media); set it via
    /// [`set_media`](Self::set_media) / [`with_media`](Self::with_media) before
    /// entering nodes whose rules depend on it.
    pub fn enter(&mut self, node: &dyn StyledNode) -> ComputedStyle {
        let parent = self.stack.last();
        let has_comb = self.sheet.has_combinators();
        // D = the depth this node will sit at once pushed.
        let depth = self.stack.len();

        let (computed, sig) = if let Some(cache) = self.cache.as_mut() {
            // Cached path. Build the parent's signature from the sig stack (the
            // last pushed sig is this node's parent's sig, transitive).
            let parent_sig = self.sig_stack.last().copied();
            if has_comb {
                let prev_sibs: &[NodeIdentity] = self
                    .siblings
                    .get(depth)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                self.sheet.compute_cached_ancestors(
                    node,
                    parent,
                    parent_sig,
                    &self.identity_stack,
                    prev_sibs,
                    &self.media,
                    &mut self.scratch,
                    cache,
                )
            } else {
                self.sheet.compute_cached(
                    node,
                    parent,
                    parent_sig,
                    &self.media,
                    &mut self.scratch,
                    cache,
                )
            }
        } else {
            // Uncached path: byte-for-byte identical to the pre-cache baseline.
            let c = if has_comb {
                let prev_sibs: &[NodeIdentity] = self
                    .siblings
                    .get(depth)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                self.sheet.compute_with_ancestors_media(
                    node,
                    parent,
                    &mut self.scratch,
                    &self.identity_stack,
                    prev_sibs,
                    &self.media,
                )
            } else {
                self.sheet
                    .compute_with_media(node, parent, &mut self.scratch, &self.media)
            };
            (c, 0)
        };

        self.stack.push(computed.clone());
        if self.cache.is_some() {
            self.sig_stack.push(sig);
        }
        if has_comb {
            self.identity_stack.push(NodeIdentity::from_node(node));
            // The node's children start with no previous siblings: ensure the
            // slot at depth+1 exists and clear it.
            let child_depth = depth + 1;
            if self.siblings.len() <= child_depth {
                self.siblings.resize_with(child_depth + 1, Vec::new);
            }
            self.siblings[child_depth].clear();
        }
        computed
    }

    /// Pop the most recently `enter`ed node (leaving its subtree).
    pub fn leave(&mut self) -> Option<ComputedStyle> {
        // Keep the stacks in sync: pop the identity stack iff it is
        // maintained (i.e. only when the stylesheet has combinators); pop the
        // sig stack iff caching is on.
        if self.sheet.has_combinators() && !self.identity_stack.is_empty() {
            let popped = self.identity_stack.pop().expect("identity stack non-empty");
            // The departed node sat at depth D == self.stack.len() AFTER the
            // style-stack pop below; but we read it before popping the style
            // stack, so its depth is the current stack length minus 1. Record
            // it as a previous sibling for the NEXT sibling at the same depth.
            let depth = self.stack.len() - 1;
            if self.siblings.len() <= depth {
                self.siblings.resize_with(depth + 1, Vec::new);
            }
            self.siblings[depth].push(popped);
        }
        if self.cache.is_some() && !self.sig_stack.is_empty() {
            self.sig_stack.pop();
        }
        self.stack.pop()
    }

    /// The current stack top — the style a subsequent `enter` will inherit
    /// from. `None` at the root (depth 0).
    pub fn current(&self) -> Option<&ComputedStyle> {
        self.stack.last()
    }

    /// Number of nodes currently on the stack (the tree depth).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The backing stylesheet.
    pub fn sheet(&self) -> &Stylesheet {
        self.sheet
    }
}

/// A reusable cascade scratch buffer.
///
/// Held across many [`Stylesheet::compute_with`] calls, it retains its
/// capacity so the per-`compute` matching buffer stops allocating once it has
/// warmed up. It stores **rule indices** (`Vec<usize>`), not references, so it
/// carries no lifetime parameter and can be owned long-term by the caller
/// without borrowing the stylesheet.
///
/// ```rust,ignore
/// let mut scratch = ComputeScratch::new();
/// // reuse across the whole draw loop:
/// for node in &nodes {
///     let style = sheet.compute_with(node, parent, &mut scratch);
/// }
/// ```
pub struct ComputeScratch {
    matching: Vec<usize>,
}

impl ComputeScratch {
    pub fn new() -> Self {
        Self {
            matching: Vec::new(),
        }
    }
}

impl Default for ComputeScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylesheet {
    /// Compute the resolved style for `node`, optionally inheriting from
    /// `parent`.
    ///
    /// **Combinator limitation**: this is a one-shot API with no ancestor
    /// context, so rules whose selector carries a descendant (`A B`) or child
    /// (`A > B`) combinator will **not match** here — they require an ancestor
    /// stack, which only a [`CascadeContext`] supplies. Use `CascadeContext`
    /// to evaluate combinator selectors against a real ancestor chain.
    ///
    /// **`@media` limitation**: this one-shot path uses a default
    /// [`MediaContext`] (all-zero / no media info), so media-gated rules with
    /// any condition will NOT apply here. Use [`compute_with_media`](Self::compute_with_media)
    /// or a [`CascadeContext`] with a non-default context to evaluate them.
    ///
    /// Thin wrapper over [`compute_with`](Self::compute_with) with a fresh
    /// [`ComputeScratch`]. Behavior is identical to `compute_with` — this
    /// exists for one-shot callers and backwards compatibility.
    ///
    /// [`CascadeContext`]: crate::CascadeContext
    pub fn compute(&self, node: &dyn StyledNode, parent: Option<&ComputedStyle>) -> ComputedStyle {
        let mut scratch = ComputeScratch::new();
        self.compute_with(node, parent, &mut scratch)
    }

    /// Compute using a caller-provided [`ComputeScratch`] so the matching
    /// buffer is reused across calls (zero allocation once warmed up).
    ///
    /// This is the allocation-conscious entry point for the draw loop. Three
    /// per-frame allocations are eliminated relative to `compute`:
    ///
    /// 1. **Classes** are fetched from the node exactly once (hoisted out of
    ///    the rule loop) and matched via [`Selector::matches_values`], so the
    ///    R-rules × 1-node cost is one `Classes` materialization, not R.
    /// 2. **Matching buffer** lives in `scratch` and is `clear()`-ed, not
    ///    re-allocated.
    /// 3. When the node is a [`NodeRef`](crate::node::NodeRef), the classes
    ///    materialization itself is zero-allocation.
    ///
    /// **Combinator limitation**: like [`compute`](Self::compute), this one-shot
    /// path has no ancestor context, so combinator selectors do not match
    /// here. Use a [`CascadeContext`] for combinator support.
    ///
    /// **`@media` limitation**: uses a default [`MediaContext`], so media-gated
    /// rules do not apply. Use [`compute_with_media`](Self::compute_with_media).
    ///
    /// [`CascadeContext`]: crate::CascadeContext
    pub fn compute_with(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
    ) -> ComputedStyle {
        // Default media context: media-gated rules with any condition will NOT
        // match (a default context carries no terminal info).
        self.compute_with_media(node, parent, scratch, &MediaContext::default())
    }

    /// Media-aware compute: like [`compute_with`](Self::compute_with) but
    /// evaluates `@media`-gated rules against the supplied [`MediaContext`].
    ///
    /// A rule tagged with a query matches only when
    /// [`MediaQuery::matches`](crate::media::MediaQuery::matches) the context;
    /// untagged (`media: None`) rules always apply. The media check is a cheap
    /// `Option::is_some()` fast-path on the no-media hot path — rules with no
    /// query pay no added cost.
    ///
    /// Use this in a draw loop that tracks terminal size (and optionally color
    /// capability) so width-/color-conditional rules apply per frame:
    ///
    /// ```rust,ignore
    /// let media = MediaContext { cols: size.width, rows: size.height, ..Default::default() };
    /// let style = sheet.compute_with_media(&node, parent, &mut scratch, &media);
    /// ```
    pub fn compute_with_media(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
        media: &MediaContext,
    ) -> ComputedStyle {
        // `None` → cheap raw-args matching path (no NodeIdentity allocation).
        // Combinator selectors never match here; they need a CascadeContext.
        self.compute_inner(node, parent, scratch, None, None, media)
    }

    /// Combinator- + media-aware compute: the full-featured entry point used by
    /// [`CascadeContext::enter`] when the stylesheet `has_combinators()`.
    /// Evaluates selectors against `ancestors` and `siblings` (for `+`/`~`)
    /// and `@media` rules against `media`.
    ///
    /// [`CascadeContext::enter`]: crate::CascadeContext::enter
    pub(crate) fn compute_with_ancestors_media(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
        ancestors: &[NodeIdentity],
        siblings: &[NodeIdentity],
        media: &MediaContext,
    ) -> ComputedStyle {
        self.compute_inner(node, parent, scratch, Some(ancestors), Some(siblings), media)
    }

    /// Cached one-shot compute: like [`compute_with_media`](Self::compute_with_media)
    /// but consults `cache` first. The returned `u64` is the node's signature —
    /// pass it as the next child's `parent_sig` so the ancestor chain is
    /// transitively captured by the signature fold.
    ///
    /// **Combinator limitation**: this one-shot path has no ancestor context
    /// (same as [`compute_with_media`](Self::compute_with_media)), so
    /// combinator selectors do NOT match here. For combinator-aware caching use
    /// [`compute_cached_ancestors`](Self::compute_cached_ancestors) (which
    /// [`CascadeContext::enter`](crate::CascadeContext::enter) picks
    /// automatically when the stylesheet `has_combinators()`).
    pub fn compute_cached(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        parent_sig: Option<u64>,
        media: &MediaContext,
        scratch: &mut ComputeScratch,
        cache: &mut ComputeCache,
    ) -> (ComputedStyle, u64) {
        let node_id = NodeIdentity::from_node(node);
        let sig = node_signature(&node_id, parent_sig, &[], media);
        if let Some(hit) = cache.get(sig, self.generation()) {
            return (hit, sig);
        }
        let computed = self.compute_with_media(node, parent, scratch, media);
        cache.insert(sig, computed.clone(), self.generation());
        (computed, sig)
    }

    /// Cached combinator-aware compute: like
    /// [`compute_with_ancestors_media`](Self::compute_with_ancestors_media) but
    /// consults `cache` first. Used by
    /// [`CascadeContext::enter`](crate::CascadeContext::enter) when the
    /// stylesheet `has_combinators()` AND a cache is attached.
    ///
    /// The signature captures the ancestor chain via `parent_sig` (each
    /// ancestor's sig folds its own parent's sig), so this is correct: a hit
    /// against a signature built from the full ancestor stack yields the same
    /// `ComputedStyle` as a fresh compute through
    /// [`compute_with_ancestors_media`](Self::compute_with_ancestors_media).
    #[allow(clippy::too_many_arguments)] // threading all combinator + cache context
    pub(crate) fn compute_cached_ancestors(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        parent_sig: Option<u64>,
        ancestors: &[NodeIdentity],
        siblings: &[NodeIdentity],
        media: &MediaContext,
        scratch: &mut ComputeScratch,
        cache: &mut ComputeCache,
    ) -> (ComputedStyle, u64) {
        let node_id = NodeIdentity::from_node(node);
        let sig = node_signature(&node_id, parent_sig, siblings, media);
        if let Some(hit) = cache.get(sig, self.generation()) {
            return (hit, sig);
        }
        let computed =
            self.compute_with_ancestors_media(node, parent, scratch, ancestors, siblings, media);
        cache.insert(sig, computed.clone(), self.generation());
        (computed, sig)
    }

    /// Shared compute body. `ancestors` selects the matching path:
    ///
    /// - `None` — cheap raw-args path ([`Selector::matches_values`]). No
    ///   [`NodeIdentity`] is built; combinator selectors never match. This
    ///   preserves the no-combinator hot path's zero-allocation property.
    /// - `Some(stack)` — combinator-aware path: builds one `NodeIdentity` for
    ///   the node and matches via [`Selector::matches_chain`] against `stack`
    ///   and the `siblings` slice (empty when `siblings` is `None`, as on the
    ///   one-shot paths). Used only when the stylesheet `has_combinators()`.
    ///
    /// `media` gates `@media`-tagged rules: a rule whose query does not match
    /// `media` is skipped. The check is `Option::is_some()`-fast for
    /// `media: None` rules (the common, no-`@media` case).
    fn compute_inner(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
        ancestors: Option<&[NodeIdentity]>,
        siblings: Option<&[NodeIdentity]>,
        media: &MediaContext,
    ) -> ComputedStyle {
        let rules = self.rules();

        // 1. Collect matching rule *indices* into the reused scratch buffer.
        //    Rules are stored pre-sorted by (origin, specificity, order) — see
        //    `Stylesheet::sort_rules` — so the indices land in ascending
        //    priority order as a side effect of iterating a sorted slice.
        scratch.matching.clear();
        match ancestors {
            None => {
                // Cheap raw-args path: hoist node fields once, no NodeIdentity.
                let type_name = node.type_name();
                let id = node.id();
                let classes: Classes<'_> = node.classes();
                let state = node.state();
                let position = node.position();
                for (i, r) in rules.iter().enumerate() {
                    if r.selector.matches_values(type_name, id, &classes, state, &position)
                        && rule_media_matches(&r.media, media)
                    {
                        scratch.matching.push(i);
                    }
                }
            }
            Some(stack) => {
                // Combinator-aware path: build one NodeIdentity for the node,
                // then match every selector (combinator or not) via matches_chain.
                let node_id = NodeIdentity::from_node(node);
                let sibs: &[NodeIdentity] = siblings.unwrap_or(&[]);
                for (i, r) in rules.iter().enumerate() {
                    if r.selector.matches_chain(&node_id, stack, sibs)
                        && rule_media_matches(&r.media, media)
                    {
                        scratch.matching.push(i);
                    }
                }
            }
        }

        // 2. Fold declarations (later = higher priority). The per-`compute`
        //    sort by (origin, specificity, order) that used to live here is
        //    gone: rules are already sorted at mutation time, so the
        //    iteration above visits them in priority order.
        let mut own = CssStyle::new();
        for &i in &scratch.matching {
            own.overlay(&rules[i].style);
        }

        // 3. Inheritance.
        if let Some(parent) = parent {
            resolve_explicit_inherit(&mut own, &parent.style);
            own.inherit_from(&parent.style);
        }

        // 4. var() resolution against the stylesheet's token table. The active
        //    `MediaContext` is threaded in so `:root { --x }` overrides declared
        //    inside a matching `@media` block participate.
        resolve_vars_in_place(&mut own, self.tokens(), media);

        ComputedStyle::new(own)
    }
}

/// The media-matching predicate for a rule: `true` when the rule is untagged
/// (`media: None`) or its query matches `ctx`. Inlined into the hot rule loop;
/// for `None` rules this collapses to a single `is_some()` check with no query
/// evaluation.
#[inline]
fn rule_media_matches(query: &Option<crate::media::MediaQuery>, ctx: &MediaContext) -> bool {
    match query {
        None => true,
        Some(q) => q.matches(ctx),
    }
}

/// Replace explicit `inherit` keyword colors with the parent's value, for all
/// three color fields (CSS `inherit` forces inheritance even for
/// non-inheritable properties like `background`).
fn resolve_explicit_inherit(own: &mut CssStyle, parent: &CssStyle) {
    if matches!(own.color, Some(Color::Inherit)) {
        own.color = parent.color.clone();
    }
    if matches!(own.background, Some(Color::Inherit)) {
        own.background = parent.background.clone();
    }
    if matches!(own.underline_color, Some(Color::Inherit)) {
        own.underline_color = parent.underline_color.clone();
    }
}

/// Resolve every `var()` / leftover `inherit` in the color and length fields
/// to a literal — including the `Color` nested inside a `border` spec. Color
/// fields degrade to `Reset` on failure; length fields degrade to `Auto` —
/// both lenient, neither panics.
///
/// `media` gates `:root { --x }` overrides declared inside `@media` blocks: a
/// matching query's overrides win over the default map (last-matching wins),
/// with the default map always consulted as fallback. Pass
/// [`MediaContext::default`] on the one-shot paths where media-gated tokens are
/// documented not to apply.
fn resolve_vars_in_place(style: &mut CssStyle, tokens: &ThemeTokens, media: &MediaContext) {
    resolve_color_field(&mut style.color, tokens, media);
    resolve_color_field(&mut style.background, tokens, media);
    resolve_color_field(&mut style.underline_color, tokens, media);
    // The border color is a `Color` nested inside `Option<BorderSpec>`, so it is
    // not covered by the top-level field passes above. Resolve it here too, or a
    // `border: rounded var(--dim)` survives the cascade as a `Var` and `paint`
    // drops it — the border then draws with no explicit color.
    if let Some(border) = style.border.as_mut() {
        resolve_color_field(&mut border.color, tokens, media);
    }
    resolve_length_field(&mut style.width, tokens, media);
    resolve_length_field(&mut style.height, tokens, media);
}

fn resolve_color_field(field: &mut Option<Color>, tokens: &ThemeTokens, media: &MediaContext) {
    if let Some(inner) = field {
        match inner {
            Color::Literal(_) | Color::Reset => {} // already concrete
            Color::Var { .. } | Color::Inherit => {
                *field = Some(Color::Literal(token::resolve_with_media(inner, tokens, media)));
            }
        }
    }
}

/// Mirrors [`resolve_color_field`] for the length path (width/height). A
/// `Length::Var` is resolved against the token table; anything else is left
/// untouched. Failures (undefined name, type mismatch, cycle) degrade to
/// [`Length::Auto`] — consistent with the lenient color path degrading to
/// `Reset`.
fn resolve_length_field(field: &mut Option<Length>, tokens: &ThemeTokens, media: &MediaContext) {
    if let Some(inner) = field {
        if let Length::Var { .. } = inner {
            *field = Some(token::resolve_length_with_media(inner, tokens, media));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{NodeRef, OwnedNode, State};
    use crate::stylesheet::Origin;
    use ratatui::style::Color as RC;

    fn sheet() -> Stylesheet {
        let mut s = Stylesheet::with_tokens(
            crate::token::ThemeTokens::new().set("accent", Color::literal(RC::Cyan)),
        );
        // type rule (low specificity)
        s.add("Button", CssStyle::new().color(RC::Gray), Origin::User)
            .unwrap();
        // class rule (higher specificity)
        s.add(
            "Button.primary",
            CssStyle::new().background(RC::Blue),
            Origin::User,
        )
        .unwrap();
        // id rule (highest specificity)
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        // focus pseudo-state
        s.add(
            "Button:focus",
            CssStyle::new().background(RC::Green),
            Origin::User,
        )
        .unwrap();
        // var() consumer
        s.add(
            ".accented",
            CssStyle::new().color(Color::var("accent")),
            Origin::User,
        )
        .unwrap();
        // inline (origin) overrides specificity
        s
    }

    #[test]
    fn specificity_wins() {
        let s = sheet();
        let n = OwnedNode::new("Button")
            .with_id("save")
            .with_classes(["primary"]);
        let c = s.compute(&n, None);
        // #save (id) wins over Button (type) for color.
        assert_eq!(c.style.color, Some(Color::literal(RC::Yellow)));
        // .primary (class) wins over Button (type) for background.
        assert_eq!(c.style.background, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn pseudo_state_matches() {
        let s = sheet();
        let n = OwnedNode::new("Button").with_state(State::focus());
        let c = s.compute(&n, None);
        assert_eq!(c.style.background, Some(Color::literal(RC::Green)));
    }

    #[test]
    fn nth_child_cascade_end_to_end() {
        // :nth-child(odd) should set red only for odd positions (1-based).
        let mut s = Stylesheet::new();
        s.add(
            "Item:nth-child(odd)",
            CssStyle::new().color(RC::Red),
            Origin::User,
        )
        .unwrap();

        // sibling_count = 3. index 0 → 1-based 1 (odd) → red.
        let first = OwnedNode::new("Item").with_position(crate::node::Position::new(0, 3));
        // index 1 → 1-based 2 (even) → no rule → None.
        let second = OwnedNode::new("Item").with_position(crate::node::Position::new(1, 3));
        // index 2 → 1-based 3 (odd) → red.
        let third = OwnedNode::new("Item").with_position(crate::node::Position::new(2, 3));

        assert_eq!(
            s.compute(&first, None).style.color,
            Some(Color::literal(RC::Red))
        );
        assert_eq!(s.compute(&second, None).style.color, None);
        assert_eq!(
            s.compute(&third, None).style.color,
            Some(Color::literal(RC::Red))
        );
    }

    #[test]
    fn nth_child_default_position_does_not_match() {
        // A node with default Position (sibling_count 0) must not match
        // :nth-child(odd) even though its index defaults to 0 (1-based 1, odd).
        let mut s = Stylesheet::new();
        s.add(
            "Item:nth-child(odd)",
            CssStyle::new().color(RC::Red),
            Origin::User,
        )
        .unwrap();

        let n = OwnedNode::new("Item"); // default position
        assert_eq!(n.position().sibling_count, 0);
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, None);
    }

    #[test]
    fn var_resolves_from_tokens() {
        let s = sheet();
        let n = OwnedNode::new("Text").with_classes(["accented"]);
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, Some(Color::literal(RC::Cyan)));
    }

    #[test]
    fn border_color_var_resolves_from_tokens() {
        // Regression for the 0.1.1 limitation: a `var()` in the border
        // shorthand color must be resolved against the token table, not left as
        // a `Color::Var` (which `paint` would silently drop).
        //
        // This is the exact downstream DetailPanel case: `#003237` was used as a
        // literal in place of `var(--border-dim)` because the cascade did not
        // resolve border colors. With the fix, the token resolves.
        let sheet = Stylesheet::parse(
            ":root{--border-dim:#003237} .panel { border: rounded var(--border-dim); }",
        )
        .unwrap();
        let n = OwnedNode::new("Div").with_classes(["panel"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyle::Rounded);
        assert_eq!(
            border.color,
            Some(Color::literal(RC::Rgb(0x00, 0x32, 0x37)))
        );
    }

    #[test]
    fn border_color_var_via_subdeclaration_resolves() {
        // The `border-color: var(--x)` sub-declaration path must resolve too —
        // it lands in the same nested `BorderSpec.color` field.
        let sheet = Stylesheet::parse(
            ":root{--rim:#ff0000} .b { border-style: single; border-color: var(--rim); }",
        )
        .unwrap();
        let n = OwnedNode::new("Div").with_classes(["b"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(
            border.color,
            Some(Color::literal(RC::Rgb(0xff, 0x00, 0x00)))
        );
    }

    #[test]
    fn border_color_var_fallback_resolves() {
        // An undefined border-color var with a fallback degrades to the
        // fallback, mirroring the other color fields.
        let sheet = Stylesheet::parse(".b { border: rounded var(--nope, #00ff00); }").unwrap();
        let n = OwnedNode::new("Div").with_classes(["b"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(
            border.color,
            Some(Color::literal(RC::Rgb(0x00, 0xff, 0x00)))
        );
    }

    #[test]
    fn inheritance_from_parent() {
        let s = sheet();
        let parent_node = OwnedNode::new("Button").with_classes(["primary"]);
        let parent = s.compute(&parent_node, None);
        // Child Text has no color of its own; inherits parent's.
        let child = OwnedNode::new("Text");
        let computed = s.compute(&child, Some(&parent));
        assert_eq!(computed.style.color, Some(Color::literal(RC::Gray)));
    }

    #[test]
    fn origin_overrides_specificity() {
        let mut s = Stylesheet::new();
        s.add("Button", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        // Inline origin wins despite identical selector.
        s.add("Button", CssStyle::new().color(RC::Blue), Origin::Inline)
            .unwrap();
        let n = OwnedNode::new("Button");
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn rules_stored_in_cascade_sorted_order() {
        // Insert rules in deliberately scrambled origin/specificity order and
        // assert the stored slice comes out sorted ascending by
        // (origin, specificity, order).
        let mut s = Stylesheet::new();
        // type rule, Origin::User — (User, (0,0,1), 0)
        s.add("Button", CssStyle::new(), Origin::User).unwrap();
        // id rule, Origin::User — (User, (1,0,0), 1)
        s.add("#save", CssStyle::new(), Origin::User).unwrap();
        // class rule, Origin::User — (User, (0,1,0), 2)
        s.add(".primary", CssStyle::new(), Origin::User).unwrap();
        // type rule, Origin::Inline — (Inline, (0,0,1), 3)
        s.add("Button", CssStyle::new(), Origin::Inline).unwrap();
        // class rule, Origin::UserAgent — (UA, (0,1,0), 4)
        s.add(".primary", CssStyle::new(), Origin::UserAgent)
            .unwrap();

        let rules = s.rules();
        for w in rules.windows(2) {
            let a = &w[0];
            let b = &w[1];
            let ka = (a.origin, a.selector.specificity(), a.order);
            let kb = (b.origin, b.selector.specificity(), b.order);
            assert!(ka <= kb, "rules not sorted: {ka:?} > {kb:?}");
        }

        // Spot-check the extremes: the lowest-priority rule is the UserAgent
        // class (origin UA) and the highest is the Inline type rule.
        assert_eq!(rules.first().unwrap().origin, Origin::UserAgent);
        assert_eq!(rules.last().unwrap().origin, Origin::Inline);
    }

    #[test]
    fn compute_unchanged_after_sort_removal_scrambled_insertion() {
        // Mirror `specificity_wins` + `origin_overrides_specificity` but with
        // rules inserted in a deliberately hostile (high→low priority) order,
        // so that removing the per-`compute` sort would visibly break the
        // result if rules weren't pre-sorted.
        let mut s = Stylesheet::new();
        // highest specificity first, lowest last (reverse of priority).
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        s.add(
            "Button.primary",
            CssStyle::new().background(RC::Blue),
            Origin::User,
        )
        .unwrap();
        s.add("Button", CssStyle::new().color(RC::Gray), Origin::User)
            .unwrap();

        let n = OwnedNode::new("Button")
            .with_id("save")
            .with_classes(["primary"]);
        let c = s.compute(&n, None);
        // id beats class beats type: #save color wins over Button color.
        assert_eq!(c.style.color, Some(Color::literal(RC::Yellow)));
        // .primary background wins over Button (type) background (none).
        assert_eq!(c.style.background, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn inline_origin_wins_in_scrambled_insertion_order() {
        // Inline origin beats User even when the User rule is added last and
        // has equal specificity — stresses the (origin, …) sort key.
        let mut s = Stylesheet::new();
        s.add("Button", CssStyle::new().color(RC::Blue), Origin::Inline)
            .unwrap();
        s.add("Button", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        let n = OwnedNode::new("Button");
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn render_computed_applies_margin_once() {
        // Regression: render_computed must render the block into the
        // margin-shrunk area and the widget into the block's inner area, with
        // apply_margin run exactly once. We can't easily materialize a
        // ratatui::Frame in a unit test, so we pin the area invariant that
        // render_computed now computes via a single apply_margin + the
        // shared `layout_with_shrunk` helper (instead of calling
        // apply_margin twice as it used to).
        let computed = ComputedStyle::new(
            CssStyle::new()
                .margin("2")
                .padding("1")
                .border("rounded #00d4ff"),
        );
        let area = Rect::new(0, 0, 44, 8);

        // This is exactly the sequence render_computed runs internally now.
        let shrunk = computed.apply_margin(area);
        let (_block, _style, inner) = computed.layout_with_shrunk(shrunk);

        // Block renders into shrunk (margin removed on each side).
        assert_eq!(shrunk, Rect::new(2, 2, 40, 4));
        // Widget renders into inner (margin + border + padding removed).
        assert_eq!(inner, Rect::new(4, 4, 36, 0));
    }

    #[test]
    fn with_inline_overrides_specificity() {
        // An id selector has the highest specificity in the sheet, but an inline
        // declaration layered on top post-compute must still win — that is the
        // whole point of inline origin being applied last.
        let mut s = Stylesheet::new();
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        let n = OwnedNode::new("Button").with_id("save");
        let c = s
            .compute(&n, None)
            .with_inline(&CssStyle::new().color("red"));
        // The id rule set Yellow; inline red wins.
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn apply_inline_in_place_overrides() {
        // Same semantics, mutating form.
        let mut s = Stylesheet::new();
        s.add(
            "Button.primary",
            CssStyle::new().color(RC::Blue),
            Origin::User,
        )
        .unwrap();
        let n = OwnedNode::new("Button").with_classes(["primary"]);
        let mut c = s.compute(&n, None);
        c.apply_inline(&CssStyle::new().color("red"));
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn layout_inner_matches_handwritten_sequence() {
        // A fully-featured style: margin (outer), rounded border, padding.
        let computed = ComputedStyle::new(
            CssStyle::new()
                .margin("2")
                .padding("1")
                .border("rounded #00d4ff"),
        );
        let area = Rect::new(0, 0, 44, 8);

        let (_block, _style, inner_from_layout) = computed.layout(area);

        // The hand-written sequence layout() must be equivalent to.
        let shrunk = computed.apply_margin(area);
        let block = computed.to_block();
        let inner_from_hand = block.inner(shrunk);

        assert_eq!(inner_from_layout, inner_from_hand);
        // Sanity: with margin 2 (each side) + 1 border + 1 padding, the inner
        // width drops by 2*(2+1+1) = 8, and height by 8 too.
        assert_eq!(inner_from_layout, Rect::new(4, 4, 36, 0));
    }

    #[test]
    fn layout_inner_equals_area_with_no_box_model() {
        let computed = ComputedStyle::new(CssStyle::new());
        let area = Rect::new(0, 0, 30, 10);
        let (_block, _style, inner) = computed.layout(area);
        assert_eq!(inner, area);
    }

    #[test]
    fn layout_content_style_matches_to_style() {
        let computed = ComputedStyle::new(CssStyle::new().color(RC::Cyan).bold().padding("1"));
        let area = Rect::new(0, 0, 20, 5);
        let (_block, style, _inner) = computed.layout(area);
        assert_eq!(style, computed.to_style());
    }

    // ---------------------------------------------------------------------
    // NodeRef / compute_with parity & reuse
    // ---------------------------------------------------------------------

    fn parity_sheet() -> Stylesheet {
        let mut s = Stylesheet::new();
        s.add("Button", CssStyle::new().color(RC::Gray), Origin::User)
            .unwrap();
        s.add(
            "Button.primary",
            CssStyle::new().background(RC::Blue),
            Origin::User,
        )
        .unwrap();
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        s.add(
            "Button:focus",
            CssStyle::new().background(RC::Green),
            Origin::User,
        )
        .unwrap();
        s
    }

    #[test]
    fn noderef_behavioral_parity() {
        // Same data via OwnedNode vs NodeRef → identical ComputedStyle across
        // all four selector dimensions (type, class, id, state).
        let sheet = parity_sheet();

        let owned = OwnedNode::new("Button")
            .with_id("save")
            .with_classes(["primary"])
            .with_state(State::focus());
        let borrowed = NodeRef::new("Button")
            .id("save")
            .classes(&["primary"])
            .state(State::focus());

        let c_owned = sheet.compute(&owned, None);
        let c_borrowed = sheet.compute(&borrowed, None);
        assert_eq!(c_owned, c_borrowed);
    }

    #[test]
    fn noderef_zero_string_construction() {
        // Pure &'static str path — no String/Vec heap allocation is possible.
        let sheet = parity_sheet();
        let node = NodeRef::new("Button")
            .classes(&["primary"])
            .state(State::focus());
        let c = sheet.compute(&node, None);
        // type + class match for background; color from the type rule.
        // (Button.primary sets Blue, Button:focus sets Green; same specificity
        // (0,1,1) so source order wins → Green, added last.)
        assert_eq!(c.style.background, Some(Color::literal(RC::Green)));
        assert_eq!(c.style.color, Some(Color::literal(RC::Gray)));
    }

    #[test]
    fn compute_with_matches_compute() {
        let sheet = parity_sheet();
        let mut scratch = ComputeScratch::new();

        let cases: [(&str, OwnedNode); 5] = [
            ("plain", OwnedNode::new("Button")),
            (
                "primary",
                OwnedNode::new("Button").with_classes(["primary"]),
            ),
            ("id", OwnedNode::new("Button").with_id("save")),
            ("focus", OwnedNode::new("Button").with_state(State::focus())),
            (
                "combo",
                OwnedNode::new("Button")
                    .with_id("save")
                    .with_classes(["primary"])
                    .with_state(State::focus()),
            ),
        ];

        for (name, node) in cases {
            let via_compute = sheet.compute(&node, None);
            let via_compute_with = sheet.compute_with(&node, None, &mut scratch);
            assert_eq!(via_compute, via_compute_with, "mismatch for case `{name}`");
        }
    }

    #[test]
    fn scratch_reuse_no_panic() {
        // Reuse the same scratch across many computes of varying sizes — the
        // clear()+push() path must stay correct and never leak prior results.
        let sheet = parity_sheet();
        let mut scratch = ComputeScratch::new();

        // A node that matches many rules, then one that matches none, then a
        // big one again — exercise the clear/reuse.
        let big = NodeRef::new("Button")
            .id("save")
            .classes(&["primary"])
            .state(State::focus());
        let none = NodeRef::new("NoSuchType");

        let c1 = sheet.compute_with(&big, None, &mut scratch);
        let c_none = sheet.compute_with(&none, None, &mut scratch);
        let c2 = sheet.compute_with(&big, None, &mut scratch);

        // The "none" node only matches the universal-less base → no rules.
        assert_eq!(c_none.style.color, None);
        // Re-running the big node after the empty one yields the same result.
        assert_eq!(c1, c2);
        assert_eq!(c1.style.color, Some(Color::literal(RC::Yellow)));
    }

    // ---------------------------------------------------------------------
    // CascadeContext
    // ---------------------------------------------------------------------

    fn context_sheet() -> Stylesheet {
        // Panel sets a color; Text sets none → Text inherits Panel's color.
        let mut s = Stylesheet::new();
        s.add("Panel", CssStyle::new().color("#cdd6f4"), Origin::User)
            .unwrap();
        s
    }

    #[test]
    fn context_inherits_without_manual_threading() {
        // enter(Panel) then enter(Text) — Text should inherit Panel's color
        // without the test ever writing `Some(&parent)`.
        let sheet = context_sheet();
        let mut ctx = CascadeContext::new(&sheet);

        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));

        assert_eq!(
            text.style.color,
            Some(Color::literal(RC::Rgb(0xcd, 0xd6, 0xf4)))
        );
    }

    #[test]
    fn context_parity_with_manual_compute() {
        // Same small tree (Root→Panel→Text) computed two ways: CascadeContext
        // vs the hand-written compute(node, Some(&parent)) chain. Every node's
        // ComputedStyle must be identical.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Root", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        sheet
            .add("Panel", CssStyle::new().padding("1"), Origin::User)
            .unwrap();
        // Text sets nothing → inherits everything inheritable from Panel/Root.
        sheet.add("Text", CssStyle::new(), Origin::User).unwrap();

        // --- CascadeContext path ---
        let mut ctx = CascadeContext::new(&sheet);
        let ctx_root = ctx.enter(&OwnedNode::new("Root"));
        let ctx_panel = ctx.enter(&OwnedNode::new("Panel"));
        let ctx_text = ctx.enter(&OwnedNode::new("Text"));

        // --- Manual threading path ---
        let man_root = sheet.compute(&OwnedNode::new("Root"), None);
        let man_panel = sheet.compute(&OwnedNode::new("Panel"), Some(&man_root));
        let man_text = sheet.compute(&OwnedNode::new("Text"), Some(&man_panel));

        assert_eq!(ctx_root, man_root);
        assert_eq!(ctx_panel, man_panel);
        assert_eq!(ctx_text, man_text);
    }

    #[test]
    fn context_leave_restores_parent() {
        // enter A (color), enter B (different color), leave, enter C (no color)
        // → C must inherit from A, not B.
        let mut sheet = Stylesheet::new();
        sheet
            .add("A", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        sheet
            .add("B", CssStyle::new().color(RC::Blue), Origin::User)
            .unwrap();
        // C has no color rule.
        sheet.add("C", CssStyle::new(), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _a = ctx.enter(&OwnedNode::new("A"));
        let _b = ctx.enter(&OwnedNode::new("B"));
        ctx.leave(); // drop B
        let c = ctx.enter(&OwnedNode::new("C"));

        // C inherits A's color (Red), not B's (Blue).
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn context_depth() {
        let sheet = context_sheet();
        let mut ctx = CascadeContext::new(&sheet);

        assert_eq!(ctx.depth(), 0);
        ctx.enter(&OwnedNode::new("Panel"));
        assert_eq!(ctx.depth(), 1);
        ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(ctx.depth(), 2);
        ctx.leave();
        assert_eq!(ctx.depth(), 1);
        ctx.leave();
        assert_eq!(ctx.depth(), 0);
        assert!(ctx.leave().is_none());
    }

    #[test]
    fn context_scratch_reused() {
        // Many consecutive enters of mixed nodes — the internal scratch buffer
        // is cleared and reused each time; correctness must not regress.
        let mut sheet = Stylesheet::new();
        sheet
            .add("A", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        sheet
            .add("A.child", CssStyle::new().bold(), Origin::User)
            .unwrap();
        sheet
            .add("NoMatch", CssStyle::new().color(RC::Green), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);

        // child matches two rules (A + A.child); NoMatch matches none.
        let child = ctx.enter(&OwnedNode::new("A").with_classes(["child"]));
        assert_eq!(child.style.color, Some(Color::literal(RC::Red)));

        let none = ctx.enter(&OwnedNode::new("TotallyUnknown"));
        // No matching rule, no inheritable parent value set on color here
        // (parent A had Red, and color is inheritable) → inherits Red.
        assert_eq!(none.style.color, Some(Color::literal(RC::Red)));

        // Re-run child-like after a no-match — must not leak prior matching set.
        ctx.leave();
        let child2 = ctx.enter(&OwnedNode::new("A").with_classes(["child"]));
        assert_eq!(child2.style.color, Some(Color::literal(RC::Red)));
    }

    // ---------------------------------------------------------------------
    // Length var() resolution (width/height)
    // ---------------------------------------------------------------------

    #[test]
    fn width_var_resolves() {
        let sheet = Stylesheet::parse(":root{--w:50%} .col { width: var(--w);}").unwrap();
        let node = OwnedNode::new("Div").with_classes(["col"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Percent(50)));
    }

    #[test]
    fn width_var_chain() {
        let sheet =
            Stylesheet::parse(":root{--w: var(--w2); --w2: 10;} .x { width: var(--w); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Cells(10)));
    }

    #[test]
    fn width_var_undefined_degrades_to_auto() {
        // Lenient parse: an undefined var degrades to Auto, no error.
        let sheet = Stylesheet::parse(".x { width: var(--nope); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Auto));
    }

    #[test]
    fn width_var_mistype_degrades_to_auto() {
        // A name bound to a Color is a type mismatch on the length path → Auto.
        let sheet = Stylesheet::parse(":root{--c:#fff} .x { width: var(--c); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Auto));
    }

    #[test]
    fn height_var_resolves() {
        let sheet = Stylesheet::parse(":root{--h:max(8)} .row { height: var(--h); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["row"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.height, Some(crate::box_model::Length::Max(8)));
    }

    #[test]
    fn width_var_undefined_uses_fallback() {
        // An undefined width var WITH a fallback resolves to the fallback,
        // mirroring the color var() path. (Lenient parse; no error.)
        let sheet = Stylesheet::parse(".x { width: var(--nope, 7); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Cells(7)));
    }

    // ---------------------------------------------------------------------
    // Combinators (descendant `A B` + child `A > B`) via CascadeContext
    // ---------------------------------------------------------------------

    #[test]
    fn has_combinators_flag() {
        // A combinator-free sheet stays false.
        let mut plain = Stylesheet::new();
        plain.add("Button", CssStyle::new(), Origin::User).unwrap();
        assert!(!plain.has_combinators());

        // Adding a combinator rule flips it true.
        let mut with_comb = Stylesheet::new();
        with_comb
            .add("Panel Button", CssStyle::new(), Origin::User)
            .unwrap();
        assert!(with_comb.has_combinators());

        // A plain sheet extended with a combinator sheet inherits the flag.
        let mut merged = Stylesheet::new();
        merged.add("Text", CssStyle::new(), Origin::User).unwrap();
        assert!(!merged.has_combinators());
        merged.extend(&with_comb);
        assert!(merged.has_combinators());
    }

    #[test]
    fn descendant_combinator_matches_in_context() {
        // `Panel Text { color: red }` — Text matches as a descendant of Panel.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel Text", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));

        assert_eq!(text.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn child_combinator_direct_child_matches() {
        // `Panel > Text { color: blue }` — Text is a direct child of Panel.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel > Text", CssStyle::new().color(RC::Blue), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));

        assert_eq!(text.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn child_combinator_indirect_child_does_not_match() {
        // `Panel > Text` — when Text's direct parent is Other (not Panel), the
        // child combinator must NOT match, even though Panel is an ancestor.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel > Text", CssStyle::new().color(RC::Blue), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let _other = ctx.enter(&OwnedNode::new("Other"));
        let text = ctx.enter(&OwnedNode::new("Text"));

        // No matching rule → color absent.
        assert_eq!(text.style.color, None);
    }

    #[test]
    fn descendant_vs_child_distinction() {
        // A 3-deep tree Root → Panel → Text.
        // `Root > Text` must NOT match (Text's direct parent is Panel).
        // `Root Text` must match (Text is a descendant of Root).
        let mut child_sheet = Stylesheet::new();
        child_sheet
            .add("Root > Text", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        let mut desc_sheet = Stylesheet::new();
        desc_sheet
            .add("Root Text", CssStyle::new().color(RC::Green), Origin::User)
            .unwrap();

        // Child combinator: does not match the 3-deep tree.
        let mut ctx_c = CascadeContext::new(&child_sheet);
        let _r = ctx_c.enter(&OwnedNode::new("Root"));
        let _p = ctx_c.enter(&OwnedNode::new("Panel"));
        let t_c = ctx_c.enter(&OwnedNode::new("Text"));
        assert_eq!(t_c.style.color, None, "Root > Text must not match a grandchild");

        // Descendant combinator: does match.
        let mut ctx_d = CascadeContext::new(&desc_sheet);
        let _r = ctx_d.enter(&OwnedNode::new("Root"));
        let _p = ctx_d.enter(&OwnedNode::new("Panel"));
        let t_d = ctx_d.enter(&OwnedNode::new("Text"));
        assert_eq!(t_d.style.color, Some(Color::literal(RC::Green)));
    }

    #[test]
    fn non_combinator_rules_match_in_context() {
        // Regression: a plain compound rule still matches through CascadeContext
        // when the sheet happens to also have combinators (exercising the
        // compute_with_ancestors path for ancestor-less selectors).
        let mut sheet = Stylesheet::new();
        sheet
            .add("Button", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        sheet
            .add("Panel Button", CssStyle::new().bold(), Origin::User)
            .unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet);
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let btn = ctx.enter(&OwnedNode::new("Button"));

        // Both rules apply: color from the plain rule, weight bold from the
        // combinator rule.
        assert_eq!(btn.style.color, Some(Color::literal(RC::Yellow)));
        assert!(btn.style.weight.is_some());
    }

    #[test]
    fn combinator_rule_does_not_match_one_shot() {
        // Documented limitation: the one-shot compute() path has no ancestor
        // context, so a combinator selector does NOT apply there.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel Text", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let node = OwnedNode::new("Text");
        let c = sheet.compute(&node, None);
        // No match — color absent.
        assert_eq!(c.style.color, None);
    }

    #[test]
    fn context_leave_keeps_stacks_in_sync() {
        // After leaving a subtree, a re-entered sibling must match against the
        // correct (popped) ancestor chain. This exercises that leave() pops
        // the identity stack alongside the style stack.
        let mut sheet = Stylesheet::new();
        // `Panel > Text` colors red only when Text's direct parent is Panel.
        sheet
            .add("Panel > Text", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text1 = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text1.style.color, Some(Color::literal(RC::Red)));
        ctx.leave(); // pop Text

        // Re-enter Text as a child of Panel again — must still match.
        let text2 = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text2.style.color, Some(Color::literal(RC::Red)));
        ctx.leave(); // pop Text
        ctx.leave(); // pop Panel

        // Now enter Text as a child of Root — must NOT match (Panel is gone).
        let text3 = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text3.style.color, None);
    }

    // ---------------------------------------------------------------------
    // Sibling combinators (`A + B`, `A ~ B`) via CascadeContext
    // ---------------------------------------------------------------------

    #[test]
    fn adjacent_combinator_matches_preceding_sibling() {
        // `Item + Item { color: red }` — three sibling Items under one parent.
        // The 2nd and 3rd each have a preceding Item sibling; the 1st does not.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Item + Item", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));

        let first = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(first.style.color, None, "first Item has no preceding sibling");
        ctx.leave();

        let second = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(
            second.style.color,
            Some(Color::literal(RC::Red)),
            "second Item follows a sibling Item"
        );
        ctx.leave();

        let third = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(
            third.style.color,
            Some(Color::literal(RC::Red)),
            "third Item follows a sibling Item"
        );
    }

    #[test]
    fn general_sibling_combinator_matches_any_preceding() {
        // `Item ~ Item { color: blue }` — same three-item layout: 2nd and 3rd
        // have at least one prior Item sibling.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Item ~ Item", CssStyle::new().color(RC::Blue), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));

        let first = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(first.style.color, None);
        ctx.leave();

        let second = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(second.style.color, Some(Color::literal(RC::Blue)));
        ctx.leave();

        let third = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(third.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn adjacent_combinator_requires_immediate_predecessor_type() {
        // `Header + Content` — Content matches only when its immediately
        // preceding sibling is Header. A Sidebar predecessor must NOT trigger.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Header + Content", CssStyle::new().color(RC::Green), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));

        // Sidebar then Content — immediate predecessor is Sidebar, not Header.
        let _sidebar = ctx.enter(&OwnedNode::new("Sidebar"));
        ctx.leave();
        let content = ctx.enter(&OwnedNode::new("Content"));
        assert_eq!(content.style.color, None);

        ctx.leave();
        // Now Header then Content — match.
        let _header = ctx.enter(&OwnedNode::new("Header"));
        ctx.leave();
        let content2 = ctx.enter(&OwnedNode::new("Content"));
        assert_eq!(content2.style.color, Some(Color::literal(RC::Green)));
    }

    #[test]
    fn sibling_plus_descendant_combinator() {
        // `Panel Item + Item` — an Item that follows an Item sibling, both
        // inside Panel (Panel as ancestor). Exercises the sibling + descendant
        // combination through the full CascadeContext path.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel Item + Item", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));

        let first = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(first.style.color, None);
        ctx.leave();

        let second = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(second.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn sibling_lists_reset_on_new_parent() {
        // Items under ParentA, then items under ParentB: a ParentB item must
        // NOT see ParentA's items as siblings. Verifies the `siblings[D+1]`
        // clear on enter resets the children context.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Item + Item", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _root = ctx.enter(&OwnedNode::new("Root"));

        // ParentA: Item, Item (the second matches `Item + Item`).
        let _pa = ctx.enter(&OwnedNode::new("ParentA"));
        let _pa_first = ctx.enter(&OwnedNode::new("Item"));
        ctx.leave();
        let _pa_second = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(_pa_second.style.color, Some(Color::literal(RC::Red)));
        ctx.leave();
        ctx.leave(); // leave ParentA

        // ParentB: first Item must NOT see ParentA's second Item as a sibling.
        let _pb = ctx.enter(&OwnedNode::new("ParentB"));
        let pb_first = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(pb_first.style.color, None, "ParentB's first item has no prior sibling");
    }

    #[test]
    fn sibling_combinator_does_not_match_one_shot() {
        // Documented limitation: the one-shot compute() path has no sibling
        // context, so a `+`/`~` selector does NOT apply there.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Item + Item", CssStyle::new().color(RC::Red), Origin::User)
            .unwrap();

        let node = OwnedNode::new("Item");
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.color, None);
    }

    #[test]
    fn descendant_combinator_still_matches_via_context() {
        // Regression: existing descendant/child combinators keep working after
        // the sibling-tracking plumbing lands.
        let mut sheet = Stylesheet::new();
        sheet
            .add("Panel Button", CssStyle::new().color(RC::Yellow), Origin::User)
            .unwrap();
        sheet
            .add("Panel > Button", CssStyle::new().bold(), Origin::User)
            .unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let btn = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(btn.style.color, Some(Color::literal(RC::Yellow)));
        assert!(btn.style.weight.is_some());
    }

    // ---------------------------------------------------------------------
    // @media queries
    // ---------------------------------------------------------------------

    fn media_sheet() -> Stylesheet {
        // A media-gated Button rule: only applies when cols >= 80.
        Stylesheet::parse("@media (min-width: 80) { Button { color: red; } }").unwrap()
    }

    #[test]
    fn media_rule_applies_when_context_matches() {
        let sheet = media_sheet();
        let mut scratch = ComputeScratch::new();
        let media = MediaContext { cols: 100, rows: 24, ..Default::default() };
        let c = sheet.compute_with_media(&OwnedNode::new("Button"), None, &mut scratch, &media);
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn media_rule_skipped_when_context_does_not_match() {
        let sheet = media_sheet();
        let mut scratch = ComputeScratch::new();
        let media = MediaContext { cols: 60, rows: 24, ..Default::default() };
        let c = sheet.compute_with_media(&OwnedNode::new("Button"), None, &mut scratch, &media);
        assert_eq!(c.style.color, None, "media-gated rule must not apply when cols < 80");
    }

    #[test]
    fn media_rule_skipped_by_default_context() {
        // The default context (cols=0) must NOT satisfy min-width: 80.
        let sheet = media_sheet();
        let c = sheet.compute(&OwnedNode::new("Button"), None);
        assert_eq!(c.style.color, None, "default-context compute does not apply media-gated rules");
    }

    #[test]
    fn plain_and_media_rules_coexist() {
        // A sheet with BOTH a plain (always-applies) rule and a media-gated rule.
        let sheet = Stylesheet::parse(
            "Button { color: blue; } @media (min-width: 80) { Button { color: red; } }",
        )
        .unwrap();
        let mut scratch = ComputeScratch::new();

        // Small terminal: only the plain rule applies → blue.
        let small = MediaContext { cols: 40, ..Default::default() };
        let c_small = sheet.compute_with_media(&OwnedNode::new("Button"), None, &mut scratch, &small);
        assert_eq!(c_small.style.color, Some(Color::literal(RC::Blue)));

        // Large terminal: media rule (later source order, same specificity) wins → red.
        let large = MediaContext { cols: 120, ..Default::default() };
        let c_large = sheet.compute_with_media(&OwnedNode::new("Button"), None, &mut scratch, &large);
        assert_eq!(c_large.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn cascade_context_with_media_applies_gated_rule() {
        let sheet = media_sheet();
        let mut ctx = CascadeContext::new(&sheet).with_media(MediaContext {
            cols: 100,
            rows: 24,
            ..Default::default()
        });
        let btn = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(btn.style.color, Some(Color::literal(RC::Red)));

        // Switch to a non-matching context and re-enter — rule no longer applies.
        ctx.set_media(MediaContext { cols: 40, ..Default::default() });
        ctx.leave();
        let btn2 = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(btn2.style.color, None);
    }

    #[test]
    fn cascade_context_media_combinator_path() {
        // Stress the compute_with_ancestors_media path: a sheet that has BOTH a
        // combinator rule and a media-gated combinator rule.
        let sheet = Stylesheet::parse(
            "@media (min-width: 80) { Panel Button { color: green; } }",
        )
        .unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet).with_media(MediaContext {
            cols: 100,
            rows: 24,
            ..Default::default()
        });
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let btn = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(btn.style.color, Some(Color::literal(RC::Green)));

        // Small context: the combinator media rule must NOT apply.
        ctx.set_media(MediaContext { cols: 40, ..Default::default() });
        ctx.leave();
        ctx.leave();
        let _panel2 = ctx.enter(&OwnedNode::new("Panel"));
        let btn2 = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(btn2.style.color, None);
    }

    // ---------------------------------------------------------------------
    // Media-gated :root token resolution end-to-end (P4-3)
    // ---------------------------------------------------------------------

    fn media_token_sheet() -> Stylesheet {
        // :root default = red; @media (min-width: 80) override = blue.
        Stylesheet::parse(
            ":root { --accent: red } @media (min-width: 80) { :root { --accent: blue } } .a { color: var(--accent); }",
        )
        .unwrap()
    }

    #[test]
    fn media_gated_token_resolves_blue_under_matching_context() {
        let sheet = media_token_sheet();
        let mut ctx = CascadeContext::new(&sheet).with_media(MediaContext {
            cols: 100,
            ..Default::default()
        });
        let a = ctx.enter(&OwnedNode::new("Div").with_classes(["a"]));
        assert_eq!(a.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn media_gated_token_resolves_red_under_non_matching_context() {
        let sheet = media_token_sheet();
        let mut ctx = CascadeContext::new(&sheet).with_media(MediaContext {
            cols: 60,
            ..Default::default()
        });
        let a = ctx.enter(&OwnedNode::new("Div").with_classes(["a"]));
        assert_eq!(a.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn media_gated_token_resolves_default_via_one_shot_compute() {
        // The one-shot compute path uses a default MediaContext, so the
        // media-gated override does NOT apply — only the default (red) does.
        let sheet = media_token_sheet();
        let a = sheet.compute(&OwnedNode::new("Div").with_classes(["a"]), None);
        assert_eq!(a.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn media_gated_token_via_compute_with_media() {
        let sheet = media_token_sheet();
        let mut scratch = ComputeScratch::new();
        let node = OwnedNode::new("Div").with_classes(["a"]);
        // Matching → blue.
        let large = MediaContext { cols: 100, ..Default::default() };
        let c_large = sheet.compute_with_media(&node, None, &mut scratch, &large);
        assert_eq!(c_large.style.color, Some(Color::literal(RC::Blue)));
        // Non-matching → red.
        let small = MediaContext { cols: 60, ..Default::default() };
        let c_small = sheet.compute_with_media(&node, None, &mut scratch, &small);
        assert_eq!(c_small.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn non_media_tokens_still_resolve_as_before() {
        // Regression: a plain :root token (no @media) resolves exactly as before
        // under both the one-shot and context paths.
        let sheet = Stylesheet::parse(
            ":root { --c: #abcdef } .x { color: var(--c); }",
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let one_shot = sheet.compute(&node, None);
        assert_eq!(
            one_shot.style.color,
            Some(Color::literal(RC::Rgb(0xab, 0xcd, 0xef)))
        );
        let mut ctx = CascadeContext::new(&sheet);
        let via_ctx = ctx.enter(&node);
        assert_eq!(via_ctx.style.color, one_shot.style.color);
    }

    // ---------------------------------------------------------------------
    // ComputeCache via CascadeContext (P4-4)
    // ---------------------------------------------------------------------

    /// Walk a 3-level tree (Root → Panel → Text) once through `ctx`, capturing
    /// each node's `ComputedStyle` into `out` in enter order.
    fn walk_tree_cached(ctx: &mut CascadeContext<'_>, out: &mut Vec<ComputedStyle>) {
        out.push(ctx.enter(&OwnedNode::new("Root")));
        out.push(ctx.enter(&OwnedNode::new("Panel")));
        out.push(ctx.enter(&OwnedNode::new("Text")));
        ctx.leave();
        ctx.leave();
        ctx.leave();
    }

    #[test]
    fn cache_warm_walk_produces_identical_styles() {
        // First (cold) walk misses; second (warm) walk hits on every node.
        // Correctness invariant: the warm walk's results are byte-identical to
        // the cold walk's.
        let mut sheet = Stylesheet::new();
        sheet.add("Root", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("Panel", CssStyle::new().padding("1"), Origin::User).unwrap();
        sheet.add("Text", CssStyle::new().bold(), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet).with_cache(16);
        let mut cold = Vec::new();
        walk_tree_cached(&mut ctx, &mut cold);
        // After the cold walk the cache holds 3 distinct signatures.
        assert_eq!(ctx.cache().unwrap().len(), 3);

        // Second walk — should be served entirely from the cache.
        let mut warm = Vec::new();
        walk_tree_cached(&mut ctx, &mut warm);

        // Correctness: warm == cold.
        assert_eq!(warm.len(), cold.len());
        for (i, (w, c)) in warm.iter().zip(cold.iter()).enumerate() {
            assert_eq!(w, c, "warm walk node {i} differs from cold walk");
        }
        // The cache size is unchanged (no new inserts on a warm walk).
        assert_eq!(ctx.cache().unwrap().len(), 3);
    }

    #[test]
    fn cache_invalidated_by_stylesheet_mutation() {
        // After sheet.add(...), the generation bumps and the next compute must
        // recompute (cache cleared). The new rule's effect must show up.
        //
        // We use the one-shot compute_cached API directly because CascadeContext
        // borrows the sheet immutably for its whole lifetime — a real host
        // drops the context, mutates the sheet, then rebuilds it. The cache
        // here stands in for a cache the host carries across frames.
        let mut sheet = Stylesheet::new();
        sheet.add("Text", CssStyle::new().color(RC::Red), Origin::User).unwrap();

        let mut scratch = ComputeScratch::new();
        let mut cache = ComputeCache::new(8);
        let media = MediaContext::default();
        let node = OwnedNode::new("Text");

        let (text1, sig1) = sheet.compute_cached(&node, None, None, &media, &mut scratch, &mut cache);
        assert_eq!(text1.style.color, Some(Color::literal(RC::Red)));
        assert_eq!(cache.len(), 1);

        // Mutate the sheet — generation bumps, cache auto-invalidates on next access.
        sheet.add("Text", CssStyle::new().color(RC::Blue), Origin::User).unwrap();

        // Re-compute: the cache detects the gen mismatch and clears; the new
        // (later, same-specificity) rule wins → Blue.
        let (text2, sig2) = sheet.compute_cached(&node, None, None, &media, &mut scratch, &mut cache);
        assert_eq!(
            text2.style.color,
            Some(Color::literal(RC::Blue)),
            "mutation must invalidate the cache"
        );
        // The signature is the same (same node, same media, same parent), but
        // the cache was cleared by the gen mismatch and repopulated.
        assert_eq!(sig1, sig2);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_invalidated_by_tokens_mut() {
        // tokens_mut bumps gen, so a downstream var() resolution changes.
        let mut sheet = Stylesheet::with_tokens(
            crate::token::ThemeTokens::new().set("accent", Color::literal(RC::Red)),
        );
        sheet.add(".a", CssStyle::new().color(Color::var("accent")), Origin::User).unwrap();

        let mut scratch = ComputeScratch::new();
        let mut cache = ComputeCache::new(8);
        let media = MediaContext::default();
        let node = OwnedNode::new("Div").with_classes(["a"]);

        let (a1, _) = sheet.compute_cached(&node, None, None, &media, &mut scratch, &mut cache);
        assert_eq!(a1.style.color, Some(Color::literal(RC::Red)));

        // Mutate the token via tokens_mut — gen bumps.
        sheet.tokens_mut().insert("accent", Color::literal(RC::Blue));

        let (a2, _) = sheet.compute_cached(&node, None, None, &media, &mut scratch, &mut cache);
        assert_eq!(
            a2.style.color,
            Some(Color::literal(RC::Blue)),
            "tokens_mut must invalidate the cache so the var re-resolves"
        );
    }

    #[test]
    fn cache_invalidated_by_media_change() {
        // Media is part of the signature, so changing the active media context
        // produces different signatures and naturally recomputes.
        let sheet = Stylesheet::parse(
            "@media (min-width: 80) { Button { color: red; } }",
        )
        .unwrap();

        let mut ctx = CascadeContext::new(&sheet).with_cache(8).with_media(MediaContext {
            cols: 100,
            ..Default::default()
        });
        let big = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(big.style.color, Some(Color::literal(RC::Red)));
        ctx.leave();

        // Switch to a non-matching context: the signature differs (media is
        // folded in), so the cache misses and the rule no longer applies.
        ctx.set_media(MediaContext { cols: 40, ..Default::default() });
        let small = ctx.enter(&OwnedNode::new("Button"));
        assert_eq!(small.style.color, None);
    }

    #[test]
    fn cache_parent_dependency_different_parents() {
        // Two nodes with identical identity but different parents inherit
        // differently → different signatures → different results.
        let mut sheet = Stylesheet::new();
        sheet.add("Red", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("Blue", CssStyle::new().color(RC::Blue), Origin::User).unwrap();
        // Child has no color of its own — inherits.
        sheet.add("Child", CssStyle::new(), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet).with_cache(8);

        // Branch A: Red → Child
        let _red = ctx.enter(&OwnedNode::new("Red"));
        let child_a = ctx.enter(&OwnedNode::new("Child"));
        assert_eq!(child_a.style.color, Some(Color::literal(RC::Red)));
        ctx.leave();
        ctx.leave();

        // Branch B: Blue → Child
        let _blue = ctx.enter(&OwnedNode::new("Blue"));
        let child_b = ctx.enter(&OwnedNode::new("Child"));
        assert_eq!(
            child_b.style.color,
            Some(Color::literal(RC::Blue)),
            "identical Child node with a different parent must produce a different result"
        );
    }

    #[test]
    fn cache_works_with_combinator_sheet_descendant() {
        // A sheet with a descendant combinator (`Panel Text`) walked via
        // with_cache must still match — the cached-ancestors path is used.
        let mut sheet = Stylesheet::new();
        sheet.add("Panel Text", CssStyle::new().color(RC::Green), Origin::User).unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet).with_cache(8);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text.style.color, Some(Color::literal(RC::Green)));

        // Second walk — the cached result is served and still matches.
        ctx.leave();
        ctx.leave();
        ctx.leave();
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text2 = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text2.style.color, Some(Color::literal(RC::Green)));
        assert_eq!(text2, text, "warm cached walk == cold walk for combinators");
    }

    #[test]
    fn cache_works_with_combinator_sheet_child() {
        // Child combinator + cache.
        let mut sheet = Stylesheet::new();
        sheet.add("Panel > Text", CssStyle::new().color(RC::Blue), Origin::User).unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet).with_cache(8);
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(text.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn cache_works_with_sibling_combinator() {
        // `Item + Item` + cache: the sibling identities are folded into the
        // parent signature transitively... actually they are NOT directly in
        // the signature, so we assert this carefully: the cached-ancestors
        // path is used, and the rule applies on the cold walk. On a warm walk
        // with the SAME sibling structure, the result is stable.
        let mut sheet = Stylesheet::new();
        sheet.add("Item + Item", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        assert!(sheet.has_combinators());

        let mut ctx = CascadeContext::new(&sheet).with_cache(16);
        let _root = ctx.enter(&OwnedNode::new("Root"));

        let first = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(first.style.color, None);
        ctx.leave();
        let second = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(second.style.color, Some(Color::literal(RC::Red)));
        ctx.leave();
        ctx.leave();

        // Warm walk with the same structure.
        let _root = ctx.enter(&OwnedNode::new("Root"));
        let first2 = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(first2.style.color, None);
        ctx.leave();
        let second2 = ctx.enter(&OwnedNode::new("Item"));
        assert_eq!(second2.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn cache_off_context_behaves_identically() {
        // Regression: a CascadeContext WITHOUT with_cache is byte-for-byte
        // identical to the uncached baseline. We assert by comparing against
        // the manual compute() chain for a 3-level tree.
        let mut sheet = Stylesheet::new();
        sheet.add("Root", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("Panel", CssStyle::new().padding("1"), Origin::User).unwrap();
        sheet.add("Text", CssStyle::new().bold(), Origin::User).unwrap();

        // Context path (no cache).
        let mut ctx = CascadeContext::new(&sheet);
        let ctx_root = ctx.enter(&OwnedNode::new("Root"));
        let ctx_panel = ctx.enter(&OwnedNode::new("Panel"));
        let ctx_text = ctx.enter(&OwnedNode::new("Text"));

        // Manual threading.
        let man_root = sheet.compute(&OwnedNode::new("Root"), None);
        let man_panel = sheet.compute(&OwnedNode::new("Panel"), Some(&man_root));
        let man_text = sheet.compute(&OwnedNode::new("Text"), Some(&man_panel));

        assert_eq!(ctx_root, man_root);
        assert_eq!(ctx_panel, man_panel);
        assert_eq!(ctx_text, man_text);

        // No cache attached.
        assert!(ctx.cache().is_none());
    }

    #[test]
    fn cache_recomputes_correctly_after_mixed_tree_walks() {
        // Stress: walk a tree with siblings, leave, walk a different shape,
        // then re-walk the first. The cache must stay correct — every signature
        // is built fresh from the current ancestor chain, so identical subtrees
        // share entries.
        let mut sheet = Stylesheet::new();
        sheet.add("A", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("B", CssStyle::new().color(RC::Blue), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet).with_cache(32);

        // Walk A → B.
        let _ = ctx.enter(&OwnedNode::new("A"));
        let b1 = ctx.enter(&OwnedNode::new("B"));
        assert_eq!(b1.style.color, Some(Color::literal(RC::Blue)));
        ctx.leave();
        ctx.leave();

        // Walk B → A (different structure).
        let _ = ctx.enter(&OwnedNode::new("B"));
        let a1 = ctx.enter(&OwnedNode::new("A"));
        // A has its own color (Red), does not inherit Blue from B (color is
        // inheritable but A's rule sets Red explicitly).
        assert_eq!(a1.style.color, Some(Color::literal(RC::Red)));
        ctx.leave();
        ctx.leave();

        // Re-walk A → B — identical to the first, should be served from cache.
        let _ = ctx.enter(&OwnedNode::new("A"));
        let b2 = ctx.enter(&OwnedNode::new("B"));
        assert_eq!(b2.style.color, Some(Color::literal(RC::Blue)));
        assert_eq!(b2, b1, "re-walked subtree is identical to the first");
    }
}
