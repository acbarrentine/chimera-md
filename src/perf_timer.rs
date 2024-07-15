use std::{path::Path, time::Instant};

pub struct PerfTimer {
    samples: Vec<(&'static str, Instant)>,
}

impl PerfTimer {
    pub fn new() -> Self {
        PerfTimer {
            samples: vec![("start", Instant::now())],
        }
    }

    pub fn add_sample(&mut self, event: &'static str) {
        self.samples.push((event, Instant::now()))
    }

    pub fn report(self, path: &Path) {
        if self.samples.len() < 2 {
            return;
        }
        tracing::info!("Completed {}", path.to_string_lossy());
        let (_, mut prev_time) = self.samples[0];
        for (event, time) in self.samples.into_iter().skip(1) {
            tracing::info!(" - {event} took {} µs", time.duration_since(prev_time).as_micros());
            prev_time = time;
        }
    }
}
