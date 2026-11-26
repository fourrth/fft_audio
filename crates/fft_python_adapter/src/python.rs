//! Embedded CPython binding for `fft_audio`.
//!
//! The module is registered directly in `sys.modules` by every script run, so
//! scripts use `import fft_audio` without an installed extension module. One
//! interpreter is initialized per process; runs are serialized (a second
//! concurrent run is rejected with [`ScriptError::Busy`]).
//!
//! Each run receives one [`AppLease`]. The adapter never owns
//! an app, constructs a runtime, or mints leases: the runner moves the granted
//! lease into the run, reaches the app through it, and releases it before the
//! exit event. Python objects retained past the run keep only an inert run
//! handle and cannot reach the finished run's app or lease.
//!
//! Cancellation is cooperative: the embedded functions check a per-run token
//! at every binding boundary and while waiting for jobs. A script that never
//! calls back into `fft_audio` cannot be interrupted from another thread.

use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fft_app::{
    AppError, AppInfo as RustAppInfo, AppLease, CutDisplayOptions, DisplayId, DisplayInfo,
    DisplayKind as RustDisplayKind, FftApp, SpectrogramDisplayOptions,
};
use fft_core::orders::LineSpec;
use fft_core::settings::{
    AnalysisSettings, CutAxis as CoreCutAxis, DisplaySettings, LineCutSettings, OrderCutSettings,
    RpmSettings, RpmSource, WindowKind,
};
use fft_core::spectrogram::RenderOptions;
use fft_core::table::TableInfo as CoreTableInfo;
use fft_runtime::{
    AnalysisId, AnalysisInfo, CutData, CutId, EntityId, FftRuntime, JobId,
    JobStatus as RustJobStatus, LineCutData, LineCutRequest, LoadFileRequest, LoadSignalRequest,
    OrderCutData, OrderCutRequest, RpmPoint, RuntimeError, ScriptRunId, SignalId, SignalInfo,
};
use fft_script::{FftScript, ScriptError, ScriptEventSink, ScriptSource, ScriptStream};
use numpy::{PyArray1, PyArray2, PyArrayMethods};
use pyo3::exceptions::{
    PyFileNotFoundError, PyOSError, PyRuntimeError, PySystemExit, PyTimeoutError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::code::PyCodeInput;
use pyo3::types::{
    PyAny, PyCode, PyCodeMethods, PyDict, PyModule, PyTuple, PyWeakrefMethods, PyWeakrefReference,
};

use crate::views;

pyo3::create_exception!(
    fft_audio,
    DataDetachedError,
    pyo3::exceptions::PyRuntimeError
);
pyo3::create_exception!(
    fft_audio,
    DataBorrowedError,
    pyo3::exceptions::PyRuntimeError
);
pyo3::create_exception!(fft_audio, CancelledError, pyo3::exceptions::PyRuntimeError);
pyo3::create_exception!(fft_audio, JobFailedError, pyo3::exceptions::PyRuntimeError);

// --------------------------------------------------------------- run context ---

struct RunResources {
    lease: Arc<dyn AppLease>,
    runtime: Arc<dyn FftRuntime>,
}

/// State shared by the runner and every Python object created during the run.
///
/// The lease/runtime live inside `resources` and are cleared before the exit
/// event. A Python object retained past the run therefore keeps only this
/// inert state; it cannot reach the finished run's app or lease.
struct RunState {
    run: ScriptRunId,
    resources: Mutex<Option<RunResources>>,
    events: Arc<dyn ScriptEventSink>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
struct RunContext {
    state: Arc<RunState>,
}

impl RunContext {
    fn new(
        run: ScriptRunId,
        lease: Arc<dyn AppLease>,
        events: Arc<dyn ScriptEventSink>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        // Use the leased app's runtime handle for this run; the adapter never
        // constructs a runtime of its own.
        let runtime = lease.app().runtime();
        Self {
            state: Arc::new(RunState {
                run,
                resources: Mutex::new(Some(RunResources { lease, runtime })),
                events,
                cancel,
            }),
        }
    }

    fn run(&self) -> ScriptRunId {
        self.state.run
    }

    fn events(&self) -> Arc<dyn ScriptEventSink> {
        Arc::clone(&self.state.events)
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancel.load(Ordering::SeqCst)
    }

    fn check_cancel(&self) -> PyResult<()> {
        if self.is_cancelled() {
            Err(CancelledError::new_err("script cancelled"))
        } else {
            Ok(())
        }
    }

    fn lease(&self) -> PyResult<Arc<dyn AppLease>> {
        let resources = self
            .state
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let resources = resources.as_ref().ok_or_else(run_ended_error)?;
        Ok(Arc::clone(&resources.lease))
    }

    fn with_app<T>(&self, f: impl FnOnce(&dyn FftApp) -> T) -> PyResult<T> {
        // Clone the lease out of the run state before calling the app: app
        // implementations may emit events or take their own locks.
        let lease = self.lease()?;
        Ok(f(lease.app()))
    }

    fn with_app_result<T>(
        &self,
        f: impl FnOnce(&dyn FftApp) -> Result<T, AppError>,
    ) -> PyResult<T> {
        self.with_app(f)?.map_err(map_app_error)
    }

    fn runtime(&self) -> PyResult<Arc<dyn FftRuntime>> {
        let resources = self
            .state
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let resources = resources.as_ref().ok_or_else(run_ended_error)?;
        Ok(Arc::clone(&resources.runtime))
    }

    /// Release the run's lease/runtime. Idempotent; every exit path calls it.
    fn release(&self) {
        let resources = self
            .state
            .resources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(resources);
    }
}

fn run_ended_error() -> PyErr {
    PyRuntimeError::new_err("script run has ended; its app lease is no longer available")
}

static CURRENT_RUN: OnceLock<Mutex<Option<RunContext>>> = OnceLock::new();

fn current_cell() -> &'static Mutex<Option<RunContext>> {
    CURRENT_RUN.get_or_init(|| Mutex::new(None))
}

fn set_current(context: Option<RunContext>) {
    *current_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = context;
}

fn current() -> PyResult<RunContext> {
    current_cell()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .ok_or_else(|| {
            PyRuntimeError::new_err(
                "no active fft_audio script run (App() is only valid while a script is running)",
            )
        })
}

/// Clears the process-global current run even if Python execution unwinds.
struct CurrentGuard;

impl Drop for CurrentGuard {
    fn drop(&mut self) {
        set_current(None);
    }
}

fn check_cancel(context: &RunContext) -> PyResult<()> {
    context.check_cancel()
}

// ------------------------------------------------------------ error mapping ---

fn map_runtime_error(error: RuntimeError) -> PyErr {
    match error.code() {
        "invalid_input" => PyValueError::new_err(error.message().to_string()),
        "not_found" => PyFileNotFoundError::new_err(error.message().to_string()),
        "closed" => PyRuntimeError::new_err(error.to_string()),
        "busy" => PyTimeoutError::new_err(error.message().to_string()),
        "analysis_failed" => JobFailedError::new_err(error.to_string()),
        "cancelled" => CancelledError::new_err(error.message().to_string()),
        "export" => PyOSError::new_err(error.to_string()),
        "unsupported" => PyRuntimeError::new_err(format!("unsupported: {}", error.message())),
        _ => PyRuntimeError::new_err(error.to_string()),
    }
}

fn map_app_error(error: AppError) -> PyErr {
    match error.code() {
        "invalid_input" => PyValueError::new_err(error.message().to_string()),
        "not_found" => PyFileNotFoundError::new_err(error.message().to_string()),
        "closed" => PyRuntimeError::new_err(error.to_string()),
        "busy" => PyTimeoutError::new_err(error.message().to_string()),
        "unsupported" => PyRuntimeError::new_err(format!("unsupported: {}", error.message())),
        _ => PyRuntimeError::new_err(error.to_string()),
    }
}

fn error_instance(py: Python<'_>, error: PyErr) -> Py<PyAny> {
    error.value(py).clone().into_any().unbind()
}

fn parse_timeout(timeout_s: Option<f64>) -> PyResult<Option<Duration>> {
    match timeout_s {
        None => Ok(None),
        Some(seconds) if !seconds.is_finite() || seconds < 0.0 => Err(PyValueError::new_err(
            "timeout_s must be a finite, non-negative number",
        )),
        Some(seconds) => Ok(Some(Duration::from_secs_f64(seconds))),
    }
}

fn poll_wait(context: &RunContext, job: JobId, timeout: Option<Duration>) -> PyResult<()> {
    let runtime = context.runtime()?;
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        check_cancel(context)?;
        let slice = match deadline {
            Some(deadline) => deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50)),
            None => Duration::from_millis(50),
        };
        let wait = Python::attach(|py| py.detach(|| runtime.wait(job, Some(slice))));
        // The runtime wait is a blocking condvar/queue poll; `Python::detach`
        // above releases the GIL so other Python threads can run.
        match wait {
            Ok(()) => return Ok(()),
            Err(error) if error.code() == "busy" => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(PyTimeoutError::new_err("timed out waiting for job"));
                }
            }
            Err(error) => return Err(map_runtime_error(error)),
        }
    }
}

fn poll_wait_all(context: &RunContext, timeout: Option<Duration>) -> PyResult<()> {
    let runtime = context.runtime()?;
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        check_cancel(context)?;
        let slice = match deadline {
            Some(deadline) => deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50)),
            None => Duration::from_millis(50),
        };
        let wait = Python::attach(|py| py.detach(|| runtime.wait_all(Some(slice))));
        match wait {
            Ok(()) => return Ok(()),
            Err(error) if error.code() == "busy" => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(PyTimeoutError::new_err("timed out waiting for jobs"));
                }
            }
            Err(error) => return Err(map_runtime_error(error)),
        }
    }
}

fn wait_entity(context: &RunContext, entity: EntityId, timeout: Option<Duration>) -> PyResult<()> {
    let runtime = context.runtime()?;
    let job = runtime
        .job_for(entity)
        .ok_or_else(|| PyRuntimeError::new_err("entity is closed or unknown"))?;
    poll_wait(context, job, timeout)
}

fn entity_status(context: &RunContext, entity: EntityId) -> PyJobStatus {
    let Ok(runtime) = context.runtime() else {
        return PyJobStatus::closed();
    };
    runtime
        .job_for(entity)
        .map(|job| PyJobStatus::from_rust(runtime.job_status(job)))
        .unwrap_or(PyJobStatus::closed())
}

fn entity_error(
    py: Python<'_>,
    context: &RunContext,
    entity: EntityId,
    detached: bool,
) -> PyResult<Option<Py<PyAny>>> {
    if detached {
        return Ok(Some(error_instance(
            py,
            DataDetachedError::new_err("data was detached by into_numpy()"),
        )));
    }
    let runtime = context.runtime()?;
    let Some(job) = runtime.job_for(entity) else {
        return Ok(None);
    };
    match runtime.wait(job, Some(Duration::ZERO)) {
        Ok(()) => Ok(None),
        Err(error) if error.code() == "busy" => Ok(None),
        Err(error) => Ok(Some(error_instance(py, map_runtime_error(error)))),
    }
}

// -------------------------------------------------------------- string enums ---

