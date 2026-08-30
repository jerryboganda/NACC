//! Normalized provider event vocabulary (master plan S8.2). Every adapter
//! maps its native output onto these variants; workflow decisions consume
//! only this normalized stream and validated artifacts, never raw
//! provider prose. Raw streams may still be *retained*, redacted, in
//! diagnostic logs (S8.2) -- that is a `nacc-observability` concern, not
//! this crate's.
//!
//! Master plan S18: "Do not store or expose hidden chain-of-thought."
//! `ReasoningStatus` below is deliberately a status/progress signal, never
//! a carrier for hidden reasoning content.

use serde::{Deserialize, Serialize};

use nacc_domain::ModelId;

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEvent {
    SessionStarted {
        provider_session_id: String,
        model: ModelId,
    },
    AssistantTextDelta {
        text: String,
    },
    /// Visible progress/status only -- e.g. "thinking", "researching".
    /// Never the hidden reasoning content itself (S18).
    ReasoningStatus {
        status: String,
    },
    ToolRequested {
        tool_name: String,
        summary: String,
    },
    ToolApproved {
        tool_name: String,
    },
    ToolDenied {
        tool_name: String,
        reason: String,
    },
    ToolStarted {
        tool_name: String,
    },
    ToolOutputDelta {
        tool_name: String,
        chunk: String,
    },
    FileChanged {
        path: String,
    },
    CommandStarted {
        command: String,
    },
    CommandOutput {
        chunk: String,
    },
    CommandCompleted {
        exit_code: i32,
    },
    PlanArtifactEmitted {
        summary: String,
    },
    HandoffEmitted {
        artifact_ref: String,
    },
    UsageUpdated {
        detail: String,
    },
    ApprovalRequested {
        summary: String,
    },
    Warning {
        message: String,
    },
    RecoverableError {
        message: String,
    },
    TerminalError {
        message: String,
    },
    SessionCompleted,
    SessionCancelled,
}

/// Sink an adapter emits normalized events to. Deliberately a trait, not a
/// concrete channel type (e.g. `tokio::sync::mpsc::Sender`): this crate
/// stays decoupled from any specific async runtime so provider adapters
/// (which may be built independently) are not forced onto tokio by this
/// contract alone. `nacc-process`/`nacc-orchestrator` supply a real
/// channel-backed implementation.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: ProviderEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct VecSink(Arc<Mutex<Vec<ProviderEvent>>>);
    impl EventSink for VecSink {
        fn emit(&self, event: ProviderEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn event_sink_trait_object_is_usable_via_dyn_dispatch() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink: Box<dyn EventSink> = Box::new(VecSink(buf.clone()));
        sink.emit(ProviderEvent::SessionStarted {
            provider_session_id: "abc123".into(),
            model: "claude-fable-5".into(),
        });
        sink.emit(ProviderEvent::SessionCompleted);
        assert_eq!(buf.lock().unwrap().len(), 2);
    }

    #[test]
    fn provider_event_json_tag_is_stable_snake_case() {
        let json = serde_json::to_string(&ProviderEvent::SessionCancelled).unwrap();
        assert_eq!(json, r#"{"type":"session_cancelled"}"#);
    }

    #[test]
    fn reasoning_status_never_carries_a_hidden_content_field() {
        // Documentation-as-test: ReasoningStatus has exactly one field,
        // `status` (a short label), by construction below -- if a future
        // edit adds a second field this call site must be updated,
        // forcing a deliberate decision rather than a silent scope-creep
        // into carrying hidden chain-of-thought (master plan S18).
        let _ = ProviderEvent::ReasoningStatus { status: "thinking".into() };
    }
}
