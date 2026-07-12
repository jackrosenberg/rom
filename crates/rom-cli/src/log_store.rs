use std::{
  collections::VecDeque,
  sync::{Condvar, Mutex},
};

const MAX_PENDING_LOG_RECORDS: usize = 1_024;
const MAX_LOG_RECORD_BYTES: usize = 64 * 1024;
const MAX_PENDING_LOG_BYTES: usize = 2 * 1024 * 1024;
const TRUNCATION_MARKER: &str = "… [log truncated]";

#[derive(Default)]
struct LogStoreState {
  pending:       VecDeque<String>,
  pending_bytes: usize,
  closed:        bool,
}

/// Bounded hand-off between the stderr parser and renderer.
///
/// Back-pressure is intentional: it preserves every record without allowing a
/// fast evaluator to grow a second, unbounded copy of the log in memory.
#[derive(Default)]
pub struct LogStore {
  state: Mutex<LogStoreState>,
  space: Condvar,
}

impl LogStore {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn push(&self, line: String) {
    let line = truncate_record(line);
    let bytes = line.len();
    let mut state = self.state.lock().unwrap();
    while (state.pending.len() >= MAX_PENDING_LOG_RECORDS
      || state.pending_bytes + bytes > MAX_PENDING_LOG_BYTES)
      && !state.closed
    {
      state = self.space.wait(state).unwrap();
    }
    if state.closed {
      return;
    }
    state.pending_bytes += bytes;
    state.pending.push_back(line);
  }

  /// Release a producer if rendering has terminated unexpectedly.
  pub fn close(&self) {
    self.state.lock().unwrap().closed = true;
    self.space.notify_all();
  }

  /// Drain parsed lines that have not yet been streamed to the terminal.
  pub fn drain_pending(&self) -> Vec<String> {
    let mut state = self.state.lock().unwrap();
    let drained = state.pending.drain(..).collect::<Vec<String>>();
    state.pending_bytes = 0;
    self.space.notify_all();
    drained
  }

  /// Drain at most `limit` pending records so a log burst cannot starve the
  /// live graph renderer for an entire frame.
  pub fn drain_pending_up_to(&self, limit: usize) -> Vec<String> {
    let mut state = self.state.lock().unwrap();
    let count = limit.min(state.pending.len());
    let drained = state.pending.drain(..count).collect::<Vec<String>>();
    let drained_bytes = drained.iter().map(String::len).sum::<usize>();
    state.pending_bytes = state.pending_bytes.saturating_sub(drained_bytes);
    self.space.notify_all();
    drained
  }

  pub fn has_pending(&self) -> bool {
    !self.state.lock().unwrap().pending.is_empty()
  }
}

fn truncate_record(mut line: String) -> String {
  if line.len() <= MAX_LOG_RECORD_BYTES {
    return line;
  }
  let mut end = MAX_LOG_RECORD_BYTES.saturating_sub(TRUNCATION_MARKER.len());
  while !line.is_char_boundary(end) {
    end = end.saturating_sub(1);
  }
  line.truncate(end);
  line.push_str(TRUNCATION_MARKER);
  line
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pending_delivery_is_exactly_once() {
    let store = LogStore::new();
    store.push("zero".to_string());
    store.push("one".to_string());

    assert!(store.has_pending());
    assert_eq!(store.drain_pending_up_to(1), ["zero"]);
    assert!(store.has_pending());
    assert_eq!(store.drain_pending_up_to(1), ["one"]);
    assert!(!store.has_pending());
    assert!(store.drain_pending().is_empty());
  }

  #[test]
  fn giant_records_are_explicitly_truncated() {
    let store = LogStore::new();
    store.push("x".repeat(MAX_LOG_RECORD_BYTES * 4));
    let record = store.drain_pending().pop().unwrap();
    assert!(record.len() <= MAX_LOG_RECORD_BYTES);
    assert!(record.ends_with(TRUNCATION_MARKER));
  }

  #[test]
  fn draining_releases_the_pending_byte_budget() {
    let store = LogStore::new();
    for _ in 0..1_000 {
      store.push("x".repeat(MAX_LOG_RECORD_BYTES * 2));
      store.drain_pending();
    }
    assert_eq!(store.state.lock().unwrap().pending_bytes, 0);
  }

  #[test]
  fn pending_queue_applies_backpressure_and_delivers_every_record() {
    let store = std::sync::Arc::new(LogStore::new());
    let producer_store = store.clone();
    let producer = std::thread::spawn(move || {
      for index in 0..MAX_PENDING_LOG_RECORDS + 17 {
        producer_store.push(index.to_string());
      }
    });

    let mut received = Vec::new();
    while received.len() < MAX_PENDING_LOG_RECORDS + 17 {
      received.extend(store.drain_pending_up_to(31));
      std::thread::yield_now();
    }
    producer.join().unwrap();

    assert_eq!(received.len(), MAX_PENDING_LOG_RECORDS + 17);
    assert_eq!(received.first().unwrap(), "0");
    assert_eq!(
      received.last().unwrap(),
      &(MAX_PENDING_LOG_RECORDS + 16).to_string()
    );
  }
}