fn hash_str(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[pyclass(module = "fft_audio", name = "Window", frozen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PyWindow {
    kind: WindowKind,
}

#[allow(non_snake_case)]
#[pymethods]
impl PyWindow {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        parse_window(value)
            .map(|kind| Self { kind })
            .ok_or_else(|| PyValueError::new_err(format!("unknown window {value:?}")))
    }

    #[classattr]
    fn HANN() -> Self {
        Self {
            kind: WindowKind::Hann,
        }
    }
    #[classattr]
    fn HAMMING() -> Self {
        Self {
            kind: WindowKind::Hamming,
        }
    }
    #[classattr]
    fn BLACKMAN() -> Self {
        Self {
            kind: WindowKind::Blackman,
        }
    }
    #[classattr]
    fn NUTTALL() -> Self {
        Self {
            kind: WindowKind::Nuttall,
        }
    }
    #[classattr]
    fn TRIANGULAR() -> Self {
        Self {
            kind: WindowKind::Triangular,
        }
    }
    #[classattr]
    fn RECTANGULAR() -> Self {
        Self {
            kind: WindowKind::Rectangular,
        }
    }
    #[classattr]
    fn NONE() -> Self {
        Self {
            kind: WindowKind::None,
        }
    }

    #[getter]
    fn value(&self) -> &'static str {
        self.kind.name()
    }

    #[getter]
    fn name(&self) -> String {
        self.kind.name().to_uppercase()
    }

    fn __str__(&self) -> &'static str {
        self.kind.name()
    }

    fn __repr__(&self) -> String {
        format!("Window.{}", self.kind.name().to_uppercase())
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.kind.name())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other) = other.cast::<PyWindow>() {
            self.kind == other.borrow().kind
        } else if let Ok(value) = other.extract::<String>() {
            self.kind.name() == value
        } else {
            false
        }
    }
}

fn extract_window(value: &Bound<'_, PyAny>) -> PyResult<PyWindow> {
    if let Ok(window) = value.cast::<PyWindow>() {
        return Ok(*window.borrow());
    }
    if let Ok(name) = value.extract::<String>() {
        return PyWindow::new(&name);
    }
    Err(PyValueError::new_err(
        "window must be a fft_audio.Window value or its string name",
    ))
}

fn extract_cut_axis(value: &Bound<'_, PyAny>) -> PyResult<PyCutAxis> {
    if let Ok(axis) = value.cast::<PyCutAxis>() {
        return Ok(*axis.borrow());
    }
    if let Ok(name) = value.extract::<String>() {
        return PyCutAxis::new(&name);
    }
    Err(PyValueError::new_err(
        "axis must be a fft_audio.CutAxis value or its string name",
    ))
}

fn parse_window(value: &str) -> Option<WindowKind> {
    match value.to_ascii_lowercase().as_str() {
        "hann" => Some(WindowKind::Hann),
        "hamming" => Some(WindowKind::Hamming),
        "blackman" => Some(WindowKind::Blackman),
        "nuttall" => Some(WindowKind::Nuttall),
        "triangular" => Some(WindowKind::Triangular),
        "rectangular" => Some(WindowKind::Rectangular),
        "none" => Some(WindowKind::None),
        _ => None,
    }
}

#[pyclass(module = "fft_audio", name = "CutAxis", frozen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PyCutAxis {
    axis: PyCutAxisKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PyCutAxisKind {
    Time,
    Rpm,
    Freq,
}

#[allow(non_snake_case)]
#[pymethods]
impl PyCutAxis {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        let axis = match value.to_ascii_lowercase().as_str() {
            "time" => PyCutAxisKind::Time,
            "rpm" => PyCutAxisKind::Rpm,
            "freq" => PyCutAxisKind::Freq,
            _ => return Err(PyValueError::new_err(format!("unknown cut axis {value:?}"))),
        };
        Ok(Self { axis })
    }

    #[classattr]
    fn TIME() -> Self {
        Self {
            axis: PyCutAxisKind::Time,
        }
    }
    #[classattr]
    fn RPM() -> Self {
        Self {
            axis: PyCutAxisKind::Rpm,
        }
    }
    #[classattr]
    fn FREQ() -> Self {
        Self {
            axis: PyCutAxisKind::Freq,
        }
    }

    #[getter]
    fn value(&self) -> &'static str {
        match self.axis {
            PyCutAxisKind::Time => "time",
            PyCutAxisKind::Rpm => "rpm",
            PyCutAxisKind::Freq => "freq",
        }
    }

    #[getter]
    fn name(&self) -> String {
        self.value().to_uppercase()
    }

    fn __str__(&self) -> &'static str {
        self.value()
    }

    fn __repr__(&self) -> String {
        format!("CutAxis.{}", self.value().to_uppercase())
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.value())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other) = other.cast::<PyCutAxis>() {
            self.axis == other.borrow().axis
        } else if let Ok(value) = other.extract::<String>() {
            self.value() == value
        } else {
            false
        }
    }
}

impl PyCutAxis {
    fn core_axis(&self) -> CoreCutAxis {
        match self.axis {
            PyCutAxisKind::Time => CoreCutAxis::Time,
            PyCutAxisKind::Rpm => CoreCutAxis::Rpm,
            PyCutAxisKind::Freq => CoreCutAxis::Freq,
        }
    }
}

#[pyclass(module = "fft_audio", name = "DisplayKind", frozen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PyDisplayKind {
    kind: RustDisplayKind,
}

#[allow(non_snake_case)]
#[pymethods]
impl PyDisplayKind {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        let kind = match value.to_ascii_lowercase().as_str() {
            "spectrogram" => RustDisplayKind::Spectrogram,
            "order_cuts" => RustDisplayKind::OrderCuts,
            "line_cuts" => RustDisplayKind::LineCuts,
            "plot" => RustDisplayKind::Plot,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown display kind {value:?}"
                )))
            }
        };
        Ok(Self { kind })
    }

    #[classattr]
    fn SPECTROGRAM() -> Self {
        Self {
            kind: RustDisplayKind::Spectrogram,
        }
    }
    #[classattr]
    fn ORDER_CUTS() -> Self {
        Self {
            kind: RustDisplayKind::OrderCuts,
        }
    }
    #[classattr]
    fn LINE_CUTS() -> Self {
        Self {
            kind: RustDisplayKind::LineCuts,
        }
    }
    #[classattr]
    fn PLOT() -> Self {
        Self {
            kind: RustDisplayKind::Plot,
        }
    }

    #[getter]
    fn value(&self) -> &'static str {
        self.kind.as_str()
    }

    #[getter]
    fn name(&self) -> String {
        match self.kind {
            RustDisplayKind::Spectrogram => "SPECTROGRAM",
            RustDisplayKind::OrderCuts => "ORDER_CUTS",
            RustDisplayKind::LineCuts => "LINE_CUTS",
            RustDisplayKind::Plot => "PLOT",
        }
        .to_string()
    }

    fn __str__(&self) -> &'static str {
        self.kind.as_str()
    }

    fn __repr__(&self) -> String {
        format!("DisplayKind.{}", self.value().to_uppercase())
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.value())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other) = other.cast::<PyDisplayKind>() {
            self.kind == other.borrow().kind
        } else if let Ok(value) = other.extract::<String>() {
            self.value() == value
        } else {
            false
        }
    }
}

impl PyDisplayKind {
    fn from_rust(kind: RustDisplayKind) -> Self {
        Self { kind }
    }
}

#[pyclass(module = "fft_audio", name = "JobStatus", frozen)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PyJobStatus {
    status: RustJobStatus,
}

#[allow(non_snake_case)]
#[pymethods]
impl PyJobStatus {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        let status = match value.to_ascii_lowercase().as_str() {
            "pending" => RustJobStatus::Pending,
            "running" => RustJobStatus::Running,
            "ready" => RustJobStatus::Ready,
            "failed" => RustJobStatus::Failed,
            "closed" => RustJobStatus::Closed,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown job status {value:?}"
                )))
            }
        };
        Ok(Self { status })
    }

    #[classattr]
    fn PENDING() -> Self {
        Self {
            status: RustJobStatus::Pending,
        }
    }
    #[classattr]
    fn RUNNING() -> Self {
        Self {
            status: RustJobStatus::Running,
        }
    }
    #[classattr]
    fn READY() -> Self {
        Self {
            status: RustJobStatus::Ready,
        }
    }
    #[classattr]
    fn FAILED() -> Self {
        Self {
            status: RustJobStatus::Failed,
        }
    }
    #[classattr]
    fn CLOSED() -> Self {
        Self {
            status: RustJobStatus::Closed,
        }
    }

    #[getter]
    fn value(&self) -> &'static str {
        match self.status {
            RustJobStatus::Pending => "pending",
            RustJobStatus::Running => "running",
            RustJobStatus::Ready => "ready",
            RustJobStatus::Failed => "failed",
            RustJobStatus::Closed => "closed",
        }
    }

    #[getter]
    fn name(&self) -> String {
        self.value().to_uppercase()
    }

    fn __str__(&self) -> &'static str {
        self.value()
    }

    fn __repr__(&self) -> String {
        format!("JobStatus.{}", self.value().to_uppercase())
    }

    fn __hash__(&self) -> u64 {
        hash_str(self.value())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        if let Ok(other) = other.cast::<PyJobStatus>() {
            self.status == other.borrow().status
        } else if let Ok(value) = other.extract::<String>() {
            self.value() == value
        } else {
            false
        }
    }
}

impl PyJobStatus {
    fn from_rust(status: RustJobStatus) -> Self {
        Self { status }
    }
    fn closed() -> Self {
        Self {
            status: RustJobStatus::Closed,
        }
    }
}

// --------------------------------------------------------------- info classes ---

#[pyclass(module = "fft_audio", name = "TableInfo")]
#[derive(Clone)]
pub struct PyTableInfo {
    rows: usize,
    columns: Vec<String>,
    signal: String,
    time: Option<String>,
    notes: Vec<String>,
}

#[pymethods]
impl PyTableInfo {
    #[getter]
    fn rows(&self) -> usize {
        self.rows
    }
    #[getter]
    fn columns<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.columns.iter())
    }
    #[getter]
    fn signal(&self) -> &str {
        &self.signal
    }
    #[getter]
    fn time(&self) -> Option<&str> {
        self.time.as_deref()
    }
    #[getter]
    fn notes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.notes.iter())
    }
}

impl From<&CoreTableInfo> for PyTableInfo {
    fn from(info: &CoreTableInfo) -> Self {
        Self {
            rows: info.rows,
            columns: info.columns.clone(),
            signal: info.signal.clone(),
            time: info.time.clone(),
            notes: info.notes.clone(),
        }
    }
}

#[pyclass(module = "fft_audio", name = "AppInfo")]
#[derive(Clone)]
pub struct PyAppInfo {
    name: String,
    version: String,
    mode: &'static str,
}

#[pymethods]
impl PyAppInfo {
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }
    #[getter]
    fn version(&self) -> &str {
        &self.version
    }
    #[getter]
    fn mode(&self) -> &'static str {
        self.mode
    }
    fn __repr__(&self) -> String {
        format!(
            "AppInfo(name={:?}, version={:?}, mode={:?})",
            self.name, self.version, self.mode
        )
    }
}

