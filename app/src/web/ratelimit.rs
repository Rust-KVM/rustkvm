use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use salvo::prelude::*;

const MAX_ATTEMPTS: u32 = 5;
const BASE_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_BACKOFF_LEVEL: u32 = 3;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug)]
struct Entry {
    attempts: u32,
    window_start: Instant,
    backoff_level: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Default)]
struct State {
    entries: HashMap<IpAddr, Entry>,
    last_cleanup: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    state: Mutex<State>,
}

fn window_duration(backoff_level: u32) -> Duration {
    let level = backoff_level.min(MAX_BACKOFF_LEVEL);
    BASE_WINDOW * (1u32 << level)
}

impl RateLimiter {
    pub fn new() -> Self {
        Self { state: Mutex::new(State::default()) }
    }

    fn maybe_cleanup(state: &mut State, now: Instant) {
        let due = match state.last_cleanup {
            None => true,
            Some(prev) => now.duration_since(prev) >= CLEANUP_INTERVAL,
        };
        if !due {
            return;
        }
        state.last_cleanup = Some(now);
        state.entries.retain(|_, entry| {
            let locked = entry.locked_until.is_some_and(|t| now < t);
            let win = window_duration(entry.backoff_level);
            locked || now.duration_since(entry.window_start) <= win
        });
    }

    pub fn is_allowed(&self, ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        let mut state = self.state.lock();
        Self::maybe_cleanup(&mut state, now);

        let Some(entry) = state.entries.get(&ip) else {
            return Ok(());
        };

        if let Some(locked_until) = entry.locked_until
            && now < locked_until
        {
            let retry_after = (locked_until - now).as_secs() + 1;
            return Err(retry_after);
        }

        let win = window_duration(entry.backoff_level);
        if now.duration_since(entry.window_start) > win {
            return Ok(());
        }

        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut state = self.state.lock();
        Self::maybe_cleanup(&mut state, now);

        let entry = state.entries.entry(ip).or_insert(Entry {
            attempts: 0,
            window_start: now,
            backoff_level: 0,
            locked_until: None,
        });

        let win = window_duration(entry.backoff_level);
        if now.duration_since(entry.window_start) > win {
            if entry.backoff_level > 0 || entry.attempts >= MAX_ATTEMPTS {
                entry.backoff_level = (entry.backoff_level + 1).min(MAX_BACKOFF_LEVEL);
            }
            entry.attempts = 1;
            entry.window_start = now;
            entry.locked_until = None;
            return;
        }

        entry.attempts += 1;
        if entry.attempts >= MAX_ATTEMPTS {
            let lock_duration = window_duration(entry.backoff_level);
            entry.locked_until = Some(now + lock_duration);
        }
    }

    pub fn record_success(&self, ip: IpAddr) {
        let mut state = self.state.lock();
        state.entries.remove(&ip);
    }
}

pub static PASSWORD_RATE_LIMITER: Lazy<RateLimiter> = Lazy::new(RateLimiter::new);

pub fn client_ip(req: &Request) -> Option<IpAddr> {
    if let Some(xff) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
        && let Ok(addr) = first.trim().parse::<IpAddr>()
    {
        return Some(addr);
    }
    if let Some(real) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok())
        && let Ok(addr) = real.trim().parse::<IpAddr>()
    {
        return Some(addr);
    }
    req.remote_addr().ip()
}

pub fn too_many_requests_error(res: &mut Response, retry_after_secs: u64) -> StatusError {
    if let Ok(hv) = retry_after_secs.to_string().parse() {
        res.headers_mut().insert("Retry-After", hv);
    }
    StatusError::too_many_requests().brief("Too many failed attempts. Please try again later.")
}
