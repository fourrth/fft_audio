//! Stable, typed handles used by the runtime and app contracts.

use serde::{Deserialize, Serialize};

macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        #[repr(transparent)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(raw: u64) -> Self {
                Self(raw)
            }
        }

        impl From<$name> for u64 {
            fn from(id: $name) -> u64 {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

typed_id! {
    /// Handle to a loaded signal (file, table column, or in-memory samples).
    SignalId
}

typed_id! {
    /// Handle to an analysed signal. The underlying result becomes available
    /// through [`crate::FftRuntime::spectrogram_data`].
    AnalysisId
}

typed_id! {
    /// Handle to an order-cut or line-cut result.
    CutId
}

typed_id! {
    /// Display handle. Displays are app-owned, but the ID type lives here so
    /// the runtime, app, and scripting crates share one definition.
    DisplayId
}

typed_id! {
    /// Handle to a scheduled or completed job. Jobs are intentionally separate
    /// from data handles; see [`crate::FftRuntime::job_for`].
    JobId
}

typed_id! {
    /// Handle to one script execution.
    ScriptRunId
}

/// The runtime-owned data entities that can be closed.
///
/// Displays are owned by `fft_app` and are closed through `FftApp::close_display`,
/// not through the runtime, so they are deliberately absent here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum EntityId {
    Signal(SignalId),
    Analysis(AnalysisId),
    Cut(CutId),
}

impl EntityId {
    /// Stable name of the entity category (`signal`, `analysis`, `cut`).
    pub const fn kind(self) -> &'static str {
        match self {
            EntityId::Signal(_) => "signal",
            EntityId::Analysis(_) => "analysis",
            EntityId::Cut(_) => "cut",
        }
    }

    /// Raw numeric id, for logging and transport adapters.
    pub const fn raw(self) -> u64 {
        match self {
            EntityId::Signal(id) => id.0,
            EntityId::Analysis(id) => id.0,
            EntityId::Cut(id) => id.0,
        }
    }
}

impl From<SignalId> for EntityId {
    fn from(id: SignalId) -> Self {
        EntityId::Signal(id)
    }
}

impl From<AnalysisId> for EntityId {
    fn from(id: AnalysisId) -> Self {
        EntityId::Analysis(id)
    }
}

impl From<CutId> for EntityId {
    fn from(id: CutId) -> Self {
        EntityId::Cut(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_transparently_and_carry_their_name() {
        let signal = SignalId::new(7);
        assert_eq!(signal.get(), 7);
        assert_eq!(signal.0, 7);
        assert_eq!(serde_json::to_string(&signal).unwrap(), "7");
        assert_eq!(SignalId::from(7), signal);
        assert_eq!(u64::from(signal), 7);
        assert_eq!(signal.to_string(), "SignalId(7)");

        let run = ScriptRunId(3);
        assert_eq!(serde_json::from_str::<ScriptRunId>("3").unwrap(), run);
    }

    #[test]
    fn entity_ids_round_trip_and_keep_runtime_categories() {
        let entity = EntityId::Analysis(AnalysisId(9));
        assert_eq!(entity.kind(), "analysis");
        assert_eq!(entity.raw(), 9);
        let json = serde_json::to_string(&entity).unwrap();
        assert_eq!(json, r#"{"kind":"analysis","id":9}"#);
        assert_eq!(serde_json::from_str::<EntityId>(&json).unwrap(), entity);
        assert_eq!(EntityId::from(CutId(2)), EntityId::Cut(CutId(2)));
    }
}