impl From<&RustAppInfo> for PyAppInfo {
    fn from(info: &RustAppInfo) -> Self {
        Self {
            name: info.name.clone(),
            version: info.version.clone(),
            mode: info.mode.as_str(),
        }
    }
}

#[pyclass(module = "fft_audio", name = "SpectrogramInfo")]
#[derive(Clone)]
pub struct PySpectrogramInfo {
    info: AnalysisInfo,
}

#[pymethods]
impl PySpectrogramInfo {
    #[getter]
    fn name(&self) -> &str {
        &self.info.name
    }
    #[getter]
    fn path(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.info
            .path
            .as_deref()
            .map(|path| path_to_py(py, path))
            .transpose()
    }
    #[getter]
    fn sample_rate(&self) -> f64 {
        self.info.sample_rate
    }
    #[getter]
    fn frame_s(&self) -> f64 {
        self.info.frame_s
    }
    #[getter]
    fn frames(&self) -> usize {
        self.info.frames
    }
    #[getter]
    fn bins(&self) -> usize {
        self.info.bins
    }
    #[getter]
    fn bin_hz(&self) -> f32 {
        self.info.bin_hz
    }
    #[getter]
    fn hop_s(&self) -> f64 {
        self.info.hop_s
    }
    #[getter]
    fn duration_s(&self) -> f64 {
        self.info.duration_s
    }
    #[getter]
    fn nyquist(&self) -> f32 {
        self.info.nyquist
    }
    #[getter]
    fn max_db(&self) -> f32 {
        self.info.max_db
    }
    #[getter]
    fn min_db(&self) -> f32 {
        self.info.min_db
    }
    #[getter]
    fn table(&self) -> Option<PyTableInfo> {
        self.info.table.as_ref().map(PyTableInfo::from)
    }
    fn __repr__(&self) -> String {
        format!(
            "SpectrogramInfo(name={:?}, frames={}, bins={})",
            self.info.name, self.info.frames, self.info.bins
        )
    }
}

#[pyclass(module = "fft_audio", name = "OrderCutInfo")]
pub struct PyOrderCutInfo {
    name: String,
    data: Arc<OrderCutData>,
}

#[pymethods]
impl PyOrderCutInfo {
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }
    #[getter]
    fn frames(&self) -> usize {
        self.data.frames
    }
    #[getter]
    fn order_count(&self) -> usize {
        self.data.order_count
    }
    #[getter]
    fn times<'py>(slf: Bound<'py, Self>) -> Bound<'py, PyArray1<f64>> {
        let base = slf.clone().into_any();
        let this = slf.borrow();
        views::readonly_1d(this.data.times.as_slice(), base)
    }
    #[getter]
    fn orders<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.data.orders.iter())
    }
    #[getter]
    fn frequencies<'py>(slf: Bound<'py, Self>) -> Bound<'py, PyArray2<f32>> {
        let base = slf.clone().into_any();
        let this = slf.borrow();
        views::readonly_2d(
            this.data.frequencies.as_slice(),
            this.data.frames,
            this.data.order_count,
            base,
        )
    }
    fn __repr__(&self) -> String {
        format!(
            "OrderCutInfo(name={:?}, frames={}, orders={})",
            self.name, self.data.frames, self.data.order_count
        )
    }
}

#[pyclass(module = "fft_audio", name = "LineCutInfo")]
pub struct PyLineCutInfo {
    name: String,
    data: Arc<LineCutData>,
}

#[pymethods]
impl PyLineCutInfo {
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }
    #[getter]
    fn length(&self) -> usize {
        self.data.len()
    }
    #[getter]
    fn times<'py>(slf: Bound<'py, Self>) -> Bound<'py, PyArray1<f64>> {
        let base = slf.clone().into_any();
        let this = slf.borrow();
        views::readonly_1d(this.data.times.as_slice(), base)
    }
    #[getter]
    fn frequencies<'py>(slf: Bound<'py, Self>) -> Bound<'py, PyArray1<f64>> {
        let base = slf.clone().into_any();
        let this = slf.borrow();
        views::readonly_1d(this.data.frequencies.as_slice(), base)
    }
    #[getter]
    fn fractions<'py>(slf: Bound<'py, Self>) -> Bound<'py, PyArray1<f64>> {
        let base = slf.clone().into_any();
        let this = slf.borrow();
        views::readonly_1d(this.data.fractions.as_slice(), base)
    }
    fn __repr__(&self) -> String {
        format!(
            "LineCutInfo(name={:?}, points={})",
            self.name,
            self.data.len()
        )
    }
}

// ------------------------------------------------------------------- Display ---

#[pyclass(module = "fft_audio", name = "Display")]
pub struct PyDisplay {
    context: RunContext,
    id: DisplayId,
    closed: AtomicBool,
}

impl PyDisplay {
    fn new(context: RunContext, id: DisplayId) -> Self {
        Self {
            context,
            id,
            closed: AtomicBool::new(false),
        }
    }

    fn info(&self) -> PyResult<DisplayInfo> {
        check_cancel(&self.context)?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(PyRuntimeError::new_err("display is closed"));
        }
        self.context.with_app_result(|app| app.display(self.id))
    }
}

#[pymethods]
impl PyDisplay {
    #[getter]
    fn id(&self) -> u64 {
        self.id.get()
    }
    #[getter]
    fn parent_id(&self) -> PyResult<Option<u64>> {
        Ok(self.info()?.parent.map(DisplayId::get))
    }
    #[getter]
    fn kind(&self) -> PyResult<PyDisplayKind> {
        Ok(PyDisplayKind::from_rust(self.info()?.kind))
    }
    #[getter]
    fn name(&self) -> PyResult<String> {
        Ok(self.info()?.name)
    }
    #[getter]
    fn visible(&self) -> PyResult<bool> {
        Ok(self.info()?.visible)
    }

    fn show(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        self.context
            .with_app_result(|app| app.set_display_visible(self.id, true))?;
        Ok(())
    }

    fn hide(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        self.context
            .with_app_result(|app| app.set_display_visible(self.id, false))?;
        Ok(())
    }

    fn close(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.context
                .with_app_result(|app| app.close_display(self.id))?;
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!("Display(id={})", self.id.get())
    }
}

// ---------------------------------------------------------------------- App ---

#[pyclass(module = "fft_audio", name = "App")]
pub struct PyApp {
    context: RunContext,
}

#[pymethods]
impl PyApp {
    #[new]
    fn new() -> PyResult<Self> {
        // `App()` resolves the run that is currently executing. Binding the
        // context at construction also keeps a retained `App` from accidentally
        // operating on a later, unrelated run.
        Ok(Self {
            context: current()?,
        })
    }

    #[pyo3(signature = (path, *, name=None))]
    fn open_file(&self, path: PathBuf, name: Option<String>) -> PyResult<PySignal> {
        let context = self.context.clone();
        check_cancel(&context)?;
        let runtime = context.runtime()?;
        let mut request = LoadFileRequest::new(path);
        if let Some(name) = name {
            request = request.with_name(name);
        }
        let signal = runtime.load_file(request).map_err(map_runtime_error)?;
        if let Some(job) = runtime.job_for(EntityId::Signal(signal)) {
            poll_wait(&context, job, None)?;
        }
        let info = runtime.signal_info(signal).map_err(map_runtime_error)?;
        Ok(PySignal::new(context, signal, info))
    }

    #[pyo3(signature = (samples, *, sample_rate, name=None))]
    fn load_signal(
        &self,
        samples: &Bound<'_, PyAny>,
        sample_rate: f64,
        name: Option<String>,
    ) -> PyResult<PySignal> {
        let context = self.context.clone();
        check_cancel(&context)?;
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return Err(PyValueError::new_err(
                "sample_rate must be a positive finite number",
            ));
        }
        let samples = extract_samples(samples)?;
        let runtime = context.runtime()?;
        let mut request = LoadSignalRequest::new(samples, sample_rate);
        if let Some(name) = name {
            request = request.with_name(name);
        }
        let signal = runtime.load_signal(request).map_err(map_runtime_error)?;
        let info = runtime.signal_info(signal).map_err(map_runtime_error)?;
        Ok(PySignal::new(context, signal, info))
    }

    fn displays<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let context = self.context.clone();
        check_cancel(&context)?;
        let displays = context.with_app_result(|app| app.displays())?;
        let items: Vec<PyDisplay> = displays
            .into_iter()
            .map(|info| PyDisplay::new(context.clone(), info.id))
            .collect();
        PyTuple::new(py, items)
    }

    #[pyo3(signature = (timeout_s=None))]
    fn wait_all(&self, timeout_s: Option<f64>) -> PyResult<()> {
        let context = self.context.clone();
        let timeout = parse_timeout(timeout_s)?;
        poll_wait_all(&context, timeout)?;
        check_cancel(&context)?;
        // Every runtime job is terminal now; the app-level pass only resolves
        // display associations and must not start a second timeout window.
        context.with_app_result(|app| app.wait_all(None))?;
        Ok(())
    }

    fn close_all(&self) -> PyResult<()> {
        let context = self.context.clone();
        check_cancel(&context)?;
        for info in context.with_app_result(|app| app.displays())? {
            context.with_app_result(|app| app.close_display(info.id))?;
        }
        Ok(())
    }

    fn info(&self) -> PyResult<PyAppInfo> {
        let context = self.context.clone();
        check_cancel(&context)?;
        let info = context.with_app(|app| app.info())?;
        Ok(PyAppInfo::from(&info))
    }

    fn version(&self) -> PyResult<String> {
        let context = self.context.clone();
        check_cancel(&context)?;
        Ok(context.with_app(|app| app.info())?.version)
    }

    fn runtime_name(&self) -> PyResult<String> {
        Ok(self.info()?.name)
    }

    fn __repr__(&self) -> String {
        "App()".to_string()
    }
}

// ------------------------------------------------------------------- Signal ---

#[pyclass(module = "fft_audio", name = "Signal")]
pub struct PySignal {
    context: RunContext,
    id: SignalId,
    info: SignalInfo,
    closed: AtomicBool,
}

impl PySignal {
    fn new(context: RunContext, id: SignalId, info: SignalInfo) -> Self {
        Self {
            context,
            id,
            info,
            closed: AtomicBool::new(false),
        }
    }

