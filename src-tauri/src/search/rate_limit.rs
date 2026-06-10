//! Per-source sliding-window rate limiter. Sources can't bypass it: the only
//! http client they receive acquires the limiter before every request.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    hits: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn per_minute(max: u32) -> Self {
        RateLimiter {
            max_per_window: max.max(1),
            window: Duration::from_secs(60),
            hits: Mutex::new(VecDeque::new()),
        }
    }

    /// waits until a slot is free, then records the hit
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut hits = self.hits.lock().await;
                let now = Instant::now();
                while let Some(front) = hits.front() {
                    if now.duration_since(*front) >= self.window {
                        hits.pop_front();
                    } else {
                        break;
                    }
                }
                if (hits.len() as u32) < self.max_per_window {
                    hits.push_back(now);
                    None
                } else {
                    // sleep until the oldest hit leaves the window
                    Some(self.window - now.duration_since(*hits.front().expect("non-empty")))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn blocks_after_window_is_full() {
        let rl = RateLimiter::per_minute(2);
        rl.acquire().await;
        rl.acquire().await;

        let start = Instant::now();
        rl.acquire().await; // must wait ~60s in virtual time
        assert!(start.elapsed() >= Duration::from_secs(59));
    }
}
