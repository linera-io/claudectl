#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use crate::config::BrainConfig;
use crate::rules::{self, RuleAction, RuleMatch};
use crate::session::{ClaudeSession, SessionStatus};

use super::client::BrainSuggestion;
use super::context;

/// Result sent back from inference thread.
pub struct BrainResult {
    pub pid: u32,
    pub suggestion: Result<BrainSuggestion, String>,
}

/// The brain inference engine. Manages async inference threads and collects results.
pub struct BrainEngine {
    config: BrainConfig,
    tx: Sender<BrainResult>,
    rx: Receiver<BrainResult>,
    /// PIDs currently being inferred (prevents duplicate requests).
    inflight: HashSet<u32>,
    /// Per-PID cooldown to avoid hammering the LLM.
    cooldown: HashMap<u32, Instant>,
    /// Pending suggestions waiting for user confirmation (advisory mode).
    pub pending: HashMap<u32, BrainSuggestion>,
}

const COOLDOWN_SECS: u64 = 10;

impl BrainEngine {
    pub fn new(config: BrainConfig) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            config,
            tx,
            rx,
            inflight: HashSet::new(),
            cooldown: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Run one tick of the brain engine. Call this from app.tick() after refresh().
    ///
    /// 1. Collect results from completed inference threads
    /// 2. Spawn new inference threads for eligible sessions
    ///
    /// Returns a list of (pid, status_message) for actions taken this tick.
    pub fn tick(
        &mut self,
        sessions: &[ClaudeSession],
        deny_rules: &[crate::rules::AutoRule],
    ) -> Vec<(u32, String)> {
        let mut actions = Vec::new();

        // Phase 1: Collect results from completed inferences
        while let Ok(result) = self.rx.try_recv() {
            self.inflight.remove(&result.pid);
            self.cooldown.insert(result.pid, Instant::now());

            match result.suggestion {
                Ok(suggestion) => {
                    // Check if a deny rule overrides the brain
                    let session = sessions.iter().find(|s| s.pid == result.pid);
                    if let Some(session) = session {
                        let deny_match = rules::evaluate(deny_rules, session);
                        if let Some(dm) = &deny_match {
                            if dm.action == RuleAction::Deny {
                                actions.push((
                                    result.pid,
                                    format!(
                                        "Brain suggested {}, but deny rule '{}' overrides",
                                        suggestion.action.label(),
                                        dm.rule_name,
                                    ),
                                ));
                                continue;
                            }
                        }
                    }

                    if self.config.auto_mode {
                        // Auto mode: execute immediately
                        if let Some(session) = session {
                            let rule_match = suggestion_to_rule_match(&suggestion);
                            match rules::execute(&rule_match, session) {
                                Ok(msg) => actions.push((result.pid, msg)),
                                Err(e) => actions.push((result.pid, format!("Brain error: {e}"))),
                            }
                        }
                    } else {
                        // Advisory mode: store for user confirmation
                        self.pending.insert(result.pid, suggestion);
                    }
                }
                Err(e) => {
                    crate::logger::log(
                        "BRAIN",
                        &format!("Inference failed for PID {}: {e}", result.pid),
                    );
                }
            }
        }

        // Phase 2: Spawn inference for eligible sessions
        for session in sessions {
            if !matches!(
                session.status,
                SessionStatus::NeedsInput | SessionStatus::WaitingInput
            ) {
                continue;
            }

            if self.inflight.contains(&session.pid) {
                continue;
            }

            if let Some(last) = self.cooldown.get(&session.pid) {
                if last.elapsed().as_secs() < COOLDOWN_SECS {
                    continue;
                }
            }

            // Already have a pending suggestion for this PID
            if self.pending.contains_key(&session.pid) {
                continue;
            }

            self.spawn_inference(session);
        }

        actions
    }

    fn spawn_inference(&mut self, session: &ClaudeSession) {
        let pid = session.pid;
        let config = self.config.clone();
        let tx = self.tx.clone();

        // Build context on the main thread (reads JSONL files)
        let brain_ctx = context::build_context(session, config.max_context_tokens);
        let prompt = context::format_brain_prompt(&brain_ctx);

        self.inflight.insert(pid);

        std::thread::spawn(move || {
            let suggestion = super::client::infer(&config, &prompt);
            let _ = tx.send(BrainResult { pid, suggestion });
        });
    }