    fn ensure_open(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.load(Ordering::SeqCst) {
            Err(PyRuntimeError::new_err("signal is closed"))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PySignal {
    #[getter]
    fn name(&self) -> PyResult<&str> {
        check_cancel(&self.context)?;
        Ok(&self.info.name)
    }
    #[getter]
    fn sample_rate(&self) -> PyResult<f64> {
        check_cancel(&self.context)?;
        Ok(self.info.sample_rate)
    }
    #[getter]
    fn sample_count(&self) -> PyResult<usize> {
        check_cancel(&self.context)?;
        Ok(self.info.sample_count)
    }
    #[getter]
    fn duration_s(&self) -> PyResult<f64> {
        check_cancel(&self.context)?;
        Ok(self.info.duration_s)
    }

    #[pyo3(signature = (*, fft_size=4096, frame=None, pad=None, hop=None, window=None, freq_range=None, db_range=None, log_freq=None, show=true, name=None))]
    #[allow(clippy::too_many_arguments)]
    fn spectrogram(
        &self,
        fft_size: usize,
        frame: Option<usize>,
        pad: Option<usize>,
        hop: Option<usize>,
        window: Option<&Bound<'_, PyAny>>,
        freq_range: Option<(f64, f64)>,
        db_range: Option<(f64, f64)>,
        log_freq: Option<bool>,
        show: bool,
        name: Option<String>,
    ) -> PyResult<PySpectrogramHandle> {
        self.ensure_open()?;
        let (frame, pad) = resolve_fft_layout(fft_size, frame, pad)?;
        let settings = AnalysisSettings {
            frame,
            hop,
            pad,
            window: window
                .map(extract_window)
                .transpose()?
                .map(|window| window.kind)
                .unwrap_or(WindowKind::Hann),
            ..AnalysisSettings::default()
        };
        settings
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let display_settings = build_display_settings(freq_range, db_range, log_freq)?;
        let runtime = self.context.runtime()?;
        let analysis = runtime
            .analyze(self.id, &settings)
            .map_err(map_runtime_error)?;

        let display_name = name.clone().unwrap_or_else(|| self.info.name.clone());
        let options = SpectrogramDisplayOptions {
            name: Some(display_name.clone()),
            settings: display_settings,
            visible: true,
            ephemeral: true,
        };
        let display = if show {
            Some(self.context.with_app_result(|app| {
                app.show_spectrogram_pending(analysis, Some(self.id), &options)
            })?)
        } else {
            None
        };
        Ok(PySpectrogramHandle {
            context: self.context.clone(),
            signal: self.id,
            analysis,
            settings,
            sample_rate: self.info.sample_rate,
            name: display_name,
            display: Mutex::new(display),
            options,
            data: Mutex::new(None),
            closed: AtomicBool::new(false),
        })
    }

    fn close(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let displays = self.context.with_app_result(|app| app.displays())?;
        for info in displays {
            if info.state.get("signalId").and_then(|value| value.as_u64()) == Some(self.id.get()) {
                let _ = self
                    .context
                    .with_app_result(|app| app.close_display(info.id));
            }
        }
        self.context
            .runtime()?
            .close(EntityId::Signal(self.id))
            .map_err(map_runtime_error)?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "Signal(name={:?}, sample_rate={}, samples={})",
            self.info.name, self.info.sample_rate, self.info.sample_count
        )
    }
}

// ------------------------------------------------------- SpectrogramHandle ---

#[pyclass(module = "fft_audio", name = "SpectrogramHandle")]
pub struct PySpectrogramHandle {
    context: RunContext,
    signal: SignalId,
    analysis: AnalysisId,
    settings: AnalysisSettings,
    sample_rate: f64,
    name: String,
    display: Mutex<Option<DisplayId>>,
    options: SpectrogramDisplayOptions,
    data: Mutex<Option<Arc<fft_runtime::SpectrogramData>>>,
    closed: AtomicBool,
}

impl PySpectrogramHandle {
    fn ensure_open(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.load(Ordering::SeqCst) {
            Err(PyRuntimeError::new_err("spectrogram handle is closed"))
        } else {
            Ok(())
        }
    }

    fn wait_for_data(&self) -> PyResult<()> {
        wait_entity(&self.context, EntityId::Analysis(self.analysis), None)
    }

    fn data(&self) -> PyResult<Arc<fft_runtime::SpectrogramData>> {
        self.ensure_open()?;
        self.wait_for_data()?;
        let data = self
            .context
            .runtime()?
            .spectrogram_data(self.analysis)
            .map_err(map_runtime_error)?;
        let mut slot = self.data.lock().unwrap_or_else(|error| error.into_inner());
        if slot.is_none() {
            *slot = Some(Arc::clone(&data));
        }
        Ok(data)
    }

    fn display_id(&self) -> Option<DisplayId> {
        *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Parent display for child cuts. The spectrogram may have been created
    /// with `show=False`; a later child graph request lazily publishes the
    /// loading-state parent instead of failing the cut setup.
    fn parent_display(&self) -> PyResult<DisplayId> {
        if let Some(id) = self.display_id() {
            match self.context.with_app_result(|app| app.display(id)) {
                Ok(_) => return Ok(id),
                Err(_) => {
                    *self
                        .display
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = None;
                }
            }
        }
        let id = self.context.with_app_result(|app| {
            app.show_spectrogram_pending(self.analysis, Some(self.signal), &self.options)
        })?;
        *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(id);
        Ok(id)
    }
}

#[pymethods]
impl PySpectrogramHandle {
    #[getter]
    fn info(&self) -> PyResult<PySpectrogramInfo> {
        self.ensure_open()?;
        self.wait_for_data()?;
        let info = self
            .context
            .runtime()?
            .analysis_info(self.analysis)
            .map_err(map_runtime_error)?;
        Ok(PySpectrogramInfo { info })
    }

    #[getter]
    fn display(&self) -> PyResult<Option<PyDisplay>> {
        check_cancel(&self.context)?;
        let Some(id) = self.display_id() else {
            return Ok(None);
        };
        match self.context.with_app_result(|app| app.display(id)) {
            Ok(_) => Ok(Some(PyDisplay::new(self.context.clone(), id))),
            Err(_) => {
                *self
                    .display
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                Ok(None)
            }
        }
    }

    #[getter]
    fn status(&self) -> PyResult<PyJobStatus> {
        check_cancel(&self.context)?;
        Ok(entity_status(
            &self.context,
            EntityId::Analysis(self.analysis),
        ))
    }

    #[getter]
    fn error(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        check_cancel(&self.context)?;
        entity_error(py, &self.context, EntityId::Analysis(self.analysis), false)
    }

    #[pyo3(signature = (*, fundamental_hz=None, rpm=None, orders=None, bandwidth_hz=None, bandwidth_orders=None, axis=None, show_graph=true, name=None))]
    #[allow(clippy::too_many_arguments)]
    fn order_cut(
        &self,
        fundamental_hz: Option<&Bound<'_, PyAny>>,
        rpm: Option<&Bound<'_, PyAny>>,
        orders: Option<&Bound<'_, PyAny>>,
        bandwidth_hz: Option<f64>,
        bandwidth_orders: Option<f64>,
        axis: Option<&Bound<'_, PyAny>>,
        show_graph: bool,
        name: Option<String>,
    ) -> PyResult<PyOrderCutHandle> {
        self.ensure_open()?;
        if fundamental_hz.is_some() && rpm.is_some() {
            return Err(PyValueError::new_err(
                "pass either fundamental_hz or rpm, not both",
            ));
        }
        if bandwidth_hz.is_some() && bandwidth_orders.is_some() {
            return Err(PyValueError::new_err(
                "pass either bandwidth_hz or bandwidth_orders, not both",
            ));
        }

        let mut settings = OrderCutSettings::default();
        let order_values = match orders {
            Some(orders) => float_sequence(orders)?,
            None => vec![1.0],
        };
        if order_values.is_empty() {
            return Err(PyValueError::new_err("orders must not be empty"));
        }
        if order_values
            .iter()
            .any(|order| !order.is_finite() || *order == 0.0)
        {
            return Err(PyValueError::new_err("orders must be finite and non-zero"));
        }
        settings.orders = order_values
            .iter()
            .map(|order| order.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if let Some(hz) = bandwidth_hz {
            if !(hz.is_finite() && hz > 0.0) {
                return Err(PyValueError::new_err("bandwidth_hz must be > 0"));
            }
            settings.order_width_hz = Some(hz);
        } else if let Some(orders) = bandwidth_orders {
            if !(orders.is_finite() && orders > 0.0) {
                return Err(PyValueError::new_err("bandwidth_orders must be > 0"));
            }
            settings.order_width = orders;
        }
        settings.axis = axis
            .map(extract_cut_axis)
            .transpose()?
            .map(|axis| axis.core_axis())
            .unwrap_or(CoreCutAxis::Time);

        let hop_s = if self.sample_rate > 0.0 {
            self.settings.hop_samples() as f64 / self.sample_rate
        } else {
            0.0
        };
        let rpm_override = if let Some(value) = fundamental_hz {
            match numeric_input(value)? {
                NumericInput::Scalar(hz) => {
                    validate_frequency(hz, "fundamental_hz")?;
                    settings.rpm = RpmSettings {
                        source: RpmSource::Constant,
                        constant: hz * 60.0,
                        ..RpmSettings::default()
                    };
                    None
                }
                NumericInput::Series(values) => {
                    if values.is_empty() {
                        return Err(PyValueError::new_err(
                            "fundamental_hz must not be an empty sequence",
                        ));
                    }
                    for hz in &values {
                        validate_frequency(*hz, "fundamental_hz")?;
                    }
                    settings.rpm = RpmSettings {
                        source: RpmSource::Constant,
                        constant: values[0] * 60.0,
                        ..RpmSettings::default()
                    };
                    if values.len() == 1 {
                        None
                    } else {
                        let points = values
                            .iter()
                            .enumerate()
                            .map(|(index, hz)| RpmPoint {
                                time_s: index as f64 * hop_s,
                                rpm: *hz * 60.0,
                            })
                            .collect::<Vec<_>>();
                        Some(fft_runtime::RpmProfileSpec::Points(points))
                    }
                }
            }
        } else if let Some(value) = rpm {
            match numeric_input(value)? {
                NumericInput::Scalar(value) => {
                    validate_frequency(value, "rpm")?;
                    settings.rpm = RpmSettings {
                        source: RpmSource::Constant,
                        constant: value,
                        ..RpmSettings::default()
                    };
                    None
                }
                NumericInput::Series(values) => {
                    if values.is_empty() {
                        return Err(PyValueError::new_err("rpm must not be an empty sequence"));
                    }
                    for value in &values {
                        validate_frequency(*value, "rpm")?;
                    }
                    settings.rpm = RpmSettings {
                        source: RpmSource::Constant,
                        constant: values[0],
                        ..RpmSettings::default()
                    };
                    if values.len() == 1 {
                        None
                    } else {
                        let points = values
                            .iter()
                            .enumerate()
                            .map(|(index, value)| RpmPoint {
                                time_s: index as f64 * hop_s,
                                rpm: *value,
                            })
                            .collect::<Vec<_>>();
                        Some(fft_runtime::RpmProfileSpec::Points(points))
                    }
                }
            }
        } else {
            None
        };

        let request = OrderCutRequest {
            settings: settings.clone(),
            rpm: rpm_override.clone(),
        };
        let runtime = self.context.runtime()?;
        let cut = runtime
            .order_cut(self.analysis, &request)
            .map_err(map_runtime_error)?;
        let display = if show_graph {
            let parent = self.parent_display().inspect_err(|_| {
                let _ = runtime.close(EntityId::Cut(cut));
            })?;
            let mut options = CutDisplayOptions::order(settings.clone())
                .with_name(name.clone().unwrap_or_else(|| "Order Cuts".to_string()))
                .with_ephemeral(true);
            if let Some(rpm) = rpm_override.clone() {
                options = options.with_rpm(rpm);
            }
            Some(
                match self
                    .context
                    .with_app_result(|app| app.show_order_cut_pending(parent, cut, &options))
                {
                    Ok(display) => display,
                    Err(error) => {
                        let _ = runtime.close(EntityId::Cut(cut));
                        return Err(error);
                    }
                },
            )
        } else {
            None
        };
        Ok(PyOrderCutHandle {
            context: self.context.clone(),
            cut: Mutex::new(cut),
            name: name.unwrap_or_else(|| "order cuts".to_string()),
            display: Mutex::new(display),
            state: Mutex::new(OrderState::default()),
            closed: AtomicBool::new(false),
        })
    }

    #[pyo3(signature = (*, start, end, num_points=256, log=false, show_graph=true, name=None))]
    fn line_cut(
        &self,
        start: (f64, f64),
        end: (f64, f64),
        num_points: usize,
        log: bool,
        show_graph: bool,
        name: Option<String>,
    ) -> PyResult<PyLineCutHandle> {
        self.ensure_open()?;
        if num_points == 0 || num_points > 100_000 {
            return Err(PyValueError::new_err(
                "num_points must be between 1 and 100000",
            ));
        }
        if !(start.0.is_finite() && start.1.is_finite() && end.0.is_finite() && end.1.is_finite()) {
            return Err(PyValueError::new_err(
                "start and end must contain finite time/frequency values",
            ));
        }
        let line = LineSpec {
            t0: start.0,
            f0: start.1,
            t1: end.0,
            f1: end.1,
            log_f: log,
        };
        let mut settings = LineCutSettings {
            samples: Some(num_points),
            ..LineCutSettings::default()
        };
        settings.line_axis = fft_core::settings::LineAxis::Auto;
        let request = LineCutRequest::new(settings.clone(), vec![line.clone()]);
        let runtime = self.context.runtime()?;
        let cut = runtime
            .line_cut(self.analysis, &request)
            .map_err(map_runtime_error)?;
        let display = if show_graph {
            let parent = self.parent_display().inspect_err(|_| {
                let _ = runtime.close(EntityId::Cut(cut));
            })?;
            let options = CutDisplayOptions::line(settings.clone())
                .with_name(name.clone().unwrap_or_else(|| "Line Cuts".to_string()))
                .with_ephemeral(true);
            Some(
                match self
                    .context
                    .with_app_result(|app| app.show_line_cut_pending(parent, cut, &options))
                {
                    Ok(display) => display,
                    Err(error) => {
                        let _ = runtime.close(EntityId::Cut(cut));
                        return Err(error);
                    }
                },
            )
        } else {
            None
        };
        Ok(PyLineCutHandle {
            context: self.context.clone(),
            analysis: self.analysis,
            cut: Mutex::new(cut),
            name: name.unwrap_or_else(|| "line cut".to_string()),
            display: Mutex::new(display),
            parent_display: Mutex::new(if show_graph { self.display_id() } else { None }),
            line: Mutex::new(line),
            settings: Mutex::new(settings),
            state: Mutex::new(LineState::default()),
            closed: AtomicBool::new(false),
        })
    }

    fn as_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let data = slf.borrow().data()?;
        let base = slf.clone().into_any();
        Ok(views::readonly_2d(
            data.data(),
            data.frames(),
            data.bins(),
            base,
        ))
    }

    fn to_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let py = slf.py();
        let data = slf.borrow().data()?;
        let values = data.data().to_vec();
        views::owned_2d(py, values, data.frames(), data.bins())
    }

    /// Canonical spectrogram data is shared with displays and child cuts, so a
    /// mutable view is never allowed. Call `to_numpy()` for a mutable copy.
    fn as_numpy_mut(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        Err(DataBorrowedError::new_err(
            "spectrogram data is read-only; use to_numpy() for a mutable copy",
        ))
    }

    /// Spectrogram ownership stays with the runtime/display; use `to_numpy()`.
    #[allow(clippy::wrong_self_convention)]
    fn into_numpy(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        Err(DataBorrowedError::new_err(
            "spectrogram data is read-only and cannot be detached; use to_numpy()",
        ))
    }

    #[pyo3(signature = (path, *, width=1600, height=900))]
    fn save_image(
        &self,
        py: Python<'_>,
        path: PathBuf,
        width: usize,
        height: usize,
    ) -> PyResult<Py<PyAny>> {
        self.ensure_open()?;
        self.wait_for_data()?;
        if width == 0 || height == 0 {
            return Err(PyValueError::new_err(
                "width and height must be greater than zero",
            ));
        }
        let display = self.options.settings.clone();
        let options = RenderOptions {
            width,
            height,
            log_freq: display.log_freq,
            min_hz: display.min_hz.unwrap_or(20.0),
            max_hz: display.max_hz,
            dynamic_range_db: display.range_db,
            db_min: display.db_min,
            db_max: display.db_max,
        };
        self.context
            .runtime()?
            .save_image(self.analysis, &path, &options)
            .map_err(map_runtime_error)?;
        path_to_py(py, &path)
    }

    #[pyo3(signature = (timeout_s=None))]
    fn wait(&self, timeout_s: Option<f64>) -> PyResult<()> {
        self.ensure_open()?;
        let timeout = parse_timeout(timeout_s)?;
        wait_entity(&self.context, EntityId::Analysis(self.analysis), timeout)
    }

    fn close(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(display) = self.display_id() {
            let _ = self
                .context
                .with_app_result(|app| app.close_display(display));
        }
        if let Ok(runtime) = self.context.runtime() {
            let _ = runtime.close(EntityId::Analysis(self.analysis));
        }
        // Keep the cached `Arc` alive: NumPy views created earlier use this
        // handle as their base object.
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "SpectrogramHandle(name={:?}, analysis={})",
            self.name,
            self.analysis.get()
        )
    }
}

// ------------------------------------------------------------ OrderCutHandle ---

#[derive(Default)]
struct OrderState {
    data: Option<Arc<CutData>>,
    owned: Option<OwnedOrder>,
    detached: bool,
    borrowed_mut: bool,
    /// Set when a read view or info snapshot may alias the result payload.
    read_borrowed: bool,
    /// Weak reference to the active writable NumPy array (no reference cycle:
    /// the array's base object is this handle).
    writable: Option<Py<PyWeakrefReference>>,
}

struct OwnedOrder {
    frames: usize,
    order_count: usize,
    values: Vec<f32>,
}

impl OwnedOrder {
    fn from_data(data: CutData) -> PyResult<Self> {
        let CutData::Order(data) = data else {
            return Err(PyRuntimeError::new_err("cut is not an order cut"));
        };
        let values = Arc::try_unwrap(data.values).map_err(|_| {
            DataBorrowedError::new_err(
                "order cut values are still borrowed; close read views before taking ownership",
            )
        })?;
        Ok(Self {
            frames: data.frames,
            order_count: data.order_count,
            values,
        })
    }
}

#[pyclass(module = "fft_audio", name = "OrderCutHandle")]
pub struct PyOrderCutHandle {
    context: RunContext,
    cut: Mutex<CutId>,
    name: String,
    display: Mutex<Option<DisplayId>>,
    state: Mutex<OrderState>,
    closed: AtomicBool,
}

impl PyOrderCutHandle {
    fn cut_id(&self) -> CutId {
        *self.cut.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn ensure_alive(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(PyRuntimeError::new_err("order cut handle is closed"));
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.detached {
            return Err(DataDetachedError::new_err(
                "cut data was detached by into_numpy()",
            ));
        }
        if state.borrowed_mut {
            return Err(DataBorrowedError::new_err(
                "cut data is under an exclusive mutable lease",
            ));
        }
        Ok(())
    }

    fn wait_for_data(&self) -> PyResult<()> {
        wait_entity(&self.context, EntityId::Cut(self.cut_id()), None)
    }

    /// Fetch the current snapshot without retaining it in the handle.
    fn fetch_data(&self) -> PyResult<Arc<CutData>> {
        self.ensure_alive()?;
        self.wait_for_data()?;
        self.context
            .runtime()?
            .cut_data(self.cut_id())
            .map_err(map_runtime_error)
    }

    /// Cache the read-only `Arc` snapshot used as the base for NumPy views.
    fn readonly_data(&self) -> PyResult<Arc<CutData>> {
        let data = self.fetch_data()?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.data.is_none() {
            state.data = Some(Arc::clone(&data));
        }
        state.read_borrowed = true;
        Ok(data)
    }

    fn order_data<'a>(&self, data: &'a CutData) -> PyResult<&'a OrderCutData> {
        match data {
            CutData::Order(data) => Ok(data),
            CutData::Line(_) => Err(PyRuntimeError::new_err("cut is not an order cut")),
        }
    }

