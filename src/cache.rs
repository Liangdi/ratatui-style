//! An opt-in LRU cache for [`ComputedStyle`](crate::ComputedStyle) results.
//!
//! The cascade is pure: given the same `(node identity, ancestor chain, media
//! context, stylesheet generation)`, it always produces the same
//! `ComputedStyle`. A stable TUI tree re-renders frame after frame with no
//! inputs changed, so recomputing every node's style each frame is wasted work.
//! This module memoizes those results.
//!
//! The cache is **opt-in and off by default**. `CascadeContext::new` and the
//! one-shot `Stylesheet::compute` path are byte-for-byte identical in behavior
//! and overhead to the uncached baseline. Attach a cache via
//! [`CascadeContext::with_cache`](crate::CascadeContext::with_cache).
//!
//! # Correctness backbone
//!
//! Two invariants make the cache safe:
//!
//! 1. **The signature captures every input.** [`node_signature`] folds the
//!    node's selector-relevant identity (type, id, sorted classes, state bits,
//!    position), the PARENT's signature (which transitively captures the whole
//!    ancestor chain), the previous-sibling identities (for `+`/`~` sibling
//!    combinators), and the media context. Two computes with the same signature
//!    produce the same `ComputedStyle`.
//!
//! 2. **Stylesheet mutations auto-invalidate.** [`Stylesheet`](crate::Stylesheet)
//!    bumps its generation counter at the start of every mutation that can
//!    change compute output (`add`, `add_rule`, `extend`, `tokens_mut`). The
//!    cache detects a generation mismatch on every lookup/insert and clears
//!    itself entirely — so a stylesheet edit between two walks throws away
//!    every stale entry with no caller action.
//!
//! # Eviction policy
//!
//! Capacity is a hard bound. When the cache is full and a NEW key is inserted,
//! the OLDEST insertion is evicted (FIFO). This is **not** access-order LRU — a
//! `get` does not reorder. FIFO is simpler, has no `&mut self`-on-read
//! complications beyond the generation check, and is fine for the stable-tree
//! render loop where the working set is touched every frame anyway. Capacity 0
//! disables storage entirely (`get` always misses).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use crate::cascade::ComputedStyle;
use crate::media::MediaContext;
use crate::node::{Position, State};
use crate::selector::NodeIdentity;

/// An opt-in bounded cache for [`ComputedStyle`] results, keyed by an opaque
/// signature that captures (node identity, ancestor-chain signature, media
/// context, stylesheet generation).
///
/// See the [module docs](crate::cache) for the correctness invariants and the
/// eviction policy. Two computes with the same signature yield the same
/// `ComputedStyle`, so a hit short-circuits the cascade. The cache is NOT
/// access-order LRU — it is bounded FIFO (oldest insertion evicted when full).
///
/// `get` takes `&mut self` because a stylesheet generation mismatch (detected
/// on every access) clears the cache in place — see [`Self::check_generation`].
pub struct ComputeCache {
    entries: HashMap<u64, ComputedStyle>,
    order: VecDeque<u64>,
    capacity: usize,
    /// The [`Stylesheet::generation`](crate::Stylesheet::generation) this cache
    /// currently holds entries for. Any mismatch against the live generation
    /// clears the cache.
    generation: u64,
}

