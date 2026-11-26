//! App-level events and subscriptions.

use fft_runtime::{DisplayId, JobId};
use serde::{Deserialize, Serialize};

use crate::display::DisplayInfo;

/// Opaque subscription handle returned by `FftApp::subscribe`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subscription(u64);

impl Subscription {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Event stream consumed by HTTP/Tauri adapters and the frontend.
///
/// Adapters translate variants into transport names such as `display.open` or
/// `analysis.progress`; the payloads stay typed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AppEvent {
    JobProgress {
        job: JobId,
        frames_done: usize,
        frames_total: Option<usize>,
        eta_s: Option<f64>,
    },
    JobDone {
        job: JobId,
    },
    JobFailed {
        job: JobId,
        message: String,
    },
    DisplayOpened {
        display: DisplayInfo,
    },
    DisplayUpdated {
        display: DisplayInfo,
    },
    DisplayClosed {
        id: DisplayId,
        /// All descendants removed by a parent close, direct and recursive.
        children: Vec<DisplayId>,
    },
}

/// Receiver for [`AppEvent`]s.
pub trait AppEventSink: Send + Sync {
    fn emit(&self, event: AppEvent);
}

impl<F> AppEventSink for F
where
    F: Fn(AppEvent) + Send + Sync,
{
    fn emit(&self, event: AppEvent) {
        self(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DisplayKind;

    #[test]
    fn app_events_serialize_with_type_tags_and_camel_case_fields() {
        let event = AppEvent::JobProgress {
            job: JobId(2),
            frames_done: 5,
            frames_total: Some(10),
            eta_s: Some(1.5),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"job_progress","job":2,"framesDone":5,"framesTotal":10,"etaS":1.5}"#
        );

        let closed = AppEvent::DisplayClosed {
            id: DisplayId(1_000_000),
            children: vec![DisplayId(1_000_001)],
        };
        assert_eq!(
            serde_json::to_string(&closed).unwrap(),
            r#"{"type":"display_closed","id":1000000,"children":[1000001]}"#
        );
    }

    #[test]
    fn display_events_carry_typed_info() {
        let info = DisplayInfo {
            id: DisplayId(1_000_000),
            parent: None,
            kind: DisplayKind::Spectrogram,
            name: "spec".into(),
            state: serde_json::json!({ "analysisId": 1 }),
            visible: true,
            ephemeral: false,
        };
        let event = AppEvent::DisplayOpened {
            display: info.clone(),
        };
        assert_eq!(event, AppEvent::DisplayOpened { display: info });
        let back: AppEvent = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn subscriptions_are_opaque_but_comparable() {
        let a = Subscription::new(1);
        let b = Subscription::new(2);
        assert_ne!(a, b);
        assert_eq!(a, Subscription::new(1));
        // The field is intentionally private; these ids are not user input.
        assert_eq!(format!("{a:?}").contains("Subscription"), true);
    }
}
