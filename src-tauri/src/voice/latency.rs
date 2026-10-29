//! Where the time goes in one voice turn, from letting go of the hotkey to hearing Aloe. Each
//! step is logged once per turn as `[voice] latency <step> +<ms>`, measured from the release.

use std::sync::Mutex;
use std::time::Instant;

use crate::config::debug_log;

static TURN: Mutex<Option<(Instant, Vec<&'static str>)>> = Mutex::new(None);

/// Listening ended: the clock starts.
pub fn start() {
    *TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), Vec::new()));
    debug_log("voice", "latency", "released +0ms");
}

/// Logs `step` the first time it happens this turn; later calls are ignored.
pub fn mark(step: &'static str) {
    let mut turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some((started, seen)) = turn.as_mut() else { return };
    if seen.contains(&step) {
        return;
    }
    seen.push(step);
    debug_log("voice", "latency", format!("{step} +{}ms", started.elapsed().as_millis()));
}