impl ComputeCache {
    /// Build a cache with the given hard capacity. `capacity == 0` disables
    /// storage: `get` always returns `None` and `insert` is a no-op.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
            generation: 0,
        }
    }

    /// If `current_gen` differs from the generation this cache was populated
    /// under, clear every entry and adopt the new generation. Called at the
    /// start of every `get`/`insert` so a stylesheet mutation between accesses
    /// auto-invalidates the whole cache.
    fn check_generation(&mut self, current_gen: u64) {
        if self.generation != current_gen {
            self.clear();
            self.generation = current_gen;
        }
    }

    /// Look up a cached result by signature. Returns an owned
    /// [`ComputedStyle`] clone on a hit (cheap — a post-resolution
    /// `ComputedStyle` holds no heap `Color::Var`).
    ///
    /// `&mut self` because [`check_generation`](Self::check_generation) may
    /// clear.
    pub fn get(&mut self, sig: u64, current_gen: u64) -> Option<ComputedStyle> {
        self.check_generation(current_gen);
        self.entries.get(&sig).cloned()
    }

    /// Insert a computed result under `sig`. If the key already exists, update
    /// its value and move it to the back of the eviction order (most-recently
    /// inserted). If at capacity and the key is new, evict the oldest
    /// insertion. A no-op when capacity is 0.
    pub fn insert(&mut self, sig: u64, value: ComputedStyle, current_gen: u64) {
        self.check_generation(current_gen);
        if self.capacity == 0 {
            return;
        }
        if let Some(existing) = self.entries.get_mut(&sig) {
            // Update in place; refresh its position in the order.
            *existing = value;
            if let Some(pos) = self.order.iter().position(|&k| k == sig) {
                self.order.remove(pos);
            }
            self.order.push_back(sig);
            return;
        }
        // New key. Evict the oldest insertion if at capacity.
        while self.entries.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            } else {
                break;
            }
        }
        self.entries.insert(sig, value);
        self.order.push_back(sig);
    }

    /// Drop every entry. Keeps the capacity for reuse.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds zero entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ComputeCache {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Fold every selector-relevant field of a node into a 64-bit signature.
///
/// The signature folds:
/// - **Node identity**: `type_name`, `id`, `classes` (sorted before hashing so
///   order is irrelevant — class match is set-membership), `state` bits, and
///   `position` (`index`, `sibling_count`, `parent_type`).
/// - **The parent's signature** (`parent_sig`): transitively captures the
///   entire ancestor chain, so descendant/child combinator and inheritance
///   dependencies are captured by a single 64-bit fold.
/// - **The previous-sibling identities** (`siblings`): the adjacent (`+`) and
///   general (`~`) sibling combinators match against these, so they must be in
///   the signature. Each sibling's selector-relevant fields are folded in
///   order. Empty slice for the no-sibling / one-shot path.
/// - **The media context**: `cols`, `rows`, `truecolor`, `no_color`.
///
/// Two nodes with identical `(identity, parent_sig, siblings, media)` produce
/// identical signatures (deterministic hashing); differing in any folded field
/// differs the signature with overwhelming probability.
///
/// Uses [`DefaultHasher`] (no new dependency). The exact hash value is an
/// implementation detail and MUST NOT be relied upon across builds — only
/// equality within a single run.
pub(crate) fn node_signature(
    node_id: &NodeIdentity,
    parent_sig: Option<u64>,
    siblings: &[NodeIdentity],
    media: &MediaContext,
) -> u64 {
    let mut h = DefaultHasher::new();

    // Marker so a re-ordering of fields never silently collides with an older
    // layout — bumped if the folded set ever changes.
    0xC5_C4_06_14u64.hash(&mut h);

    // Parent signature first: transitive ancestor chain.
    parent_sig.hash(&mut h);

    // Node identity — in a fixed order.
    node_id.type_name.hash(&mut h);
    node_id.id.hash(&mut h);

    // Classes are set-membership, so sort before hashing for order-independence.
    let mut sorted: Vec<&str> = node_id.classes.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.len().hash(&mut h);
    for c in sorted {
        c.hash(&mut h);
    }

    hash_state(&mut h, node_id.state);
    hash_position(&mut h, &node_id.position);

    // Previous siblings (in order — the adjacent combinator keys off the LAST
    // one, the general combinator off the whole list). Fold each one's
    // selector-relevant fields.
    siblings.len().hash(&mut h);
    for sib in siblings {
        sib.type_name.hash(&mut h);
        sib.id.hash(&mut h);
        let mut sib_classes: Vec<&str> = sib.classes.iter().map(String::as_str).collect();
        sib_classes.sort_unstable();
        sib_classes.len().hash(&mut h);
        for c in sib_classes {
            c.hash(&mut h);
        }
        hash_state(&mut h, sib.state);
        hash_position(&mut h, &sib.position);
    }

    // Media context bytes.
    media.cols.hash(&mut h);
    media.rows.hash(&mut h);
    media.truecolor.hash(&mut h);
    media.no_color.hash(&mut h);

    h.finish()
}

