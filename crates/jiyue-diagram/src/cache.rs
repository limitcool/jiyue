use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::DiagramError;
use crate::render::{RenderedDiagram, render_mermaid};
use crate::theme::DiagramTheme;

type Key = [u8; 32];

#[derive(Clone)]
enum Entry {
    Ok(Arc<RenderedDiagram>),
    /// Deterministic failures are cached so a broken block is not re-parsed on every frame.
    Err(DiagramError),
}

struct Slot {
    entry: Entry,
    cost: usize,
}

#[derive(Default)]
struct State {
    slots: HashMap<Key, Slot>,
    /// Least recently used at the front.
    order: VecDeque<Key>,
    bytes: usize,
}

/// Bounded LRU cache of rendered diagrams, keyed by `blake3(source, theme)`.
///
/// Successful renders and deterministic errors (syntax, unsupported, too large)
/// are cached. Timeouts and internal errors are not, so they are retried. The
/// cache is thread-safe and can be shared behind an `Arc`.
pub struct DiagramCache {
    max_bytes: usize,
    state: Mutex<State>,
}

impl DiagramCache {
    /// Creates a cache that keeps at most `max_bytes` of SVG (plus small overhead per entry).
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            state: Mutex::new(State::default()),
        }
    }

    /// Returns the cached render for `source` and `theme`, rendering it on a miss.
    pub fn get_or_render(
        &self,
        source: &str,
        theme: &DiagramTheme,
    ) -> Result<Arc<RenderedDiagram>, DiagramError> {
        let key = cache_key(source, theme);
        if let Some(entry) = self.lookup(&key) {
            return match entry {
                Entry::Ok(diagram) => Ok(diagram),
                Entry::Err(err) => Err(err),
            };
        }
        // Rendering happens without holding the lock so other callers are not blocked.
        match render_mermaid(source, theme) {
            Ok(diagram) => {
                let shared = Arc::new(diagram);
                let cost = shared.svg.len() + 128;
                self.insert(key, Entry::Ok(Arc::clone(&shared)), cost);
                Ok(shared)
            }
            Err(err) => {
                if is_deterministic(&err) {
                    let cost = err.to_string().len() + 128;
                    self.insert(key, Entry::Err(err.clone()), cost);
                }
                Err(err)
            }
        }
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.guard().slots.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Approximate bytes held by cached entries.
    pub fn bytes(&self) -> usize {
        self.guard().bytes
    }

    /// Removes every entry.
    pub fn clear(&self) {
        let mut state = self.guard();
        state.slots.clear();
        state.order.clear();
        state.bytes = 0;
    }

    fn lookup(&self, key: &Key) -> Option<Entry> {
        let mut state = self.guard();
        let entry = state.slots.get(key)?.entry.clone();
        touch(&mut state.order, key);
        Some(entry)
    }

    fn insert(&self, key: Key, entry: Entry, cost: usize) {
        if cost > self.max_bytes {
            // Too big to ever fit; caching it would evict everything else for nothing.
            return;
        }
        let mut state = self.guard();
        if let Some(old) = state.slots.remove(&key) {
            state.bytes = state.bytes.saturating_sub(old.cost);
            state.order.retain(|k| k != &key);
        }
        while state.bytes + cost > self.max_bytes {
            let Some(victim) = state.order.pop_front() else {
                break;
            };
            if let Some(old) = state.slots.remove(&victim) {
                state.bytes = state.bytes.saturating_sub(old.cost);
            }
        }
        state.bytes += cost;
        state.order.push_back(key);
        state.slots.insert(key, Slot { entry, cost });
    }

    fn guard(&self) -> MutexGuard<'_, State> {
        // A poisoned lock only means another thread panicked while holding it; the
        // data is still consistent for our purposes, so keep using it.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn touch(order: &mut VecDeque<Key>, key: &Key) {
    if let Some(pos) = order.iter().position(|k| k == key) {
        order.remove(pos);
    }
    order.push_back(*key);
}

fn is_deterministic(err: &DiagramError) -> bool {
    matches!(
        err,
        DiagramError::Parse { .. } | DiagramError::Unsupported(_) | DiagramError::TooLarge
    )
}

