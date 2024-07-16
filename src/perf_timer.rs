use std::time::Instant;

pub struct PerfTimer {
    prev_time: Instant,
}

impl PerfTimer {
    pub fn new() -> Self {
        PerfTimer {
            prev_time: Instant::now(),
        }
    }

    pub fn sample(&mut self, event: &'static str) {
        if cfg!(feature = "detailed-timing") {
            let now = Instant::now();
            tracing::info!(" - {event} took {} µs", now.duration_since(self.prev_time).as_micros());
            self.prev_time = now;
        }
    }
}
