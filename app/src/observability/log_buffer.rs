use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub const CAPACITY: usize = 1000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

fn ring() -> &'static Mutex<VecDeque<LogEntry>> {
    static RING: OnceLock<Mutex<VecDeque<LogEntry>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

/// Records every event that passes the live filter, so the buffer always
/// matches what the reloadable log level lets through.
pub struct LogBufferLayer;

pub fn layer() -> LogBufferLayer {
    LogBufferLayer
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let mut message = visitor.message;
        if !visitor.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&visitor.fields);
        }
        let entry = LogEntry {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            level: event.metadata().level().as_str(),
            target: event.metadata().target().to_owned(),
            message,
        };
        let mut ring = ring().lock();
        if ring.len() == CAPACITY {
            ring.pop_front();
        }
        ring.push_back(entry);
    }
}

fn severity(level: &str) -> u8 {
    match level {
        "ERROR" => 4,
        "WARN" => 3,
        "INFO" => 2,
        "DEBUG" => 1,
        _ => 0,
    }
}

pub fn parse_level(level: &str) -> Option<Level> {
    level.parse().ok()
}

/// Newest `limit` entries at or above `min_level` whose message or target
/// contains `contains`, oldest first.
pub fn recent(limit: usize, min_level: Option<Level>, contains: Option<&str>) -> Vec<LogEntry> {
    let floor = min_level.map_or(0, |l| severity(l.as_str()));
    let ring = ring().lock();
    let mut matched: Vec<LogEntry> = ring
        .iter()
        .rev()
        .filter(|e| severity(e.level) >= floor)
        .filter(|e| {
            contains.is_none_or(|needle| e.message.contains(needle) || e.target.contains(needle))
        })
        .take(limit)
        .cloned()
        .collect();
    matched.reverse();
    matched
}
