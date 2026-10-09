//! Lightweight Instant timing for debug builds and `--features perf`.
//!
//! Compiled out of release unless the `perf` feature is on. Debug builds
//! log only when `MYCAD_PERF` is set to a non-empty, non-`0` value so tests
//! stay quiet. Logs fire only for spans slower than 8 ms.

use std::time::Duration;
#[cfg(any(debug_assertions, feature = "perf"))]
use std::time::Instant;

#[cfg(any(debug_assertions, feature = "perf"))]
const LOG_PREFIX: &str = "[mycad-perf]";

// ------------------------------------------------------------
// Function: threshold_label
// Purpose: Map elapsed time to the coarsest exceeded budget.
// ------------------------------------------------------------
pub fn threshold_label(elapsed: Duration) -> Option<&'static str> {
    let ms = elapsed.as_secs_f64() * 1000.0;
    if ms > 100.0 {
        Some(">100ms")
    } else if ms > 50.0 {
        Some(">50ms")
    } else if ms > 16.0 {
        Some(">16ms")
    } else if ms > 8.0 {
        Some(">8ms")
    } else {
        None
    }
}

/// Fewest spare slots a freshly built collection keeps for edits.
pub const MIN_EDIT_HEADROOM: usize = 4096;

// ------------------------------------------------------------
// Function: edit_headroom
// Purpose: Spare capacity a freshly built index reserves on its
//          worker, so the first edit after opening a large drawing
//          does not reallocate or rehash the whole collection on
//          the UI thread.
// ------------------------------------------------------------
pub fn edit_headroom(len: usize) -> usize {
    (len / 16).max(MIN_EDIT_HEADROOM)
}

pub fn reserve_vec_headroom<T>(items: &mut Vec<T>) {
    items.reserve_exact(edit_headroom(items.len()));
}

pub fn reserve_map_headroom<K, V, S>(map: &mut std::collections::HashMap<K, V, S>)
where
    K: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
{
    map.reserve(edit_headroom(map.len()));
}

// ------------------------------------------------------------
// Function: enabled
// Purpose: Decide whether spans should record Instant samples.
// ------------------------------------------------------------
pub fn enabled() -> bool {
    #[cfg(feature = "perf")]
    {
        true
    }
    #[cfg(not(feature = "perf"))]
    {
        debug_env_enabled()
    }
}

#[cfg(all(debug_assertions, not(feature = "perf")))]
fn debug_env_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| match std::env::var("MYCAD_PERF") {
        Ok(value) => {
            let value = value.trim();
            !value.is_empty() && value != "0"
        }
        Err(_) => false,
    })
}

#[cfg(not(any(debug_assertions, feature = "perf")))]
fn debug_env_enabled() -> bool {
    false
}

#[cfg(any(debug_assertions, feature = "perf"))]
fn log_span(name: &str, elapsed: Duration) {
    let Some(label) = threshold_label(elapsed) else {
        return;
    };
    let ms = elapsed.as_secs_f64() * 1000.0;
    eprintln!("{LOG_PREFIX} {name} {ms:.2}ms ({label})");
}

// ------------------------------------------------------------
// Type: Span
// Purpose: RAII timer. Hold the value until the work finishes.
// ------------------------------------------------------------
#[cfg(any(debug_assertions, feature = "perf"))]
#[must_use = "the span stops measuring when dropped"]
pub struct Span {
    name: &'static str,
    start: Option<Instant>,
}

#[cfg(not(any(debug_assertions, feature = "perf")))]
#[must_use = "the span stops measuring when dropped"]
pub struct Span;

#[cfg(any(debug_assertions, feature = "perf"))]
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start.take() {
            log_span(self.name, start.elapsed());
        }
    }
}

// ------------------------------------------------------------
// Function: span
// Purpose: Start a named timer if instrumentation is active.
// ------------------------------------------------------------
#[cfg(any(debug_assertions, feature = "perf"))]
pub fn span(name: &'static str) -> Span {
    let start = if enabled() {
        Some(Instant::now())
    } else {
        None
    };
    Span { name, start }
}

#[cfg(not(any(debug_assertions, feature = "perf")))]
#[inline(always)]
pub fn span(_name: &'static str) -> Span {
    Span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_labels_match_frame_budgets() {
        assert_eq!(threshold_label(Duration::from_millis(7)), None);
        assert_eq!(threshold_label(Duration::from_micros(8001)), Some(">8ms"));
        assert_eq!(threshold_label(Duration::from_micros(16001)), Some(">16ms"));
        assert_eq!(threshold_label(Duration::from_micros(50001)), Some(">50ms"));
        assert_eq!(threshold_label(Duration::from_millis(101)), Some(">100ms"));
    }

    #[test]
    fn built_collections_keep_room_for_edits() {
        assert_eq!(edit_headroom(0), MIN_EDIT_HEADROOM);
        assert_eq!(edit_headroom(1_600_000), 100_000);
        let mut items: Vec<u32> = (0..10).collect();
        items.shrink_to_fit();
        reserve_vec_headroom(&mut items);
        assert!(items.capacity() >= 10 + MIN_EDIT_HEADROOM);
        let mut map: std::collections::HashMap<u32, u32> = (0..10).map(|i| (i, i)).collect();
        reserve_map_headroom(&mut map);
        assert!(map.capacity() >= 10 + MIN_EDIT_HEADROOM);
    }

    #[test]
    fn inactive_span_does_not_panic() {
        let _span = span("test_span");
    }
}
