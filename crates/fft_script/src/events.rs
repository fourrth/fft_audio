//! Script output/exit events.

use fft_runtime::ScriptRunId;
use serde::{Deserialize, Serialize};

/// Output stream of one script line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptStream {
    Stdout,
    Stderr,
    /// Structured log messages emitted by the embedded engine.
    Log,
}

impl ScriptStream {
    pub const fn as_str(self) -> &'static str {
        match self {
            ScriptStream::Stdout => "stdout",
            ScriptStream::Stderr => "stderr",
            ScriptStream::Log => "log",
        }
    }
}

/// Typed script event. Adapters may map this to their own transport events.
///
/// Script events are owned by the scripting layer and are deliberately not
/// [`fft_app::AppEvent`] variants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ScriptEvent {
    Output {
        run: ScriptRunId,
        stream: ScriptStream,
        line: String,
    },
    Exited {
        run: ScriptRunId,
        success: bool,
        code: Option<i32>,
    },
}

impl ScriptEvent {
    pub const fn run(&self) -> ScriptRunId {
        match self {
            ScriptEvent::Output { run, .. } | ScriptEvent::Exited { run, .. } => *run,
        }
    }
}

/// Receiver for script output and exit notifications.
pub trait ScriptEventSink: Send + Sync {
    fn output(&self, run: ScriptRunId, stream: ScriptStream, line: String);
    fn exited(&self, run: ScriptRunId, success: bool, code: Option<i32>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_events_serialize_with_typed_streams() {
        let event = ScriptEvent::Output {
            run: ScriptRunId(3),
            stream: ScriptStream::Stderr,
            line: "boom".into(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"output","run":3,"stream":"stderr","line":"boom"}"#
        );
        assert_eq!(event.run(), ScriptRunId(3));

        let exit = ScriptEvent::Exited {
            run: ScriptRunId(3),
            success: false,
            code: Some(2),
        };
        assert_eq!(
            serde_json::to_string(&exit).unwrap(),
            r#"{"type":"exited","run":3,"success":false,"code":2}"#
        );
    }
}