    fn has_display(&self) -> bool {
        self.display
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    fn reject_read_aliases(&self) -> PyResult<()> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.read_borrowed || state.data.is_some() {
            return Err(DataBorrowedError::new_err(
                "cut data has read views or info snapshots; close the handle before taking ownership",
            ));
        }
        Ok(())
    }

    fn mark_read_borrowed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.read_borrowed = true;
    }

    /// Take the unique runtime buffer and map a borrowed result to the Python
    /// lease error instead of exposing an implicit copy.
    fn take_unique_data(&self) -> PyResult<CutData> {
        self.wait_for_data()?;
        self.context
            .runtime()?
            .take_cut_data(self.cut_id())
            .map_err(|error| {
                if error.code() == "busy" {
                    DataBorrowedError::new_err(error.message().to_string())
                } else {
                    map_runtime_error(error)
                }
            })
    }

    fn release_writable(&self, py: Python<'_>) {
        let weak = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.borrowed_mut = false;
            state.writable.take()
        };
        if let Some(weak) = weak {
            if let Some(target) = weak.bind(py).upgrade() {
                if let Ok(array) = target.cast::<PyArray2<f32>>() {
                    views::mark_readonly(array);
                }
            }
        }
    }
}

#[pymethods]
impl PyOrderCutHandle {
    #[getter]
    fn info(&self) -> PyResult<PyOrderCutInfo> {
        let data = self.fetch_data()?;
        let order = self.order_data(&data)?;
        self.mark_read_borrowed();
        Ok(PyOrderCutInfo {
            name: self.name.clone(),
            data: Arc::new(order.clone()),
        })
    }

    #[getter]
    fn display(&self) -> PyResult<Option<PyDisplay>> {
        check_cancel(&self.context)?;
        let id = *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(id) = id else {
            return Ok(None);
        };
        match self.context.with_app_result(|app| app.display(id)) {
            Ok(_) => Ok(Some(PyDisplay::new(self.context.clone(), id))),
            Err(_) => {
                *self
                    .display
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                Ok(None)
            }
        }
    }

    #[getter]
    fn status(&self) -> PyResult<PyJobStatus> {
        check_cancel(&self.context)?;
        Ok(entity_status(&self.context, EntityId::Cut(self.cut_id())))
    }

    #[getter]
    fn error(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        check_cancel(&self.context)?;
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.borrowed_mut {
            return Ok(Some(error_instance(
                py,
                DataBorrowedError::new_err("cut data is under an exclusive mutable lease"),
            )));
        }
        let detached = state.detached;
        drop(state);
        entity_error(py, &self.context, EntityId::Cut(self.cut_id()), detached)
    }