fn cache_key(source: &str, theme: &DiagramTheme) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(source.len() as u64).to_le_bytes());
    hasher.update(source.as_bytes());
    hasher.update(theme.fingerprint().as_bytes());
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "flowchart LR\n A[开始] --> B[End]";

    #[test]
    fn second_lookup_is_a_hit_and_returns_same_svg() {
        let cache = DiagramCache::new(1024 * 1024);
        let theme = DiagramTheme::light();
        let a = cache
            .get_or_render(SRC, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cache.len(), 1);
        let b = cache
            .get_or_render(SRC, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cache.len(), 1);
        assert!(
            Arc::ptr_eq(&a, &b),
            "second call should reuse the cached Arc"
        );
    }

    #[test]
    fn different_theme_is_a_different_entry() {
        let cache = DiagramCache::new(1024 * 1024);
        let a = cache
            .get_or_render(SRC, &DiagramTheme::light())
            .unwrap_or_else(|e| panic!("{e}"));
        let b = cache
            .get_or_render(SRC, &DiagramTheme::dark())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cache.len(), 2);
        assert_ne!(a.svg, b.svg);
    }

    #[test]
    fn parse_errors_are_cached_but_not_rendered_twice_visibly() {
        let cache = DiagramCache::new(1024 * 1024);
        let theme = DiagramTheme::light();
        let first = cache.get_or_render("nonsense here", &theme);
        let second = cache.get_or_render("nonsense here", &theme);
        assert!(first.is_err());
        assert_eq!(first.err(), second.err());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn eviction_keeps_total_bytes_within_budget() {
        // Each render costs its SVG length plus 128; a budget of ~2 renders forces eviction.
        let probe = DiagramCache::new(usize::MAX);
        let one = probe
            .get_or_render(SRC, &DiagramTheme::light())
            .unwrap_or_else(|e| panic!("{e}"));
        let cost = one.svg.len() + 128;
        let cache = DiagramCache::new(cost * 2 + 10);
        let theme = DiagramTheme::light();
        for i in 0..5 {
            let src = format!("flowchart LR\n A{i}[节点{i}] --> B{i}[End]");
            cache
                .get_or_render(&src, &theme)
                .unwrap_or_else(|e| panic!("{e}"));
            assert!(cache.bytes() <= cost * 2 + 10);
        }
        assert!(cache.len() <= 2);
    }

    #[test]
    fn least_recently_used_entry_is_evicted_first() {
        let probe = DiagramCache::new(usize::MAX);
        let theme = DiagramTheme::light();
        let a_src = "flowchart LR\n A[a] --> B[b]";
        let b_src = "flowchart LR\n C[c] --> D[d]";
        let c_src = "flowchart LR\n E[e] --> F[f]";
        let cost = probe
            .get_or_render(a_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"))
            .svg
            .len()
            + 128;
        let cache = DiagramCache::new(cost * 2 + 10);
        let a1 = cache
            .get_or_render(a_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        cache
            .get_or_render(b_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        // touch A so B becomes the LRU entry
        let a2 = cache
            .get_or_render(a_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(Arc::ptr_eq(&a1, &a2));
        cache
            .get_or_render(c_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        // A must still be cached (same Arc), B must have been evicted (fresh Arc on re-render)
        let a3 = cache
            .get_or_render(a_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(Arc::ptr_eq(&a1, &a3), "A should have survived eviction");
        let b2 = cache
            .get_or_render(b_src, &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(b2.width > 0.0);
    }

    #[test]
    fn oversized_entries_are_not_cached() {
        let cache = DiagramCache::new(16);
        let r = cache.get_or_render(SRC, &DiagramTheme::light());
        assert!(r.is_ok());
        assert!(cache.is_empty());
    }

    #[test]
    fn clear_empties_cache() {
        let cache = DiagramCache::new(1024 * 1024);
        cache
            .get_or_render(SRC, &DiagramTheme::light())
            .unwrap_or_else(|e| panic!("{e}"));
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn key_depends_on_source_and_theme() {
        let t = DiagramTheme::light();
        assert_eq!(cache_key(SRC, &t), cache_key(SRC, &t));
        assert_ne!(cache_key(SRC, &t), cache_key("flowchart LR\n A-->C", &t));
        assert_ne!(cache_key(SRC, &t), cache_key(SRC, &DiagramTheme::dark()));
    }

    #[test]
    fn cache_is_shareable_across_threads() {
        let cache = Arc::new(DiagramCache::new(4 * 1024 * 1024));
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let c = Arc::clone(&cache);
                std::thread::spawn(move || c.get_or_render(SRC, &DiagramTheme::light()).is_ok())
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap_or(false));
        }
        assert_eq!(cache.len(), 1);
    }
}
