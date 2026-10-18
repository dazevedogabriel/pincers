//! Global speed limiter shared by every connection of a download
//! (the equivalent of IDM's `CSpeedLimiter`). A token bucket behind a fair
//! (FIFO) async mutex, so connections take turns instead of the fastest ones
//! starving the rest.

use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct SpeedLimiter {
    /// Bytes per second; `None` = unlimited.
    rate: Option<u64>,
    state: Mutex<(f64, Instant)>,
}

impl SpeedLimiter {
    pub fn new(rate: Option<u64>) -> Self {
        Self { rate, state: Mutex::new((0.0, Instant::now())) }
    }

    /// Accounts for `n` received bytes, sleeping as long as needed to stay
    /// under the configured rate. Waiters are served in arrival order.
    pub async fn consume(&self, n: usize) {
        let Some(rate) = self.rate else { return };
        let rate = rate as f64;
        let mut st = self.state.lock().await;
        let now = Instant::now();
        // Refill, capping the burst at a tenth of a second worth of data so a
        // single connection cannot grab a large share at once.
        st.0 = (st.0 + now.duration_since(st.1).as_secs_f64() * rate).min(rate / 10.0);
        st.1 = now;
        st.0 -= n as f64;
        if st.0 < 0.0 {
            // Sleep while holding the lock: the next connection in the queue
            // goes only after this one has paid for its bytes.
            tokio::time::sleep(Duration::from_secs_f64(-st.0 / rate)).await;
            st.0 = 0.0;
            st.1 = Instant::now();
        }
    }
}