    /// Accept a pending brain suggestion (user pressed 'b').
    pub fn accept(&mut self, pid: u32, session: &ClaudeSession) -> Option<String> {
        let suggestion = self.pending.remove(&pid)?;
        let rule_match = suggestion_to_rule_match(&suggestion);
        match rules::execute(&rule_match, session) {
            Ok(msg) => Some(msg),
            Err(e) => Some(format!("Brain execute error: {e}")),
        }
    }

    /// Reject a pending brain suggestion (user pressed 'B').
    pub fn reject(&mut self, pid: u32) -> Option<BrainSuggestion> {
        self.pending.remove(&pid)
    }

    /// Clear pending suggestions for PIDs that are no longer in NeedsInput/WaitingInput.
    pub fn cleanup(&mut self, sessions: &[ClaudeSession]) {
        let active_pids: HashSet<u32> = sessions.iter().map(|s| s.pid).collect();
        self.pending.retain(|pid, _| {
            active_pids.contains(pid)
                && sessions.iter().any(|s| {
                    s.pid == *pid
                        && matches!(
                            s.status,
                            SessionStatus::NeedsInput | SessionStatus::WaitingInput
                        )
                })
        });
        self.inflight.retain(|pid| active_pids.contains(pid));
    }
}

fn suggestion_to_rule_match(suggestion: &BrainSuggestion) -> RuleMatch {
    RuleMatch {
        rule_name: format!(
            "brain ({}% confidence)",
            (suggestion.confidence * 100.0) as u32
        ),
        action: suggestion.action.clone(),
        message: suggestion.message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{RawSession, TelemetryStatus};

    fn make_config() -> BrainConfig {
        BrainConfig {
            enabled: true,
            endpoint: "http://localhost:11434/api/generate".into(),
            model: "test".into(),
            auto_mode: false,
            timeout_ms: 1000,
            max_context_tokens: 1000,
        }
    }

    fn make_session(pid: u32, status: SessionStatus) -> ClaudeSession {
        let raw = RawSession {
            pid,
            session_id: "test".into(),
            cwd: "/tmp/test".into(),
            started_at: 0,
        };
        let mut s = ClaudeSession::from_raw(raw);
        s.status = status;
        s.telemetry_status = TelemetryStatus::Available;
        s.pending_tool_name = Some("Bash".into());
        s
    }

    #[test]
    fn engine_creates_without_panic() {
        let _engine = BrainEngine::new(make_config());
    }

    #[test]
    fn suggestion_to_rule_match_format() {
        let suggestion = BrainSuggestion {
            action: RuleAction::Approve,
            message: None,
            reasoning: "safe".into(),
            confidence: 0.95,
        };
        let rm = suggestion_to_rule_match(&suggestion);
        assert_eq!(rm.action, RuleAction::Approve);
        assert!(rm.rule_name.contains("95%"));
    }

    #[test]
    fn cleanup_removes_stale_pending() {
        let mut engine = BrainEngine::new(make_config());
        engine.pending.insert(
            999,
            BrainSuggestion {
                action: RuleAction::Approve,
                message: None,
                reasoning: "test".into(),
                confidence: 0.9,
            },
        );

        // PID 999 not in sessions list → should be cleaned up
        engine.cleanup(&[]);
        assert!(engine.pending.is_empty());
    }

    #[test]
    fn cleanup_keeps_active_pending() {
        let mut engine = BrainEngine::new(make_config());
        let session = make_session(100, SessionStatus::NeedsInput);
        engine.pending.insert(
            100,
            BrainSuggestion {
                action: RuleAction::Approve,
                message: None,
                reasoning: "test".into(),
                confidence: 0.9,
            },
        );

        engine.cleanup(&[session]);
        assert!(engine.pending.contains_key(&100));
    }

    #[test]
    fn reject_removes_and_returns_suggestion() {
        let mut engine = BrainEngine::new(make_config());
        engine.pending.insert(
            100,
            BrainSuggestion {
                action: RuleAction::Approve,
                message: None,
                reasoning: "test".into(),
                confidence: 0.9,
            },
        );

        let rejected = engine.reject(100);
        assert!(rejected.is_some());
        assert!(engine.pending.is_empty());
    }
}
