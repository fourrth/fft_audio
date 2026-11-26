//! In-memory display registry with parent/child lifecycle and subscriptions.
//!
//! The registry owns the authoritative display records. Runtime data handles
//! are recorded alongside each display, but the registry never interprets DSP
//! payloads. Lock discipline: records/sinks are locked only to mutate or clone
//! state; [`DisplayRegistry::publish`] always emits after that lock is dropped.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use fft_core::session::{JsonSessionDisplay, SessionDocument};
use fft_runtime::{AnalysisId, CutId, DisplayId, SignalId};
use serde_json::{json, Value};

use crate::display::{
    CutDisplayOptions, CutDisplaySettings, DisplayIdAllocator, DisplayInfo, DisplayKind,
    DisplayOwner, DisplayRecord, PlotDisplayOptions, SpectrogramDisplayOptions,
};
use crate::error::AppError;
use crate::events::{AppEvent, AppEventSink, Subscription};

/// Result of closing one display subtree.
///
/// `children` is sorted and contains all direct and recursive descendants.
/// `removed` contains the root record first, then descendants in `children`
/// order. An unknown/already-closed id yields an empty `removed` and publishes
/// no event.
#[derive(Clone, Debug, PartialEq)]
pub struct AppClose {
    pub id: DisplayId,
    pub children: Vec<DisplayId>,
    pub removed: Vec<DisplayRecord>,
}

impl AppClose {
    /// True when this close was a no-op.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty()
    }
}

struct RegistryState {
    records: BTreeMap<DisplayId, DisplayRecord>,
    sinks: HashMap<Subscription, Arc<dyn AppEventSink>>,
    next_subscription: u64,
}

/// Authoritative app-level display table plus event subscriptions.
///
/// Creation is restricted to the typed `insert_*` methods; there is no generic
/// `open(DisplayInfo)` path, so backend ids always allocate monotonically.
pub struct DisplayRegistry {
    state: Mutex<RegistryState>,
    allocator: DisplayIdAllocator,
}