fn hash_state(h: &mut DefaultHasher, state: State) {
    state.focus.hash(h);
    state.hover.hash(h);
    state.disabled.hash(h);
    state.checked.hash(h);
    state.active.hash(h);
}

fn hash_position(h: &mut DefaultHasher, pos: &Position) {
    pos.index.hash(h);
    pos.sibling_count.hash(h);
    pos.parent_type.hash(h);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::ComputedStyle;
    use crate::node::Position;
    use crate::selector::NodeIdentity;
    use ratatui::style::Color as RC;

    // ---- signature tests ---------------------------------------------------

    fn nid(type_name: &str) -> NodeIdentity {
        NodeIdentity {
            type_name: type_name.to_string(),
            id: None,
            classes: Vec::new(),
            state: State::empty(),
            position: Position::default(),
        }
    }

    fn nid_with_classes(type_name: &str, classes: &[&str]) -> NodeIdentity {
        NodeIdentity {
            type_name: type_name.to_string(),
            id: None,
            classes: classes.iter().map(|s| s.to_string()).collect(),
            state: State::empty(),
            position: Position::default(),
        }
    }

    fn default_media() -> MediaContext {
        MediaContext::default()
    }

    #[test]
    fn signature_identical_inputs_match() {
        let a = nid("Button");
        let b = nid("Button");
        let m = default_media();
        assert_eq!(
            node_signature(&a, None, &[], &m),
            node_signature(&b, None, &[], &m)
        );
    }

    #[test]
    fn signature_differs_on_type() {
        let m = default_media();
        let a = nid("Button");
        let b = nid("Text");
        assert_ne!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_differs_on_id() {
        let m = default_media();
        let mut a = nid("Button");
        a.id = Some("save".to_string());
        let b = nid("Button");
        assert_ne!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_classes_are_order_independent() {
        let m = default_media();
        let a = nid_with_classes("Button", &["primary", "large"]);
        let b = nid_with_classes("Button", &["large", "primary"]);
        assert_eq!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_differs_on_classes() {
        let m = default_media();
        let a = nid_with_classes("Button", &["primary"]);
        let b = nid_with_classes("Button", &["secondary"]);
        assert_ne!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_differs_on_state() {
        let m = default_media();
        let mut a = nid("Button");
        a.state = State::focus();
        let b = nid("Button");
        assert_ne!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_differs_on_position() {
        let m = default_media();
        let mut a = nid("Item");
        a.position = Position::new(0, 3);
        let mut b = nid("Item");
        b.position = Position::new(1, 3);
        assert_ne!(node_signature(&a, None, &[], &m), node_signature(&b, None, &[], &m));
    }

    #[test]
    fn signature_differs_on_parent_sig() {
        let m = default_media();
        let n = nid("Text");
        let s_none = node_signature(&n, None, &[], &m);
        let s_some = node_signature(&n, Some(42), &[], &m);
        assert_ne!(s_none, s_some);
    }

    #[test]
    fn signature_differs_on_media() {
        let n = nid("Button");
        let m1 = MediaContext { cols: 80, ..Default::default() };
        let m2 = MediaContext { cols: 100, ..Default::default() };
        assert_ne!(node_signature(&n, None, &[], &m1), node_signature(&n, None, &[], &m2));

        // truecolor flag
        let mt = MediaContext { truecolor: true, ..Default::default() };
        let mf = MediaContext { truecolor: false, ..Default::default() };
        assert_ne!(node_signature(&n, None, &[], &mt), node_signature(&n, None, &[], &mf));

        // no_color flag
        let mc = MediaContext { no_color: true, ..Default::default() };
        let mn = MediaContext { no_color: false, ..Default::default() };
        assert_ne!(node_signature(&n, None, &[], &mc), node_signature(&n, None, &[], &mn));
    }

    #[test]
    fn signature_transitively_captures_parent() {
        // Two Text nodes with identical identity but different parent
        // signatures get different signatures.
        let m = default_media();
        let n = nid("Text");
        let s1 = node_signature(&n, Some(111), &[], &m);
        let s2 = node_signature(&n, Some(222), &[], &m);
        assert_ne!(s1, s2);
    }

    #[test]
    fn signature_differs_on_siblings() {
        // A node with no previous siblings vs the same node with one previous
        // sibling must get different signatures — the adjacent (`+`) and
        // general (`~`) sibling combinators depend on the previous-sibling list.
        let m = default_media();
        let n = nid("Item");
        let no_sibs = node_signature(&n, None, &[], &m);
        let with_sib = node_signature(&n, None, std::slice::from_ref(&nid("Item")), &m);
        assert_ne!(no_sibs, with_sib);

        // Different sibling content → different signature.
        let with_other = node_signature(&n, None, std::slice::from_ref(&nid("Other")), &m);
        assert_ne!(with_sib, with_other);
    }

    // ---- ComputeCache tests ------------------------------------------------

    fn cs(c: RC) -> ComputedStyle {
        ComputedStyle::new(crate::style::CssStyle::new().color(c))
    }

    #[test]
    fn cache_get_miss_on_empty() {
        let mut cache = ComputeCache::new(8);
        assert!(cache.get(1, 0).is_none());
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cache_insert_then_get_hit() {
        let mut cache = ComputeCache::new(8);
        let val = cs(RC::Red);
        cache.insert(1, val.clone(), 0);
        let got = cache.get(1, 0).expect("hit");
        assert_eq!(got, val);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_capacity_evicts_oldest() {
        let mut cache = ComputeCache::new(2);
        cache.insert(10, cs(RC::Red), 0);
        cache.insert(20, cs(RC::Blue), 0);
        // At capacity. Inserting a new key (30) evicts the oldest (10).
        cache.insert(30, cs(RC::Green), 0);
        assert!(cache.get(10, 0).is_none(), "oldest evicted");
        assert!(cache.get(20, 0).is_some(), "second still present");
        assert!(cache.get(30, 0).is_some(), "newest present");
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_update_existing_does_not_evict() {
        let mut cache = ComputeCache::new(2);
        cache.insert(10, cs(RC::Red), 0);
        cache.insert(20, cs(RC::Blue), 0);
        // Update key 10 in place — no eviction.
        cache.insert(10, cs(RC::Yellow), 0);
        assert_eq!(cache.len(), 2);
        let got = cache.get(10, 0).expect("hit");
        assert_eq!(got.style.color, Some(crate::color::Color::literal(RC::Yellow)));
        assert!(cache.get(20, 0).is_some());
    }

    #[test]
    fn cache_generation_mismatch_clears_on_get() {
        let mut cache = ComputeCache::new(8);
        cache.insert(1, cs(RC::Red), 0);
        assert_eq!(cache.len(), 1);
        // get under a different generation: clears, returns None.
        let got = cache.get(1, 1);
        assert!(got.is_none());
        assert!(cache.is_empty(), "cache cleared by gen mismatch");
    }

    #[test]
    fn cache_generation_mismatch_clears_on_insert() {
        let mut cache = ComputeCache::new(8);
        cache.insert(1, cs(RC::Red), 0);
        assert_eq!(cache.len(), 1);
        // Insert under a different generation: clears, then inserts the new key.
        cache.insert(2, cs(RC::Blue), 1);
        assert_eq!(cache.len(), 1, "old entry cleared, only the new one remains");
        // The old key is gone.
        assert!(cache.get(1, 1).is_none());
        // The new one is present.
        assert!(cache.get(2, 1).is_some());
    }

    #[test]
    fn cache_capacity_zero_never_stores() {
        let mut cache = ComputeCache::new(0);
        cache.insert(1, cs(RC::Red), 0);
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
        assert!(cache.get(1, 0).is_none());
    }

    #[test]
    fn cache_clear_drops_entries() {
        let mut cache = ComputeCache::new(8);
        cache.insert(1, cs(RC::Red), 0);
        cache.insert(2, cs(RC::Blue), 0);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1, 0).is_none());
    }

    #[test]
    fn cache_stable_across_same_gen_lookups() {
        // Many gets/inserts under the same generation never clear.
        let mut cache = ComputeCache::new(4);
        for i in 0u64..4 {
            cache.insert(i, cs(RC::Red), 0);
        }
        for i in 0u64..4 {
            assert!(cache.get(i, 0).is_some());
        }
        assert_eq!(cache.len(), 4);
    }
}