    fn as_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let data = slf.borrow().readonly_data()?;
        let order = match data.as_ref() {
            CutData::Order(order) => order,
            CutData::Line(_) => {
                return Err(PyRuntimeError::new_err("cut is not an order cut"));
            }
        };
        let base = slf.clone().into_any();
        Ok(views::readonly_2d(
            order.values.as_slice(),
            order.frames,
            order.order_count,
            base,
        ))
    }

    fn to_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let py = slf.py();
        let data = slf.borrow().fetch_data()?;
        let order = match data.as_ref() {
            CutData::Order(order) => order,
            CutData::Line(_) => {
                return Err(PyRuntimeError::new_err("cut is not an order cut"));
            }
        };
        views::owned_2d(
            py,
            order.values.as_ref().clone(),
            order.frames,
            order.order_count,
        )
    }

    /// Exclusive zero-copy lease. A second call, any prior read view/info, or a
    /// live cut display is rejected. Other data methods raise
    /// `DataBorrowedError` until `close()` (or handle drop), which clears the
    /// lease and makes the NumPy array read-only again.
    fn as_numpy_mut<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let this = slf.borrow();
        check_cancel(&this.context)?;
        if this.closed.load(Ordering::SeqCst) {
            return Err(PyRuntimeError::new_err("order cut handle is closed"));
        }
        if this.has_display() {
            return Err(DataBorrowedError::new_err(
                "cut data is used by a display; use to_numpy() for a mutable copy",
            ));
        }
        let needs_take = {
            let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.detached {
                return Err(DataDetachedError::new_err(
                    "cut data was detached by into_numpy()",
                ));
            }
            if state.borrowed_mut {
                return Err(DataBorrowedError::new_err(
                    "cut data already has an exclusive mutable lease",
                ));
            }
            if state.read_borrowed || state.data.is_some() {
                return Err(DataBorrowedError::new_err(
                    "cut data has read views or info snapshots; close the handle before leasing it mutably",
                ));
            }
            if state.owned.is_none() {
                state.borrowed_mut = true;
                true
            } else {
                false
            }
        };
        if needs_take {
            let data = match this.take_unique_data() {
                Ok(data) => data,
                Err(error) => {
                    let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
                    state.borrowed_mut = false;
                    return Err(error);
                }
            };
            let owned = match OwnedOrder::from_data(data) {
                Ok(owned) => owned,
                Err(error) => {
                    let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
                    state.borrowed_mut = false;
                    return Err(error);
                }
            };
            let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
            state.owned = Some(owned);
        }
        let base = slf.clone().into_any();
        let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
        let owned = state
            .owned
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("order cut data is not owned"))?;
        let array = views::mutable_2d(
            owned.values.as_mut_slice(),
            owned.frames,
            owned.order_count,
            base,
        );
        match PyWeakrefReference::new(array.as_any()) {
            Ok(weak) => {
                state.writable = Some(weak.unbind());
                Ok(array)
            }
            Err(error) => {
                views::mark_readonly(&array);
                state.borrowed_mut = false;
                state.writable = None;
                Err(error)
            }
        }
    }

    fn into_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let py = slf.py();
        let this = slf.borrow();
        this.ensure_alive()?;
        if this.has_display() {
            return Err(DataBorrowedError::new_err(
                "cut data is used by a display; use to_numpy() instead of detaching it",
            ));
        }
        this.reject_read_aliases()?;
        let data = this.take_unique_data()?;
        let owned = OwnedOrder::from_data(data)?;
        {
            let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
            state.detached = true;
            state.read_borrowed = false;
            state.data = None;
        }
        drop(this);
        views::owned_2d(py, owned.values, owned.frames, owned.order_count)
    }

    fn export_csv(&self, py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
        self.ensure_alive()?;
        self.wait_for_data()?;
        self.context
            .runtime()?
            .export_csv(self.cut_id(), &path)
            .map_err(map_runtime_error)?;
        path_to_py(py, &path)
    }

    #[pyo3(signature = (timeout_s=None))]
    fn wait(&self, timeout_s: Option<f64>) -> PyResult<()> {
        self.ensure_alive()?;
        let timeout = parse_timeout(timeout_s)?;
        wait_entity(&self.context, EntityId::Cut(self.cut_id()), timeout)
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        check_cancel(&self.context)?;
        self.release_writable(py);
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(display) = *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            let _ = self
                .context
                .with_app_result(|app| app.close_display(display));
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.detached && state.owned.is_none() {
            if let Ok(runtime) = self.context.runtime() {
                let _ = runtime.close(EntityId::Cut(self.cut_id()));
            }
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "OrderCutHandle(name={:?}, cut={})",
            self.name,
            self.cut_id().get()
        )
    }
}

impl Drop for PyOrderCutHandle {
    fn drop(&mut self) {
        let weak = {
            let Ok(state) = self.state.get_mut() else {
                return;
            };
            state.borrowed_mut = false;
            state.writable.take()
        };
        if let Some(weak) = weak {
            let _ = Python::try_attach(|py| {
                if let Some(target) = weak.bind(py).upgrade() {
                    if let Ok(array) = target.cast::<PyArray2<f32>>() {
                        views::mark_readonly(array);
                    }
                }
            });
        }
    }
}

// ------------------------------------------------------------- LineCutHandle ---

#[derive(Default)]
struct LineState {
    data: Option<Arc<CutData>>,
    /// Old snapshots kept alive for NumPy views returned before `set_line`.
    retired: Vec<Arc<CutData>>,
    owned: Option<OwnedLine>,
    detached: bool,
    borrowed_mut: bool,
    /// Set when a read view or info snapshot may alias the result payload.
    read_borrowed: bool,
}

struct OwnedLine {
    times: Vec<f64>,
    db: Vec<f32>,
}

impl OwnedLine {
    fn from_data(data: LineCutData) -> PyResult<Self> {
        let times = Arc::try_unwrap(data.times).map_err(|_| {
            DataBorrowedError::new_err(
                "line cut times are still borrowed; close read views before taking ownership",
            )
        })?;
        let db = Arc::try_unwrap(data.db).map_err(|_| {
            DataBorrowedError::new_err(
                "line cut levels are still borrowed; close read views before taking ownership",
            )
        })?;
        Ok(Self { times, db })
    }
}

type LineArrayPair<'py> = (Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<f32>>);

#[pyclass(module = "fft_audio", name = "LineCutHandle")]
pub struct PyLineCutHandle {
    context: RunContext,
    analysis: AnalysisId,
    cut: Mutex<CutId>,
    name: String,
    display: Mutex<Option<DisplayId>>,
    parent_display: Mutex<Option<DisplayId>>,
    line: Mutex<LineSpec>,
    settings: Mutex<LineCutSettings>,
    state: Mutex<LineState>,
    closed: AtomicBool,
}

impl PyLineCutHandle {
    fn cut_id(&self) -> CutId {
        *self.cut.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn ensure_alive(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(PyRuntimeError::new_err("line cut handle is closed"));
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.detached {
            return Err(DataDetachedError::new_err(
                "cut data was detached by into_numpy()",
            ));
        }
        if state.borrowed_mut {
            return Err(DataBorrowedError::new_err(
                "cut data is under an exclusive mutable lease",
            ));
        }
        Ok(())
    }

    fn wait_for_data(&self) -> PyResult<()> {
        wait_entity(&self.context, EntityId::Cut(self.cut_id()), None)
    }

    fn fetch_track(&self) -> PyResult<(Arc<CutData>, Arc<LineCutData>)> {
        self.ensure_alive()?;
        self.wait_for_data()?;
        let data = self
            .context
            .runtime()?
            .cut_data(self.cut_id())
            .map_err(map_runtime_error)?;
        let track = match data.as_ref() {
            CutData::Line(tracks) => tracks
                .first()
                .cloned()
                .ok_or_else(|| PyRuntimeError::new_err("line cut has no tracks"))?,
            CutData::Order(_) => {
                return Err(PyRuntimeError::new_err("cut is not a line cut"));
            }
        };
        Ok((data, Arc::new(track)))
    }

    fn readonly_track(&self) -> PyResult<Arc<LineCutData>> {
        let (data, track) = self.fetch_track()?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.data.is_none() {
            state.data = Some(data);
        }
        state.read_borrowed = true;
        Ok(track)
    }

    fn check_mutable(&self) -> PyResult<()> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.borrowed_mut {
            return Err(DataBorrowedError::new_err(
                "cut data is under an exclusive mutable lease",
            ));
        }
        Ok(())
    }

    fn has_display(&self) -> bool {
        self.display
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    fn reject_read_aliases(&self) -> PyResult<()> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.read_borrowed || state.data.is_some() {
            return Err(DataBorrowedError::new_err(
                "cut data has read views or info snapshots; close the handle before taking ownership",
            ));
        }
        Ok(())
    }

    fn take_unique_data(&self) -> PyResult<CutData> {
        self.wait_for_data()?;
        self.context
            .runtime()?
            .take_cut_data(self.cut_id())
            .map_err(|error| {
                if error.code() == "busy" {
                    DataBorrowedError::new_err(error.message().to_string())
                } else {
                    map_runtime_error(error)
                }
            })
    }
}

