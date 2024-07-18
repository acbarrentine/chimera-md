use std::time::Instant;
use axum::http::header::HeaderMap;

use crate::SERVER_TIMING;

pub struct PerfTimer {
    prev_time: Instant,
}

impl PerfTimer {
    pub fn new() -> Self {
        PerfTimer {
            prev_time: Instant::now(),
        }
    }

    pub fn sample(&mut self, event: &'static str, headers: &mut HeaderMap) {
        if cfg!(feature = "detailed-timing") {
            let now = Instant::now();
            let elapsed = now.duration_since(self.prev_time).as_micros() as f64 / 1000.0;
            let header = format!("{event}; dur={}", elapsed);
            tracing::trace!(" - {header}");
            if let Ok(hval) = axum::http::HeaderValue::from_str(header.as_str()) {
                headers.append(SERVER_TIMING, hval);
            }
            self.prev_time = now;
        }
    }
}
