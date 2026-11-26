//! Language-neutral CLI scripting composition.
//!
//! The blocking sink and sequential run loop work with any [`FftScript`]
//! engine. The `python` feature is only the current engine constructor; adding
//! another language means adding another engine and feature, not another CLI
//! composition implementation.
//!
//! Each script run receives a clone of the adapter-lifetime lease; the engine
//! drops its clone when the run exits. This module blocks until the engine
//! reports the exit so scripts run sequentially and stop on the first failure.

use std::path::Path;
use std::sync::Arc;

use fft_app::AppLease;

#[cfg(feature = "python")]
use std::sync::{mpsc, Mutex};

#[cfg(feature = "python")]
use fft_script::{FftScript, ScriptEventSink, ScriptRunId, ScriptSource, ScriptStream};

use crate::composition::ScriptComposition;
#[cfg(not(feature = "python"))]
use crate::error::PYTHON_DISABLED;
use crate::CliError;

/// CLI scripting adapter used by [`crate::DesktopApp`].
///
/// The engine is injected. Python is the current feature; future languages
/// add another engine constructor/feature and reuse this adapter.
pub(crate) struct CliScriptComposition {
    #[cfg(feature = "python")]
    engine: Arc<dyn FftScript>,
}

impl CliScriptComposition {
    pub(crate) fn new() -> Self {
        #[cfg(feature = "python")]
        {
            Self {
                engine: Arc::new(fft_python_adapter::PythonScript::new()),
            }
        }
        #[cfg(not(feature = "python"))]
        {
            Self {}
        }
    }

    #[cfg(all(test, feature = "python"))]
    pub(crate) fn with_engine(engine: Arc<dyn FftScript>) -> Self {
        Self { engine }
    }
}

impl ScriptComposition for CliScriptComposition {
    fn run(&self, lease: Arc<dyn AppLease>, path: &Path) -> Result<(), CliError> {
        #[cfg(feature = "python")]
        {
            run_with_engine(self.engine.as_ref(), lease, path)
        }
        #[cfg(not(feature = "python"))]
        {
            let _ = (lease, path);
            Err(CliError::unsupported_feature("python", PYTHON_DISABLED))
        }
    }
}

#[cfg(feature = "python")]
fn run_with_engine(
    engine: &dyn FftScript,
    lease: Arc<dyn AppLease>,
    path: &Path,
) -> Result<(), CliError> {
    println!("fft_desktop: running script {}", path.display());

    let (sink, exit) = CliScriptSink::new();
    let _run = engine
        .run(lease, ScriptSource::File(path.to_path_buf()), sink)
        .map_err(|error| {
            CliError::app(format!(
                "could not start script {}: {error}",
                path.display()
            ))
        })?;

    let outcome = exit.recv().map_err(|_| {
        CliError::app(format!(
            "script {} ended without an exit event",
            path.display()
        ))
    })?;

    if outcome.success {
        Ok(())
    } else {
        Err(match outcome.code {
            Some(code) => CliError::app(format!(
                "script {} failed with exit code {code}",
                path.display()
            )),
            None => CliError::app(format!("script {} failed", path.display())),
        })
    }
}

#[cfg(feature = "python")]
struct ScriptExit {
    success: bool,
    code: Option<i32>,
}

#[cfg(feature = "python")]
struct CliScriptSink {
    exit: Mutex<Option<mpsc::Sender<ScriptExit>>>,
}

#[cfg(feature = "python")]
impl CliScriptSink {
    fn new() -> (Arc<Self>, mpsc::Receiver<ScriptExit>) {
        let (exit_tx, exit_rx) = mpsc::channel();
        (
            Arc::new(Self {
                exit: Mutex::new(Some(exit_tx)),
            }),
            exit_rx,
        )
    }
}

#[cfg(feature = "python")]
impl ScriptEventSink for CliScriptSink {
    fn output(&self, _run: ScriptRunId, stream: ScriptStream, line: String) {
        match stream {
            ScriptStream::Stdout => println!("{line}"),
            ScriptStream::Stderr | ScriptStream::Log => eprintln!("{line}"),
        }
    }

    fn exited(&self, _run: ScriptRunId, success: bool, code: Option<i32>) {
        let sender = self
            .exit
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(ScriptExit { success, code });
        }
    }
}