#[pymethods]
impl PyLineCutHandle {
    #[getter]
    fn info(&self) -> PyResult<PyLineCutInfo> {
        let (_, track) = self.fetch_track()?;
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.read_borrowed = true;
        }
        Ok(PyLineCutInfo {
            name: self.name.clone(),
            data: track,
        })
    }

    #[getter]
    fn display(&self) -> PyResult<Option<PyDisplay>> {
        check_cancel(&self.context)?;
        let id = *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(id) = id else {
            return Ok(None);
        };
        match self.context.with_app_result(|app| app.display(id)) {
            Ok(_) => Ok(Some(PyDisplay::new(self.context.clone(), id))),
            Err(_) => {
                *self
                    .display
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                Ok(None)
            }
        }
    }

    #[getter]
    fn status(&self) -> PyResult<PyJobStatus> {
        check_cancel(&self.context)?;
        Ok(entity_status(&self.context, EntityId::Cut(self.cut_id())))
    }

    #[getter]
    fn error(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        check_cancel(&self.context)?;
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.borrowed_mut {
            return Ok(Some(error_instance(
                py,
                DataBorrowedError::new_err("cut data is under an exclusive mutable lease"),
            )));
        }
        let detached = state.detached;
        drop(state);
        entity_error(py, &self.context, EntityId::Cut(self.cut_id()), detached)
    }

    fn as_arrays<'py>(slf: Bound<'py, Self>) -> PyResult<LineArrayPair<'py>> {
        let track = slf.borrow().readonly_track()?;
        let base_times = slf.clone().into_any();
        let base_db = slf.clone().into_any();
        Ok((
            views::readonly_1d(track.times.as_slice(), base_times),
            views::readonly_1d(track.db.as_slice(), base_db),
        ))
    }

    fn to_arrays<'py>(slf: Bound<'py, Self>) -> PyResult<LineArrayPair<'py>> {
        let py = slf.py();
        let (_, track) = slf.borrow().fetch_track()?;
        Ok((
            views::owned_1d(py, track.times.as_ref().clone()),
            views::owned_1d(py, track.db.as_ref().clone()),
        ))
    }

    fn to_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f64>>> {
        let py = slf.py();
        let (_, track) = slf.borrow().fetch_track()?;
        let mut values = Vec::with_capacity(track.len() * 2);
        for index in 0..track.len() {
            values.push(track.times[index]);
            values.push(track.db[index] as f64);
        }
        views::owned_2d(py, values, track.len(), 2)
    }

    #[pyo3(signature = (*, start, end, num_points=None))]
    fn set_line(
        &self,
        start: (f64, f64),
        end: (f64, f64),
        num_points: Option<usize>,
    ) -> PyResult<()> {
        self.ensure_alive()?;
        self.check_mutable()?;
        if !(start.0.is_finite() && start.1.is_finite() && end.0.is_finite() && end.1.is_finite()) {
            return Err(PyValueError::new_err(
                "start and end must contain finite time/frequency values",
            ));
        }
        if let Some(points) = num_points {
            if points == 0 || points > 100_000 {
                return Err(PyValueError::new_err(
                    "num_points must be between 1 and 100000",
                ));
            }
        }
        let line = LineSpec {
            t0: start.0,
            f0: start.1,
            t1: end.0,
            f1: end.1,
            log_f: self
                .line
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .log_f,
        };
        let settings = {
            let mut settings = self
                .settings
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if let Some(points) = num_points {
                settings.samples = Some(points);
            }
            settings
        };
        let request = LineCutRequest::new(settings.clone(), vec![line.clone()]);
        let runtime = self.context.runtime()?;
        let new_cut = runtime
            .line_cut(self.analysis, &request)
            .map_err(map_runtime_error)?;
        let old_cut = self.cut_id();
        let old_display = *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Publish the replacement display before removing the old one. A
        // failure then leaves the original cut/display fully usable.
        let new_display = if old_display.is_some() {
            let parent = *self
                .parent_display
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(parent) = parent else {
                let _ = runtime.close(EntityId::Cut(new_cut));
                return Err(PyRuntimeError::new_err("line cut has no parent display"));
            };
            let options = CutDisplayOptions::line(settings.clone())
                .with_name(self.name.clone())
                .with_ephemeral(true);
            Some(
                match self
                    .context
                    .with_app_result(|app| app.show_line_cut_pending(parent, new_cut, &options))
                {
                    Ok(display) => display,
                    Err(error) => {
                        let _ = runtime.close(EntityId::Cut(new_cut));
                        return Err(error);
                    }
                },
            )
        } else {
            None
        };
        if let Some(display) = old_display {
            self.context
                .with_app_result(|app| app.close_display(display))?;
        }
        let _ = runtime.close(EntityId::Cut(old_cut));
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(old) = state.data.take() {
                state.retired.push(old);
            }
        }
        *self.cut.lock().unwrap_or_else(|error| error.into_inner()) = new_cut;
        *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = new_display;
        *self.line.lock().unwrap_or_else(|error| error.into_inner()) = line;
        *self
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = settings;
        Ok(())
    }

    fn into_numpy<'py>(slf: Bound<'py, Self>) -> PyResult<Bound<'py, PyArray2<f64>>> {
        let py = slf.py();
        let this = slf.borrow();
        this.ensure_alive()?;
        if this.has_display() {
            return Err(DataBorrowedError::new_err(
                "cut data is used by a display; use to_numpy() instead of detaching it",
            ));
        }
        this.reject_read_aliases()?;
        let data = this.take_unique_data()?;
        let track = match data {
            CutData::Line(tracks) => tracks
                .into_iter()
                .next()
                .ok_or_else(|| PyRuntimeError::new_err("line cut has no tracks"))?,
            CutData::Order(_) => {
                return Err(PyRuntimeError::new_err("cut is not a line cut"));
            }
        };
        let owned = OwnedLine::from_data(track)?;
        {
            let mut state = this.state.lock().unwrap_or_else(|error| error.into_inner());
            state.detached = true;
            state.read_borrowed = false;
            state.data = None;
        }
        // Line results store time/dB in separate buffers; interleaving them into
        // the documented (N, 2) ownership-transfer array is the one unavoidable
        // allocation here. It happens only after the unique runtime buffer was
        // taken, so no read view is aliased or silently copied.
        let mut values = Vec::with_capacity(owned.times.len() * 2);
        for index in 0..owned.times.len() {
            values.push(owned.times[index]);
            values.push(owned.db[index] as f64);
        }
        drop(this);
        views::owned_2d(py, values, owned.times.len(), 2)
    }

    fn export_csv(&self, py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
        self.ensure_alive()?;
        self.wait_for_data()?;
        self.context
            .runtime()?
            .export_csv(self.cut_id(), &path)
            .map_err(map_runtime_error)?;
        path_to_py(py, &path)
    }

    #[pyo3(signature = (timeout_s=None))]
    fn wait(&self, timeout_s: Option<f64>) -> PyResult<()> {
        self.ensure_alive()?;
        let timeout = parse_timeout(timeout_s)?;
        wait_entity(&self.context, EntityId::Cut(self.cut_id()), timeout)
    }

    fn close(&self) -> PyResult<()> {
        check_cancel(&self.context)?;
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(display) = *self
            .display
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            let _ = self
                .context
                .with_app_result(|app| app.close_display(display));
        }
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.detached && state.owned.is_none() {
            if let Ok(runtime) = self.context.runtime() {
                let _ = runtime.close(EntityId::Cut(self.cut_id()));
            }
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "LineCutHandle(name={:?}, cut={})",
            self.name,
            self.cut_id().get()
        )
    }
}

// ------------------------------------------------------------- input helpers ---

enum NumericInput {
    Scalar(f64),
    Series(Vec<f64>),
}

fn extract_samples(samples: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
    if let Ok(array) = samples.cast::<PyArray1<f32>>() {
        // `to_vec` on the ndarray view copies strided/non-contiguous inputs;
        // `as_slice` would reject them.
        return Ok(array.readonly().as_array().to_vec());
    }
    if let Ok(array) = samples.cast::<PyArray1<f64>>() {
        return Ok(array
            .readonly()
            .as_array()
            .iter()
            .map(|value| *value as f32)
            .collect());
    }
    let mut values = Vec::new();
    for item in samples.try_iter()? {
        values.push(float_value(&item?)? as f32);
    }
    Ok(values)
}

fn float_value(value: &Bound<'_, PyAny>) -> PyResult<f64> {
    if let Ok(value) = value.extract::<f64>() {
        return Ok(value);
    }
    if let Ok(value) = value.extract::<f32>() {
        return Ok(value as f64);
    }
    let converted = value.call_method0("__float__")?;
    converted.extract::<f64>()
}

fn float_sequence(value: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    let mut values = Vec::new();
    for item in value.try_iter()? {
        values.push(float_value(&item?)?);
    }
    Ok(values)
}

fn numeric_input(value: &Bound<'_, PyAny>) -> PyResult<NumericInput> {
    if let Ok(value) = value.extract::<f64>() {
        return Ok(NumericInput::Scalar(value));
    }
    if let Ok(length) = value.len() {
        let mut values = Vec::with_capacity(length);
        for item in value.try_iter()? {
            values.push(float_value(&item?)?);
        }
        return Ok(NumericInput::Series(values));
    }
    Ok(NumericInput::Scalar(float_value(value)?))
}

fn validate_frequency(value: f64, label: &str) -> PyResult<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(PyValueError::new_err(format!(
            "{label} must be a finite number"
        )))
    }
}

fn resolve_fft_layout(
    fft_size: usize,
    frame: Option<usize>,
    pad: Option<usize>,
) -> PyResult<(usize, usize)> {
    if fft_size < 16 {
        return Err(PyValueError::new_err("fft_size must be at least 16"));
    }
    let (frame, pad) = match (frame, pad) {
        (Some(frame), None) => {
            if frame == 0 || !fft_size.is_multiple_of(frame) {
                return Err(PyValueError::new_err("frame must divide fft_size exactly"));
            }
            (frame, fft_size / frame)
        }
        (None, Some(pad)) => {
            if pad == 0 || !fft_size.is_multiple_of(pad) {
                return Err(PyValueError::new_err("pad must divide fft_size exactly"));
            }
            (fft_size / pad, pad)
        }
        (Some(frame), Some(pad)) => {
            if frame == 0 || pad == 0 || frame.saturating_mul(pad) != fft_size {
                return Err(PyValueError::new_err("frame * pad must equal fft_size"));
            }
            (frame, pad)
        }
        (None, None) => (fft_size, 1),
    };
    Ok((frame, pad))
}

fn build_display_settings(
    freq_range: Option<(f64, f64)>,
    db_range: Option<(f64, f64)>,
    log_freq: Option<bool>,
) -> PyResult<DisplaySettings> {
    let mut settings = DisplaySettings::default();
    if let Some((min, max)) = freq_range {
        settings.min_hz = Some(min as f32);
        settings.max_hz = Some(max as f32);
    }
    if let Some((min, max)) = db_range {
        settings.db_min = Some(min as f32);
        settings.db_max = Some(max as f32);
        settings.range_db = (max - min) as f32;
    }
    if let Some(log_freq) = log_freq {
        settings.log_freq = log_freq;
    }
    settings
        .validate()
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(settings)
}

fn path_to_py(py: Python<'_>, path: &Path) -> PyResult<Py<PyAny>> {
    let path_type = py.import("pathlib")?.getattr("Path")?;
    Ok(path_type
        .call1((path.to_string_lossy().into_owned(),))?
        .unbind())
}

// ------------------------------------------------------------ stdout capture ---

#[pyclass(module = "fft_audio", name = "_ScriptWriter")]
struct ScriptWriter {
    run: ScriptRunId,
    stream: ScriptStream,
    events: Arc<dyn ScriptEventSink>,
    buffer: Mutex<String>,
}

impl ScriptWriter {
    fn new(run: ScriptRunId, stream: ScriptStream, events: Arc<dyn ScriptEventSink>) -> Self {
        Self {
            run,
            stream,
            events,
            buffer: Mutex::new(String::new()),
        }
    }

    fn emit_line(&self, line: &str) {
        let line = line.trim_end_matches('\r').to_string();
        self.events.output(self.run, self.stream, line);
    }

    fn flush_buffer(&self) {
        let mut buffer = self
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !buffer.is_empty() {
            let line = std::mem::take(&mut *buffer);
            drop(buffer);
            self.emit_line(&line);
        }
    }
}

#[pymethods]
impl ScriptWriter {
    fn write(&self, text: &str) -> usize {
        let mut buffer = self
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        buffer.push_str(text);
        while let Some(index) = buffer.find('\n') {
            let mut line: String = buffer.drain(..=index).collect();
            line.pop();
            drop(buffer);
            self.emit_line(&line);
            buffer = self
                .buffer
                .lock()
                .unwrap_or_else(|error| error.into_inner());
        }
        text.chars().count()
    }

    fn flush(&self) {
        self.flush_buffer();
    }

    fn isatty(&self) -> bool {
        false
    }

    fn writable(&self) -> bool {
        true
    }

    fn readable(&self) -> bool {
        false
    }

    fn encoding(&self) -> &'static str {
        "utf-8"
    }

    fn errors(&self) -> &'static str {
        "strict"
    }
}

// ---------------------------------------------------------------- execution ---

struct RunOutcome {
    success: bool,
    code: Option<i32>,
}

impl RunOutcome {
    fn success() -> Self {
        Self {
            success: true,
            code: Some(0),
        }
    }
    fn failure() -> Self {
        Self {
            success: false,
            code: Some(1),
        }
    }
}

