use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};

static LOGICAL: AtomicU64 = AtomicU64::new(0);

/// Returns an HLC timestamp string: "<wall_ms>-<logical>".
/// Note: This is a simplified HLC that provides local monotonicity.
/// It does not advance the logical counter from incoming remote timestamps
/// (that is Phase 2 work when networking is implemented).
pub fn now() -> String {
    let wall = Utc::now().timestamp_millis() as u64;
    let logical = LOGICAL.fetch_add(1, Ordering::SeqCst);
    format!("{wall:016x}-{logical:08x}")
}
