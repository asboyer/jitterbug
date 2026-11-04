//! What agents asked of Jitterbug this app session: which tool, from which agent and session,
//! and how it ended. Metadata only, in memory, bounded; no arguments, reasons, or answers.
use serde::Serialize;
use std::{collections::VecDeque, sync::Mutex};

const LIMIT: usize = 100;
static LOG: Mutex<VecDeque<Call>> = Mutex::new(VecDeque::new());

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Call {
    pub at: u64,
    /// The control method, such as `overview`, `pause_session`, or `pressure_hint`.
    pub method: String,
    /// The coding agent behind the caller by the name people know it, when recognized.
    pub agent: Option<String>,
    /// The title of the session the caller runs in, when an app launched it.
    pub session: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
    /// For pause, close, and background requests: the approval case it opened.
    pub request_id: Option<u64>,
    /// For pressure hints: whether the agent was actually told about pressure.
    pub hinted: Option<bool>,
}

pub fn record(call: Call) {
    if let Ok(mut log) = LOG.lock() {
        if log.len() >= LIMIT {
            log.pop_front();
        }
        log.push_back(call);
    }
}

/// Calls this session, newest first.
pub fn list() -> Vec<Call> {
    LOG.lock()
        .map(|log| log.iter().rev().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_is_bounded_and_newest_first() {
        for i in 0..(LIMIT as u64 + 5) {
            record(Call {
                at: 1_000_000 + i,
                method: "overview".into(),
                agent: None,
                session: None,
                ok: true,
                error: None,
                request_id: None,
                hinted: None,
            });
        }
        let calls = list();
        assert!(calls.len() <= LIMIT);
        assert!(calls.windows(2).all(|w| w[0].at >= w[1].at));
    }
}