fn populate_module(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyWindow>()?;
    module.add_class::<PyCutAxis>()?;
    module.add_class::<PyDisplayKind>()?;
    module.add_class::<PyJobStatus>()?;
    module.add_class::<PyApp>()?;
    module.add_class::<PySignal>()?;
    module.add_class::<PySpectrogramHandle>()?;
    module.add_class::<PyOrderCutHandle>()?;
    module.add_class::<PyLineCutHandle>()?;
    module.add_class::<PyDisplay>()?;
    module.add_class::<PyAppInfo>()?;
    module.add_class::<PyTableInfo>()?;
    module.add_class::<PySpectrogramInfo>()?;
    module.add_class::<PyOrderCutInfo>()?;
    module.add_class::<PyLineCutInfo>()?;
    module.add_class::<ScriptWriter>()?;
    module.add(
        "DataDetachedError",
        module.py().get_type::<DataDetachedError>(),
    )?;
    module.add(
        "DataBorrowedError",
        module.py().get_type::<DataBorrowedError>(),
    )?;
    module.add("CancelledError", module.py().get_type::<CancelledError>())?;
    module.add("JobFailedError", module.py().get_type::<JobFailedError>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_function(wrap_pyfunction!(module_app, module)?)?;
    Ok(())
}

#[pyfunction]
#[pyo3(name = "app")]
fn module_app() -> PyResult<PyApp> {
    Ok(PyApp {
        context: current()?,
    })
}

/// The Rust `#[pymodule]` entry point. Embedded runs also create a fresh
/// module object and register it in `sys.modules`, so scripts import it by name.
#[pymodule]
pub fn fft_audio(module: &Bound<'_, PyModule>) -> PyResult<()> {
    populate_module(module)
}

fn load_source(source: &ScriptSource) -> PyResult<(String, String)> {
    match source {
        ScriptSource::Inline { name, source } => Ok((source.clone(), name.clone())),
        ScriptSource::File(path) => {
            let text = std::fs::read_to_string(path).map_err(|error| {
                PyOSError::new_err(format!("failed to read script {}: {error}", path.display()))
            })?;
            Ok((text, path.display().to_string()))
        }
    }
}

fn execute_run(
    run: ScriptRunId,
    source: ScriptSource,
    lease: Arc<dyn AppLease>,
    events: Arc<dyn ScriptEventSink>,
    cancel: Arc<AtomicBool>,
) -> RunOutcome {
    let (code, filename) = match load_source(&source) {
        Ok(loaded) => loaded,
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    let code = match CString::new(code) {
        Ok(code) => code,
        Err(_) => {
            events.output(
                run,
                ScriptStream::Stderr,
                "script source contains a NUL byte".to_string(),
            );
            return RunOutcome::failure();
        }
    };
    let filename = match CString::new(filename.clone()) {
        Ok(filename) => filename,
        Err(_) => {
            events.output(
                run,
                ScriptStream::Stderr,
                "script filename contains a NUL byte".to_string(),
            );
            return RunOutcome::failure();
        }
    };
    let context = RunContext::new(run, lease, Arc::clone(&events), Arc::clone(&cancel));

    // The interpreter is initialized once per process (auto-initialize).
    Python::initialize();
    let runner_context = context.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Python::attach(|py| run_in_python(py, &code, &filename, runner_context))
    }));
    // Release the lease/runtime even if CPython or the runner panicked. The
    // normal path also releases before returning so retained Python handles
    // cannot observe the app after the exit event.
    context.release();
    match outcome {
        Ok(outcome) => outcome,
        Err(_) => {
            events.output(
                run,
                ScriptStream::Stderr,
                "fft_audio: Python runner panicked".to_string(),
            );
            RunOutcome::failure()
        }
    }
}

fn run_in_python(
    py: Python<'_>,
    code: &CString,
    filename: &CString,
    context: RunContext,
) -> RunOutcome {
    let events = context.events();
    let run = context.run();
    let stdout = match Py::new(
        py,
        ScriptWriter::new(run, ScriptStream::Stdout, Arc::clone(&events)),
    ) {
        Ok(writer) => writer,
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    let stderr = match Py::new(
        py,
        ScriptWriter::new(run, ScriptStream::Stderr, Arc::clone(&events)),
    ) {
        Ok(writer) => writer,
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    let sys = match py.import("sys") {
        Ok(sys) => sys,
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    let old_stdout = match sys.getattr("stdout") {
        Ok(value) => value.unbind(),
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    let old_stderr = match sys.getattr("stderr") {
        Ok(value) => value.unbind(),
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };

    let module = match PyModule::new(py, "fft_audio")
        .and_then(|module| populate_module(&module).map(|_| module))
    {
        Ok(module) => module,
        Err(error) => {
            events.output(run, ScriptStream::Stderr, error.to_string());
            return RunOutcome::failure();
        }
    };
    if let Err(error) = (|| -> PyResult<()> {
        let modules = sys.getattr("modules")?.cast_into::<PyDict>()?;
        modules.set_item("fft_audio", &module)?;
        sys.setattr("stdout", &stdout)?;
        sys.setattr("stderr", &stderr)?;
        Ok(())
    })() {
        events.output(run, ScriptStream::Stderr, error.to_string());
        restore_streams(py, &sys, &old_stdout, &old_stderr);
        return RunOutcome::failure();
    }

    set_current(Some(context.clone()));
    let _current_guard = CurrentGuard;
    // Compile with the caller's filename so tracebacks point at the script, and
    // execute in a fresh `__main__` namespace whose `__file__` is already set.
    let execution = (|| -> PyResult<()> {
        let globals = PyDict::new(py);
        globals.set_item("__name__", "__main__")?;
        globals.set_item("__file__", filename.to_string_lossy().as_ref())?;
        globals.set_item("__builtins__", py.import("builtins")?)?;
        let compiled = PyCode::compile(py, code, filename, PyCodeInput::File)?;
        compiled.run(Some(&globals), None)?;
        Ok(())
    })();

    let mut outcome = match execution {
        Ok(()) => RunOutcome::success(),
        Err(_error) if context.is_cancelled() => {
            events.output(
                run,
                ScriptStream::Stderr,
                "fft_audio: script cancelled".to_string(),
            );
            RunOutcome::failure()
        }
        // `PyErr::print` delegates `SystemExit` to CPython's process-exit
        // handler, which would terminate the host. Extract the requested code
        // and report a normal exit instead.
        Err(error) if error.is_instance_of::<PySystemExit>(py) => system_exit_outcome(py, &error),
        Err(error) => {
            // Reinstall the capturing writer: the script may have reassigned
            // `sys.stderr`, but engine tracebacks still belong on our sink.
            let _ = sys.setattr("stderr", &stderr);
            error.print(py);
            RunOutcome::failure()
        }
    };
    if outcome.success && context.is_cancelled() {
        events.output(
            run,
            ScriptStream::Stderr,
            "fft_audio: script cancelled".to_string(),
        );
        outcome = RunOutcome::failure();
    }

    // Clear the current run and release its lease before the exit event: no
    // Python object, global, or Arc may keep the app reachable after the run.
    // Flush the capturing writers after restoring the real streams.
    set_current(None);
    context.release();
    restore_streams(py, &sys, &old_stdout, &old_stderr);
    flush_writer(py, &stdout);
    flush_writer(py, &stderr);

    outcome
}

/// Map `SystemExit` to a normal script outcome without invoking CPython's
/// process-exit handler. `None` means success; an integer is the requested
/// status (zero succeeds); any other payload is a failure with status 1.
fn system_exit_outcome(py: Python<'_>, error: &PyErr) -> RunOutcome {
    let Ok(code) = error.value(py).getattr("code") else {
        return RunOutcome::failure();
    };
    if code.is_none() {
        return RunOutcome::success();
    }
    match code.extract::<i32>() {
        Ok(0) => RunOutcome::success(),
        Ok(code) => RunOutcome {
            success: false,
            code: Some(code),
        },
        Err(_) => RunOutcome::failure(),
    }
}

fn restore_streams(
    py: Python<'_>,
    sys: &Bound<'_, PyModule>,
    old_stdout: &Py<PyAny>,
    old_stderr: &Py<PyAny>,
) {
    let _ = sys.setattr("stdout", old_stdout.bind(py));
    let _ = sys.setattr("stderr", old_stderr.bind(py));
}

fn flush_writer(py: Python<'_>, writer: &Py<ScriptWriter>) {
    if let Ok(writer) = writer.bind(py).try_borrow() {
        writer.flush_buffer();
    }
}

// ------------------------------------------------------------------- script ---

struct ActiveRun {
    run: ScriptRunId,
    cancel: Arc<AtomicBool>,
}

/// Process-global interpreter occupancy.
///
/// There is one CPython interpreter per process, so the guard must not be
/// per-`PythonScript`: two engines/app hosts sharing the process would
/// otherwise run concurrently and clobber `sys.stdout` and the current app.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);
static ACTIVE_RUN: OnceLock<Mutex<Option<ActiveRun>>> = OnceLock::new();

fn active_slot() -> &'static Mutex<Option<ActiveRun>> {
    ACTIVE_RUN.get_or_init(|| Mutex::new(None))
}

fn finish_run(run: ScriptRunId) {
    let mut active = active_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if active.as_ref().is_some_and(|active| active.run == run) {
        *active = None;
    }
}

/// Embedded Python engine. One interpreter per process; one active run.
///
/// Every [`FftScript::run`] call moves its granted lease handle into the run.
/// The root may grant clones of one adapter-lifetime lease; the adapter never
/// owns an app, constructs a runtime, or mints leases.
#[derive(Default)]
pub struct PythonScript;

impl PythonScript {
    pub fn new() -> Self {
        Self
    }

    /// Whether a run is currently executing.
    pub fn is_running(&self) -> bool {
        active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    /// The active run id, if any.
    pub fn active_run(&self) -> Option<ScriptRunId> {
        active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|active| active.run)
    }
}

impl FftScript for PythonScript {
    fn run(
        &self,
        lease: Arc<dyn AppLease>,
        source: ScriptSource,
        events: Arc<dyn ScriptEventSink>,
    ) -> std::result::Result<ScriptRunId, ScriptError> {
        let run = ScriptRunId(NEXT_RUN.fetch_add(1, Ordering::SeqCst));
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = active_slot()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if active.is_some() {
                return Err(ScriptError::busy(
                    "the embedded Python interpreter is already running a script",
                ));
            }
            *active = Some(ActiveRun {
                run,
                cancel: Arc::clone(&cancel),
            });
        }

        let thread_cancel = Arc::clone(&cancel);
        let thread_events = Arc::clone(&events);
        std::thread::Builder::new()
            .name("fft-python-script".to_string())
            .spawn(move || {
                // `execute_run` owns and drops the granted lease on every exit
                // path; the current-run context is cleared before it returns.
                let outcome = execute_run(
                    run,
                    source,
                    lease,
                    Arc::clone(&thread_events),
                    thread_cancel,
                );
                finish_run(run);
                thread_events.exited(run, outcome.success, outcome.code);
            })
            .map_err(|error| {
                finish_run(run);
                ScriptError::internal(format!("failed to start Python script thread: {error}"))
            })?;
        Ok(run)
    }

    fn cancel(&self, run: ScriptRunId) {
        let active = active_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(active) = active.as_ref() {
            if active.run == run {
                active.cancel.store(true, Ordering::SeqCst);
            }
        }
    }
}
