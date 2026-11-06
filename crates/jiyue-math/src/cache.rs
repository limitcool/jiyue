//! Content-addressed LRU cache for rendered formulas.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::{MathError, MathTheme, RenderedMath, render_math};

/// Cache key type: BLAKE3 digest of (latex, display, theme).
pub type Key = [u8; 32];

/// Least-recently-used cache of [`RenderedMath`], keyed by BLAKE3(latex, display, theme).
///
/// Failed renders are not cached, so a transient timeout is retried next time.
#[derive(Debug)]
pub struct MathCache {
    capacity: usize,
    map: HashMap<Key, Arc<RenderedMath>>,
    /// Most recently used at the back.
    order: VecDeque<Key>,
}

impl MathCache {
    /// Create a cache holding at most `capacity` formulas (minimum 1).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Digest identifying one rendering request. Length-prefixed so that the
    /// fields cannot be confused with each other.
    pub fn key(latex: &str, display: bool, theme: &MathTheme) -> Key {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&(latex.len() as u64).to_le_bytes());
        hasher.update(latex.as_bytes());
        hasher.update(&[u8::from(display)]);
        hasher.update(&(theme.color.len() as u64).to_le_bytes());
        hasher.update(theme.color.as_bytes());
        hasher.update(&theme.font_size.to_bits().to_le_bytes());
        *hasher.finalize().as_bytes()
    }

    /// Return the cached rendering, or render and insert it.
    pub fn get_or_render(
        &mut self,
        latex: &str,
        display: bool,
        theme: &MathTheme,
    ) -> Result<Arc<RenderedMath>, MathError> {
        let key = Self::key(latex, display, theme);
        if let Some(hit) = self.map.get(&key).cloned() {
            self.touch(&key);
            return Ok(hit);
        }
        let rendered = Arc::new(render_math(latex, display, theme)?);
        self.insert(key, Arc::clone(&rendered));
        Ok(rendered)
    }

    /// Number of cached formulas.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Drop every entry.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn insert(&mut self, key: Key, value: Arc<RenderedMath>) {
        if self.map.insert(key, value).is_none() {
            self.order.push_back(key);
        } else {
            self.touch(&key);
        }
        while self.map.len() > self.capacity {
            match self.order.pop_front() {
                Some(old) => {
                    self.map.remove(&old);
                }
                None => break,
            }
        }
    }

    fn touch(&mut self, key: &Key) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
            self.order.push_back(*key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(color: &str) -> MathTheme {
        MathTheme {
            color: color.to_string(),
            font_size: 16.0,
        }
    }

    #[test]
    fn key_separates_every_input() {
        let base = MathCache::key("x", false, &theme("#000000"));
        assert_ne!(base, MathCache::key("x", true, &theme("#000000")));
        assert_ne!(base, MathCache::key("y", false, &theme("#000000")));
        assert_ne!(base, MathCache::key("x", false, &theme("#111111")));
        let bigger = MathTheme {
            color: "#000000".into(),
            font_size: 17.0,
        };
        assert_ne!(base, MathCache::key("x", false, &bigger));
        assert_eq!(base, MathCache::key("x", false, &theme("#000000")));
    }

    #[test]
    fn length_prefix_prevents_field_confusion() {
        let a = MathCache::key("ab", false, &theme("#000000"));
        let b = MathCache::key("a", false, &theme("b#000000"));
        assert_ne!(a, b);
    }

    #[test]
    fn hit_returns_same_arc_and_miss_renders() {
        let mut cache = MathCache::new(8);
        let t = theme("#000000");
        let first = cache.get_or_render("x^2", false, &t).expect("render");
        let second = cache.get_or_render("x^2", false, &t).expect("hit");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn errors_are_not_cached() {
        let mut cache = MathCache::new(8);
        assert!(
            cache
                .get_or_render(r"\frac{", false, &theme("#000000"))
                .is_err()
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut cache = MathCache::new(2);
        let t = theme("#000000");
        cache.get_or_render("a", false, &t).expect("a");
        cache.get_or_render("b", false, &t).expect("b");
        // Touch "a" so "b" becomes the eviction candidate.
        cache.get_or_render("a", false, &t).expect("a again");
        cache.get_or_render("c", false, &t).expect("c");
        assert_eq!(cache.len(), 2);
        assert!(cache.map.contains_key(&MathCache::key("a", false, &t)));
        assert!(!cache.map.contains_key(&MathCache::key("b", false, &t)));
        assert!(cache.map.contains_key(&MathCache::key("c", false, &t)));
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn zero_capacity_is_clamped() {
        let mut cache = MathCache::new(0);
        cache
            .get_or_render("x", false, &theme("#000000"))
            .expect("x");
        assert_eq!(cache.len(), 1);
    }
}