impl DisplayRegistry {
    /// Create an empty registry. Persisted display ids only seed the allocator;
    /// restored records are adopted by the frontend/session layer.
    pub fn new(persisted: Option<&SessionDocument>) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                records: BTreeMap::new(),
                sinks: HashMap::new(),
                next_subscription: 1,
            }),
            allocator: DisplayIdAllocator::new(persisted),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    // ------------------------------------------------------------ transports ---

    /// Subscribe an event sink. The sink is called without any registry lock.
    pub fn subscribe(&self, sink: Arc<dyn AppEventSink>) -> Subscription {
        let mut state = self.lock();
        let subscription = Subscription::new(state.next_subscription);
        state.next_subscription += 1;
        state.sinks.insert(subscription, sink);
        subscription
    }

    /// Remove a subscription. Returns whether it was present.
    pub fn unsubscribe(&self, subscription: Subscription) -> bool {
        self.lock().sinks.remove(&subscription).is_some()
    }

    /// Number of live subscriptions (including adapter-internal capturers).
    pub fn subscription_count(&self) -> usize {
        self.lock().sinks.len()
    }

    /// Emit one event to every sink currently subscribed.
    ///
    /// Sinks are cloned under a short lock, then called after it is dropped;
    /// sink code may safely call back into the registry.
    pub fn publish(&self, event: AppEvent) {
        let sinks: Vec<_> = self.lock().sinks.values().cloned().collect();
        for sink in sinks {
            sink.emit(event.clone());
        }
    }

    /// Drop all sinks. Used by app shutdown; later events are not delivered.
    pub fn clear_sinks(&self) {
        self.lock().sinks.clear();
    }

    // --------------------------------------------------------------- queries ---

    /// All live display metadata, sorted by id.
    pub fn list(&self) -> Vec<DisplayInfo> {
        self.lock()
            .records
            .values()
            .map(|record| record.info.clone())
            .collect()
    }

    /// Metadata for one display, if still open.
    pub fn get(&self, id: DisplayId) -> Option<DisplayInfo> {
        self.lock()
            .records
            .get(&id)
            .map(|record| record.info.clone())
    }

    /// Full record including typed handles and owner.
    pub fn record(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.lock().records.get(&id).cloned()
    }

    /// All full records, sorted by id.
    pub fn records(&self) -> Vec<DisplayRecord> {
        self.lock().records.values().cloned().collect()
    }

    /// Non-ephemeral displays converted to the persisted `session.json` shape.
    ///
    /// Ephemeral (in-memory/script) displays are excluded from session and undo.
    pub fn session_displays(&self) -> Vec<JsonSessionDisplay> {
        self.lock()
            .records
            .values()
            .filter(|record| !record.info.ephemeral)
            .map(|record| record.info.to_json_session_display())
            .collect()
    }

    // -------------------------------------------------------------- creation ---

    /// Insert a spectrogram display. A display may be created while its
    /// analysis is still pending. Backend/script displays always allocate a
    /// monotonic id in the reserved [`crate::SCRIPTED_DISPLAY_ID_BASE`] range.
    pub fn insert_spectrogram(
        &self,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        let (id, info) = {
            let mut state = self.lock();
            let id = self.allocator.allocate();
            let info = spectrogram_info(id, analysis, signal, options, owner, ephemeral);
            state.records.insert(
                id,
                DisplayRecord {
                    info: info.clone(),
                    analysis: Some(analysis),
                    signal,
                    cut: None,
                    owner,
                },
            );
            (id, info)
        };
        self.publish(AppEvent::DisplayOpened { display: info });
        Ok(id)
    }

    /// Insert a spectrogram display under a caller-owned id.
    ///
    /// Frontend-owned displays use ids below
    /// [`crate::SCRIPTED_DISPLAY_ID_BASE`]; the reserved range stays
    /// backend-only. Explicit ids are recorded with owner [`DisplayOwner::User`]
    /// by the concrete app host, and never allocate from the backend allocator.
    /// Reusing a live id is rejected before any record is created.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_spectrogram_with_id(
        &self,
        id: DisplayId,
        analysis: AnalysisId,
        signal: Option<SignalId>,
        options: &SpectrogramDisplayOptions,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        if id.get() == 0 {
            return Err(AppError::invalid_input("display id 0 is reserved"));
        }
        if id.get() >= crate::SCRIPTED_DISPLAY_ID_BASE {
            return Err(AppError::invalid_input(format!(
                "user display id {} must stay below {}",
                id.get(),
                crate::SCRIPTED_DISPLAY_ID_BASE
            )));
        }
        let info = {
            let mut state = self.lock();
            if state.records.contains_key(&id) {
                return Err(AppError::invalid_input(format!(
                    "display {} is already open",
                    id.get()
                )));
            }
            let info = spectrogram_info(id, analysis, signal, options, owner, ephemeral);
            state.records.insert(
                id,
                DisplayRecord {
                    info: info.clone(),
                    analysis: Some(analysis),
                    signal,
                    cut: None,
                    owner,
                },
            );
            info
        };
        self.publish(AppEvent::DisplayOpened { display: info });
        Ok(id)
    }

    /// Insert a standalone generic Plot display.
    ///
    /// A plot has no runtime handle: its state keeps the table path and layout,
    /// and the transport maps the file onto series on request.
    pub fn insert_plot(
        &self,
        options: &PlotDisplayOptions,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        if options.path.trim().is_empty() {
            return Err(AppError::invalid_input("a plot display needs a file path"));
        }
        let (id, info) = {
            let mut state = self.lock();
            let id = self.allocator.allocate();
            let name = options.name.clone().unwrap_or_else(|| {
                Path::new(&options.path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Plot")
                    .to_string()
            });
            let info = DisplayInfo {
                id,
                parent: None,
                kind: DisplayKind::Plot,
                name,
                state: json!({
                    "path": options.path,
                    "plotLayout": options.layout,
                    "sourceKind": owner.as_str(),
                    "scripted": owner == DisplayOwner::Backend,
                }),
                visible: options.visible,
                ephemeral,
            };
            state.records.insert(
                id,
                DisplayRecord {
                    info: info.clone(),
                    analysis: None,
                    signal: None,
                    cut: None,
                    owner,
                },
            );
            (id, info)
        };
        self.publish(AppEvent::DisplayOpened { display: info });
        Ok(id)
    }

    /// Insert an Order Cuts child under a spectrogram display.
    pub fn insert_order_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        analysis: AnalysisId,
        options: &CutDisplayOptions,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        let settings = match &options.settings {
            CutDisplaySettings::Order(settings) => settings,
            CutDisplaySettings::Line(_) => {
                return Err(AppError::invalid_input(
                    "show_order_cut requires order-cut display settings",
                ))
            }
        };
        self.insert_cut(
            parent,
            cut,
            analysis,
            options,
            settings,
            DisplayKind::OrderCuts,
            "Order Cuts",
            owner,
            ephemeral,
        )
    }

    /// Insert a Line Cuts child under a spectrogram display.
    pub fn insert_line_cut(
        &self,
        parent: DisplayId,
        cut: CutId,
        analysis: AnalysisId,
        options: &CutDisplayOptions,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        let settings = match &options.settings {
            CutDisplaySettings::Line(settings) => settings,
            CutDisplaySettings::Order(_) => {
                return Err(AppError::invalid_input(
                    "show_line_cut requires line-cut display settings",
                ))
            }
        };
        self.insert_cut(
            parent,
            cut,
            analysis,
            options,
            settings,
            DisplayKind::LineCuts,
            "Line Cuts",
            owner,
            ephemeral,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_cut<S: serde::Serialize>(
        &self,
        parent: DisplayId,
        cut: CutId,
        analysis: AnalysisId,
        options: &CutDisplayOptions,
        settings: &S,
        kind: DisplayKind,
        default_name: &str,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> Result<DisplayId, AppError> {
        let (id, info) = {
            let mut state = self.lock();
            let Some(parent_record) = state.records.get(&parent) else {
                return Err(AppError::not_found(format!(
                    "parent display {} not found",
                    parent.get()
                )));
            };
            if parent_record.info.kind != DisplayKind::Spectrogram {
                return Err(AppError::invalid_input(
                    "cut displays need a spectrogram parent display",
                ));
            }

            let id = self.allocator.allocate();
            let mut state_json = json!({
                "analysisId": analysis.get(),
                "cutId": cut.get(),
                "settings": settings,
                "sourceKind": owner.as_str(),
                "scripted": owner == DisplayOwner::Backend,
            });
            if let Some(rpm) = &options.rpm {
                state_json["rpm"] = serde_json::to_value(rpm)
                    .map_err(|error| AppError::internal(error.to_string()))?;
            }
            let info = DisplayInfo {
                id,
                parent: Some(parent),
                kind,
                name: options
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("{default_name} {}", cut.get())),
                state: state_json,
                visible: options.visible,
                ephemeral,
            };
            state.records.insert(
                id,
                DisplayRecord {
                    info: info.clone(),
                    analysis: Some(analysis),
                    // Signal ownership stays with the root spectrogram; a
                    // child close must never release the parent's signal.
                    signal: None,
                    cut: Some(cut),
                    owner,
                },
            );
            (id, info)
        };
        self.publish(AppEvent::DisplayOpened { display: info });
        Ok(id)
    }

    // --------------------------------------------------------------- updates ---

    /// Replace a display's app state and visibility, emitting
    /// `display.updated`.
    pub fn update(
        &self,
        id: DisplayId,
        state: Value,
        visible: bool,
    ) -> Result<DisplayInfo, AppError> {
        let info = {
            let mut guard = self.lock();
            let Some(record) = guard.records.get_mut(&id) else {
                return Err(AppError::not_found(format!(
                    "display {} not found",
                    id.get()
                )));
            };
            record.info.state = state;
            record.info.visible = visible;
            record.info.clone()
        };
        self.publish(AppEvent::DisplayUpdated {
            display: info.clone(),
        });
        Ok(info)
    }

    // ----------------------------------------------------------------- close ---

    /// Remove one display record without emitting an event.
    ///
    /// Used when a failed open must be abandoned without a `display.closed`
    /// broadcast. Descendants are not touched; callers that own a subtree use
    /// [`DisplayRegistry::close`].
    pub fn remove_silent(&self, id: DisplayId) -> Option<DisplayRecord> {
        self.lock().records.remove(&id)
    }

    /// Remove a display subtree and emit exactly one `display.closed` event.
    ///
    /// Unknown/already-closed ids are a successful no-op with no event.
    pub fn close(&self, id: DisplayId) -> AppClose {
        let (children, removed) = {
            let mut state = self.lock();
            if !state.records.contains_key(&id) {
                return AppClose {
                    id,
                    children: Vec::new(),
                    removed: Vec::new(),
                };
            }
            let children = descendants(&state.records, id);
            let mut removed = Vec::new();
            if let Some(record) = state.records.remove(&id) {
                removed.push(record);
            }
            for child in &children {
                if let Some(record) = state.records.remove(child) {
                    removed.push(record);
                }
            }
            (children, removed)
        };

        self.publish(AppEvent::DisplayClosed {
            id,
            children: children.clone(),
        });
        AppClose {
            id,
            children,
            removed,
        }
    }
}

impl Default for DisplayRegistry {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Build the wire metadata and app state for a spectrogram display.
fn spectrogram_info(
    id: DisplayId,
    analysis: AnalysisId,
    signal: Option<SignalId>,
    options: &SpectrogramDisplayOptions,
    owner: DisplayOwner,
    ephemeral: bool,
) -> DisplayInfo {
    let mut state_json = json!({
        "analysisId": analysis.get(),
        "settings": &options.settings,
        "sourceKind": owner.as_str(),
        "scripted": owner == DisplayOwner::Backend,
    });
    if let Some(signal) = signal {
        state_json["signalId"] = json!(signal.get());
    }
    if ephemeral {
        state_json["ephemeral"] = json!(true);
    }
    DisplayInfo {
        id,
        parent: None,
        kind: DisplayKind::Spectrogram,
        name: options
            .name
            .clone()
            .unwrap_or_else(|| "Spectrogram".to_string()),
        state: state_json,
        visible: options.visible,
        ephemeral,
    }
}

/// All direct and recursive descendants of `root`, sorted by id.
fn descendants(records: &BTreeMap<DisplayId, DisplayRecord>, root: DisplayId) -> Vec<DisplayId> {
    let mut collected = Vec::new();
    let mut stack = vec![root];
    while let Some(parent) = stack.pop() {
        for (&id, record) in records {
            if record.info.parent == Some(parent) && !collected.contains(&id) {
                collected.push(id);
                stack.push(id);
            }
        }
    }
    collected.sort();
    collected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AppEvent;
    use fft_core::settings::{LineCutSettings, OrderCutSettings};
    use fft_core::table::PlotLayout;
    use std::sync::Mutex as StdMutex;

    fn capture(registry: &DisplayRegistry) -> Arc<StdMutex<Vec<AppEvent>>> {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let target = Arc::clone(&events);
        registry.subscribe(Arc::new(move |event| {
            target.lock().unwrap().push(event);
        }));
        events
    }

    fn spectrogram(
        registry: &DisplayRegistry,
        analysis: u64,
        owner: DisplayOwner,
        ephemeral: bool,
    ) -> DisplayId {
        registry
            .insert_spectrogram(
                AnalysisId(analysis),
                Some(SignalId(analysis + 100)),
                &SpectrogramDisplayOptions::default(),
                owner,
                ephemeral,
            )
            .unwrap()
    }

    #[test]
    fn ephemeral_owner_and_state_round_trip_and_filter_from_session() {
        let registry = DisplayRegistry::new(None);
        let ephemeral = registry
            .insert_spectrogram(
                AnalysisId(3),
                Some(SignalId(9)),
                &SpectrogramDisplayOptions::default(),
                DisplayOwner::Backend,
                true,
            )
            .unwrap();

        let record = registry.record(ephemeral).unwrap();
        assert_eq!(record.owner, DisplayOwner::Backend);
        assert_eq!(record.analysis, Some(AnalysisId(3)));
        assert_eq!(record.signal, Some(SignalId(9)));
        assert!(record.info.ephemeral);
        assert_eq!(record.info.state["analysisId"], 3);
        assert_eq!(record.info.state["signalId"], 9);
        assert_eq!(record.info.state["ephemeral"], true);
        assert!(record.info.state.get("settings").is_some());

        // Owner is internal; ownership info does not leak into the wire type.
        let wire = serde_json::to_value(&record.info).unwrap();
        assert_eq!(wire["ephemeral"], true);
        assert!(wire.get("owner").is_none());

        assert!(
            registry.session_displays().is_empty(),
            "ephemeral displays are excluded from session/undo"
        );

        let persistent = spectrogram(&registry, 4, DisplayOwner::User, false);
        let session = registry.session_displays();
        assert_eq!(session.len(), 1);
        assert_eq!(session[0].id, persistent.get());
        assert_eq!(session[0].kind, "spectrogram");
        assert!(session[0].parent.is_none());
        let info = registry.get(persistent).unwrap();
        assert!(!info.ephemeral);
        assert_eq!(info.to_json_session_display(), session[0]);
    }

    #[test]
    fn order_display_state_carries_optional_rpm_points() {
        let registry = DisplayRegistry::new(None);
        let parent = spectrogram(&registry, 1, DisplayOwner::User, false);
        let options = CutDisplayOptions::order(OrderCutSettings::default()).with_rpm(
            fft_runtime::RpmProfileSpec::Points(vec![
                fft_runtime::RpmPoint {
                    time_s: 0.0,
                    rpm: 6_000.0,
                },
                fft_runtime::RpmPoint {
                    time_s: 0.5,
                    rpm: 7_200.0,
                },
            ]),
        );
        let order = registry
            .insert_order_cut(
                parent,
                CutId(11),
                AnalysisId(1),
                &options,
                DisplayOwner::Backend,
                true,
            )
            .unwrap();
        let state = &registry.get(order).unwrap().state;
        assert_eq!(state["rpm"]["kind"], "points");
        assert_eq!(state["rpm"]["value"][1]["timeS"], 0.5);
        assert_eq!(state["rpm"]["value"][1]["rpm"], 7_200.0);

        let plain = registry
            .insert_order_cut(
                parent,
                CutId(12),
                AnalysisId(1),
                &CutDisplayOptions::order(OrderCutSettings::default()),
                DisplayOwner::Backend,
                true,
            )
            .unwrap();
        assert!(registry.get(plain).unwrap().state.get("rpm").is_none());
    }

    #[test]
    fn plot_display_is_standalone_and_keeps_path_and_layout() {
        let registry = DisplayRegistry::new(None);
        let events = capture(&registry);
        let options = PlotDisplayOptions::new("/data/series.csv")
            .with_layout(PlotLayout::Pairs)
            .with_name("CSV plot");
        let id = registry
            .insert_plot(&options, DisplayOwner::User, false)
            .unwrap();
        let info = registry.get(id).unwrap();
        assert_eq!(info.kind, DisplayKind::Plot);
        assert_eq!(info.name, "CSV plot");
        assert!(info.parent.is_none());
        assert_eq!(info.state["path"], "/data/series.csv");
        assert_eq!(info.state["plotLayout"], "pairs");
        assert_eq!(info.state["sourceKind"], "user");
        let record = registry.record(id).unwrap();
        assert!(record.analysis.is_none());
        assert!(record.signal.is_none());
        assert!(record.cut.is_none());
        assert_eq!(info.to_json_session_display().kind, "plot");
        assert!(matches!(
            events.lock().unwrap().first(),
            Some(AppEvent::DisplayOpened { .. })
        ));
    }

    #[test]
    fn plot_display_names_itself_after_the_file_and_is_ephemeral() {
        let registry = DisplayRegistry::new(None);
        let id = registry
            .insert_plot(
                &PlotDisplayOptions::new("/x/run_01.csv"),
                DisplayOwner::Backend,
                true,
            )
            .unwrap();
        let info = registry.get(id).unwrap();
        assert_eq!(info.name, "run_01.csv");
        assert!(info.ephemeral);
        assert_eq!(info.state["scripted"], true);
        assert!(registry.session_displays().is_empty());
    }

    #[test]
    fn plot_display_requires_a_path() {
        let registry = DisplayRegistry::new(None);
        let error = registry
            .insert_plot(&PlotDisplayOptions::default(), DisplayOwner::User, false)
            .unwrap_err();
        assert_eq!(error.code(), "invalid_input");
    }

    #[test]
    fn close_cascades_with_sorted_children_and_one_event() {
        let registry = DisplayRegistry::new(None);
        let events = capture(&registry);
        let parent = spectrogram(&registry, 1, DisplayOwner::User, false);
        let order = registry
            .insert_order_cut(
                parent,
                CutId(11),
                AnalysisId(1),
                &CutDisplayOptions::order(OrderCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap();
        let line = registry
            .insert_line_cut(
                parent,
                CutId(12),
                AnalysisId(1),
                &CutDisplayOptions::line(LineCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap();

        // Cut state carries enough for attach.
        let state = &registry.get(line).unwrap().state;
        assert_eq!(state["analysisId"], 1);
        assert_eq!(state["cutId"], 12);
        assert!(state.get("settings").is_some());

        let close = registry.close(parent);
        assert_eq!(close.id, parent);
        assert_eq!(close.children, vec![order, line]);
        assert_eq!(close.removed.len(), 3);
        assert!(registry.list().is_empty());

        let captured = events.lock().unwrap().clone();
        assert_eq!(captured.len(), 4, "three opens then one close");
        assert!(matches!(captured[0], AppEvent::DisplayOpened { .. }));
        assert!(matches!(captured[1], AppEvent::DisplayOpened { .. }));
        assert!(matches!(captured[2], AppEvent::DisplayOpened { .. }));
        assert!(matches!(
            &captured[3],
            AppEvent::DisplayClosed { id, children }
                if *id == parent && *children == vec![order, line]
        ));

        let again = registry.close(parent);
        assert!(again.is_empty());
        assert_eq!(events.lock().unwrap().len(), 4, "no duplicate close event");
        assert!(registry.close(DisplayId(42)).is_empty());
        assert_eq!(events.lock().unwrap().len(), 4, "unknown close is a no-op");
    }

    #[test]
    fn update_changes_state_and_visibility_and_emits_payload() {
        let registry = DisplayRegistry::new(None);
        let events = capture(&registry);
        let id = spectrogram(&registry, 1, DisplayOwner::User, false);
        let updated = registry
            .update(id, json!({ "analysisId": 1, "zoom": 2.5 }), false)
            .unwrap();
        assert!(!updated.visible);
        assert_eq!(updated.state["zoom"], 2.5);
        assert_eq!(registry.get(id).unwrap(), updated);

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(matches!(
            &captured[1],
            AppEvent::DisplayUpdated { display }
                if display.id == id && !display.visible && display.state["zoom"] == 2.5
        ));
        drop(captured);

        let missing = registry.update(DisplayId(42), json!({}), true).unwrap_err();
        assert_eq!(missing.code(), "not_found");
        assert_eq!(
            events.lock().unwrap().len(),
            2,
            "failed update emits nothing"
        );
    }

    #[test]
    fn subscribe_and_unsubscribe_receive_events_once() {
        let registry = DisplayRegistry::new(None);
        let events = Arc::new(StdMutex::new(Vec::new()));
        let target = Arc::clone(&events);
        let subscription = registry.subscribe(Arc::new(move |event: AppEvent| {
            target.lock().unwrap().push(event);
        }));
        assert_eq!(registry.subscription_count(), 1);

        spectrogram(&registry, 1, DisplayOwner::User, false);
        assert_eq!(events.lock().unwrap().len(), 1);

        assert!(registry.unsubscribe(subscription));
        assert!(!registry.unsubscribe(subscription));
        assert_eq!(registry.subscription_count(), 0);

        spectrogram(&registry, 2, DisplayOwner::User, false);
        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "unsubscribed sink receives nothing further"
        );
    }

    #[test]
    fn publishing_never_holds_the_registry_lock() {
        let registry = Arc::new(DisplayRegistry::new(None));
        let target = Arc::clone(&registry);
        registry.subscribe(Arc::new(move |_event| {
            // Re-entering the registry would deadlock if publish held the lock.
            assert_eq!(target.list().len(), 1);
        }));
        spectrogram(&registry, 1, DisplayOwner::User, false);
    }

    #[test]
    fn backend_ids_are_reserved_monotonic_and_never_reused() {
        let registry = DisplayRegistry::new(None);
        let first = spectrogram(&registry, 1, DisplayOwner::Backend, false);
        assert_eq!(first, DisplayId(crate::SCRIPTED_DISPLAY_ID_BASE));
        registry.close(first);
        let second = spectrogram(&registry, 2, DisplayOwner::Backend, false);
        assert_eq!(
            second,
            DisplayId(crate::SCRIPTED_DISPLAY_ID_BASE + 1),
            "closed ids are not reused"
        );

        let session = SessionDocument {
            displays: vec![JsonSessionDisplay {
                id: second.get() + 10,
                parent: None,
                kind: "spectrogram".into(),
                name: "persisted".into(),
                state: json!({}),
            }],
            ..SessionDocument::default()
        };
        let seeded = DisplayRegistry::new(Some(&session));
        let allocated = spectrogram(&seeded, 3, DisplayOwner::Backend, false);
        assert_eq!(allocated.get(), second.get() + 11);
    }

    #[test]
    fn cut_parent_and_options_are_validated_before_allocation() {
        let registry = DisplayRegistry::new(None);
        let parent = spectrogram(&registry, 1, DisplayOwner::User, false);
        let missing = registry
            .insert_order_cut(
                DisplayId(42),
                CutId(1),
                AnalysisId(1),
                &CutDisplayOptions::order(OrderCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap_err();
        assert_eq!(missing.code(), "not_found");

        let wrong = registry
            .insert_order_cut(
                parent,
                CutId(1),
                AnalysisId(1),
                &CutDisplayOptions::line(LineCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap_err();
        assert_eq!(wrong.code(), "invalid_input");

        let order = registry
            .insert_order_cut(
                parent,
                CutId(1),
                AnalysisId(1),
                &CutDisplayOptions::order(OrderCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap();
        let nested = registry
            .insert_line_cut(
                order,
                CutId(2),
                AnalysisId(1),
                &CutDisplayOptions::line(LineCutSettings::default()),
                DisplayOwner::Backend,
                false,
            )
            .unwrap_err();
        assert_eq!(nested.code(), "invalid_input");

        // Failed validation never burned a display id.
        assert_eq!(
            order,
            DisplayId(crate::SCRIPTED_DISPLAY_ID_BASE + 1),
            "only the parent and one valid cut were allocated"
        );
    }

    #[test]
    fn explicit_user_ids_are_recorded_below_the_reserved_range() {
        let registry = DisplayRegistry::new(None);
        let events = capture(&registry);
        let id = registry
            .insert_spectrogram_with_id(
                DisplayId(42),
                AnalysisId(7),
                Some(SignalId(8)),
                &SpectrogramDisplayOptions::default(),
                DisplayOwner::User,
                false,
            )
            .unwrap();
        assert_eq!(id, DisplayId(42));

        let record = registry.record(id).unwrap();
        assert_eq!(record.owner, DisplayOwner::User);
        assert!(!record.info.ephemeral);
        assert_eq!(record.analysis, Some(AnalysisId(7)));
        assert_eq!(record.signal, Some(SignalId(8)));
        assert_eq!(record.info.state["analysisId"], 7);
        assert_eq!(record.info.state["signalId"], 8);
        assert_eq!(record.info.state["sourceKind"], "user");
        assert_eq!(record.info.state["scripted"], false);
        assert!(record.info.state.get("settings").is_some());

        // Backend allocation still starts in the reserved range.
        let backend = spectrogram(&registry, 9, DisplayOwner::Backend, false);
        assert_eq!(backend, DisplayId(crate::SCRIPTED_DISPLAY_ID_BASE));
        let backend_state = &registry.get(backend).unwrap().state;
        assert_eq!(backend_state["sourceKind"], "backend");
        assert_eq!(backend_state["scripted"], true);

        assert_eq!(
            events.lock().unwrap().len(),
            2,
            "both explicit and allocated displays open exactly once"
        );

        let duplicate = registry
            .insert_spectrogram_with_id(
                id,
                AnalysisId(10),
                None,
                &SpectrogramDisplayOptions::default(),
                DisplayOwner::User,
                false,
            )
            .unwrap_err();
        assert_eq!(duplicate.code(), "invalid_input");
        let reserved = registry
            .insert_spectrogram_with_id(
                DisplayId(crate::SCRIPTED_DISPLAY_ID_BASE),
                AnalysisId(10),
                None,
                &SpectrogramDisplayOptions::default(),
                DisplayOwner::User,
                false,
            )
            .unwrap_err();
        assert_eq!(reserved.code(), "invalid_input");
        let zero = registry
            .insert_spectrogram_with_id(
                DisplayId(0),
                AnalysisId(10),
                None,
                &SpectrogramDisplayOptions::default(),
                DisplayOwner::User,
                false,
            )
            .unwrap_err();
        assert_eq!(zero.code(), "invalid_input");
        assert_eq!(
            events.lock().unwrap().len(),
            2,
            "rejected explicit ids emit nothing"
        );
    }

    #[test]
    fn remove_silent_deletes_one_record_without_an_event() {
        let registry = DisplayRegistry::new(None);
        let id = spectrogram(&registry, 1, DisplayOwner::User, false);
        let events = capture(&registry);
        let removed = registry.remove_silent(id).expect("record");
        assert_eq!(removed.info.id, id);
        assert!(registry.get(id).is_none());
        assert!(registry.remove_silent(id).is_none());
        assert!(
            events.lock().unwrap().is_empty(),
            "silent removal emits nothing"
        );
    }
}
