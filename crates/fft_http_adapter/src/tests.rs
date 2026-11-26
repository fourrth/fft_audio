//! Integration-style tests for the HTTP/WS adapter.
//!
//! Tests use `fft_test` so the adapter only sees `Arc<dyn AppLease>` and the
//! `FftApp` contract. Servers in these tests bind ephemeral loopback ports so
//! no listener is reachable from other hosts; the adapter itself accepts any
//! configured address.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use fft_app::{AnalysisId, AppCapabilities, DisplayId, FftApp, SignalId, SCRIPTED_DISPLAY_ID_BASE};
use fft_core::orders::LineSpec;
use fft_core::settings::{AnalysisSettings, LineCutSettings, OrderCutSettings};
use fft_runtime::{EntityId, FftRuntime, LoadSignalRequest};
use fft_test::{TestApp, TestHost};

use crate::{HttpAdapter, HttpAdapterConfig, HttpAdapterError, HttpServerHandle};
use futures_util::StreamExt;
use reqwest::header::{
    ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS,
    ACCESS_CONTROL_REQUEST_METHOD, CONTENT_TYPE, ORIGIN,
};
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct TestServer {
    host: TestHost,
    temp: tempfile::TempDir,
    handle: Option<HttpServerHandle>,
    base: String,
    uploads_dir: PathBuf,
    dist: PathBuf,
}

impl TestServer {
    fn app(&self) -> &TestApp {
        self.host.app()
    }

    fn runtime(&self) -> &std::sync::Arc<fft_runtime::MockRuntime> {
        self.host.app().runtime_handle()
    }

    fn addr(&self) -> SocketAddr {
        self.handle.as_ref().unwrap().addr()
    }

    async fn stop(mut self) -> TestHost {
        if let Some(handle) = self.handle.take() {
            handle.stop().await.expect("server stop");
        }
        self.host
    }
}

async fn start(app: TestApp) -> TestServer {
    start_host(TestHost::with_app(app)).await
}

async fn start_host(host: TestHost) -> TestServer {
    let temp = tempfile::tempdir().expect("temp dir");
    let config = HttpAdapterConfig::new(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        temp.path().join("dist"),
        temp.path(),
    );
    std::fs::create_dir_all(&config.dist).expect("dist dir");
    let uploads_dir = config.uploads_dir.clone();
    let dist = config.dist.clone();
    let lease = host.lease();
    let lease_id = lease.lease_id();
    let adapter = HttpAdapter::new(config, lease).expect("adapter construction");
    assert_eq!(adapter.lease_id(), lease_id);
    let handle = adapter.serve().await.expect("server start");
    let base = format!("http://{}", handle.addr());
    TestServer {
        host,
        temp,
        handle: Some(handle),
        base,
        uploads_dir,
        dist,
    }
}

fn seed_analysis(server: &TestServer) -> (SignalId, AnalysisId) {
    let runtime = server.runtime();
    let signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .expect("load signal");
    let analysis = runtime
        .analyze(signal, &AnalysisSettings::default())
        .expect("analyze");
    (signal, analysis)
}

async fn response_json(response: reqwest::Response) -> (reqwest::StatusCode, Value) {
    let status = response.status();
    let body = response.text().await.expect("response body");
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&body).expect("JSON response")
    };
    (status, value)
}

async fn next_ws_event(socket: &mut Ws) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("WS event timeout")
            .expect("WS closed")
            .expect("WS message");
        match message {
            Message::Text(text) => return serde_json::from_str(text.as_str()).expect("WS JSON"),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(frame) => panic!("WS closed: {frame:?}"),
            Message::Binary(_) | Message::Frame(_) => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn app_info_and_capabilities_shape() {
    let app = TestApp::new().with_capabilities(AppCapabilities {
        displays: false,
        table_formats: Vec::new(),
    });
    let server = start(app).await;
    let client = reqwest::Client::new();
    let (status, info) = response_json(
        client
            .get(format!("{}/api/app/info", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(info["name"], "fft_audio-test");
    assert!(!info["version"].as_str().unwrap().is_empty());
    assert_eq!(info["mode"], "headless");
    assert_eq!(info["capabilities"]["displays"], false);
    assert!(info["capabilities"].get("scripting").unwrap().is_null());
    assert_eq!(info["capabilities"]["tableFormats"], json!([]));

    let (status, ping) = response_json(
        client
            .post(format!("{}/api/app/ping", server.base))
            .json(&json!({ "nonce": "round-trip" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(ping["nonce"], "round-trip");
    assert!(ping["at"].as_u64().unwrap() > 0);

    let host = server.stop().await;
    host.assert_all_leases_dropped();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_server_lease_survives_requests_errors_and_ws_disconnect() {
    let server = start(TestApp::new()).await;
    let host = &server.host;
    let created = host.created_lease_ids();
    assert_eq!(created.len(), 1, "the server receives exactly one lease");
    assert_eq!(host.active_leases(), 1);
    assert_eq!(
        server.app().user_subscription_count(),
        1,
        "the adapter subscribes to app events once for the server lifetime"
    );
    let client = reqwest::Client::new();

    for path in [
        "/api/app/info",
        "/api/displays",
        "/api/settings",
        "/api/missing",
        "/api/script/run",
    ] {
        let _ = client
            .get(format!("{}{path}", server.base))
            .send()
            .await
            .unwrap();
    }
    let _ = client
        .post(format!("{}/api/displays/close", server.base))
        .header(CONTENT_TYPE, "application/json")
        .body("not-json")
        .send()
        .await
        .unwrap();

    let socket = connect_async(format!("ws://{}/ws", server.addr()))
        .await
        .unwrap()
        .0;
    drop(socket);

    assert_eq!(
        host.created_lease_ids(),
        created,
        "request handling must not mint leases"
    );
    assert_eq!(host.active_leases(), 1);
    assert_eq!(
        server.app().user_subscription_count(),
        1,
        "requests must not create event subscriptions"
    );

    let host = server.stop().await;
    host.assert_all_leases_dropped();
    assert_eq!(host.created_lease_ids(), created);
}

#[tokio::test(flavor = "multi_thread")]
async fn non_loopback_bind_address_is_accepted() {
    let host = TestHost::new();
    let temp = tempfile::tempdir().unwrap();
    // `new` opens no listener, so checking acceptance here never exposes a
    // wildcard listener while asserting that the address is no longer rejected.
    let config = HttpAdapterConfig::new(
        "0.0.0.0:0".parse().unwrap(),
        temp.path().join("dist"),
        temp.path(),
    );
    let adapter = HttpAdapter::new(config, host.lease()).expect("any bind address is allowed");
    assert_eq!(host.active_leases(), 1);
    drop(adapter);
    host.assert_all_leases_dropped();
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_failure_releases_the_lease() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = occupied.local_addr().unwrap();
    let host = TestHost::new();
    let temp = tempfile::tempdir().unwrap();
    let config = HttpAdapterConfig::new(addr, temp.path().join("dist"), temp.path());
    let adapter = HttpAdapter::new(config, host.lease()).unwrap();
    let error = adapter.serve().await.err().expect("bind must fail");
    assert!(matches!(error, HttpAdapterError::Bind { .. }));
    host.assert_all_leases_dropped();
}

#[tokio::test(flavor = "multi_thread")]
async fn display_list_get_show_close_and_unknown_404() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();
    let (_, analysis) = seed_analysis(&server);

    let (status, shown) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": analysis.get(), "ephemeral": true }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let display_id = shown["id"].as_u64().expect("display id");
    assert!(
        display_id >= SCRIPTED_DISPLAY_ID_BASE,
        "backend displays use the reserved id range"
    );
    assert_eq!(shown["kind"], "spectrogram");
    assert_eq!(shown["state"]["analysisId"], analysis.get());
    assert_eq!(shown["state"]["sourceKind"], "backend");
    assert_eq!(shown["ephemeral"], true);

    let (status, list) = response_json(
        client
            .get(format!("{}/api/displays", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], display_id);

    let (status, fetched) = response_json(
        client
            .get(format!("{}/api/displays/{display_id}", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(fetched, shown);

    let (status, error) = response_json(
        client
            .get(format!("{}/api/displays/999999", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert!(error["error"].as_str().unwrap().contains("not found"));

    let (status, error) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": 999_999 }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert!(error["error"].as_str().unwrap().contains("not_found"));

    let status = client
        .post(format!("{}/api/displays/close", server.base))
        .json(&json!({ "id": display_id }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    let (_, list) = response_json(
        client
            .get(format!("{}/api/displays", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list, json!([]));

    // Unknown close is an idempotent 204 no-op.
    let status = client
        .post(format!("{}/api/displays/close", server.base))
        .json(&json!({ "id": display_id }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);

    let (status, _) = response_json(
        client
            .get(format!("{}/api/spectrogram/{display_id}", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spectrogram_open_get_image_cancel_and_cleanup() {
    let server = start_host(TestHost::held()).await;
    let client = reqwest::Client::new();
    let settings = AnalysisSettings::default();

    let status = client
        .post(format!("{}/api/spectrogram/open", server.base))
        .json(&json!({ "displayId": 0, "path": "clip.wav", "settings": settings }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);

    let mut invalid = settings.clone();
    invalid.frame = 1;
    let (status, error) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({ "displayId": 1, "path": "clip.wav", "settings": invalid }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("invalid analysis settings"));

    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({ "displayId": 1, "path": "clip.wav", "settings": settings }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let first_display = opened["displayId"].as_u64().unwrap();
    assert_eq!(first_display, 1, "user display ids are caller-owned");

    let (status, list) = response_json(
        client
            .get(format!("{}/api/displays", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(list[0]["id"], first_display);

    let (status, _) = response_json(
        client
            .get(format!("{}/api/spectrogram/{first_display}", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    let (status, cancelled) = response_json(
        client
            .post(format!("{}/api/analysis/cancel", server.base))
            .json(&json!({ "displayId": first_display }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(cancelled["cancelled"], true);
    let (_, cancelled_again) = response_json(
        client
            .post(format!("{}/api/analysis/cancel", server.base))
            .json(&json!({ "displayId": first_display }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(cancelled_again["cancelled"], false);
    let (_, list) = response_json(
        client
            .get(format!("{}/api/displays", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list, json!([]), "cancelled analyses are cleaned up");
    assert!(server.runtime().analysis_ids().is_empty());
    assert!(server.runtime().signal_ids().is_empty());

    // Complete a second pending analysis and read its grid/image.
    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({ "displayId": 2, "path": "second.wav", "settings": settings }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let second_display = opened["displayId"].as_u64().unwrap();
    let second_analysis = AnalysisId(opened["analysisId"].as_u64().unwrap());
    let job = server
        .runtime()
        .job_for(EntityId::Analysis(second_analysis))
        .expect("analysis job");
    server.app().complete_job(job).unwrap();

    let (status, payload) = response_json(
        client
            .get(format!("{}/api/spectrogram/{second_display}", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(payload["meta"]["frames"], 0);
    assert_eq!(payload["meta"]["bins"], 0);
    assert_eq!(payload["meta"]["sampleRate"], 1.0);
    let encoded = payload["data"].as_str().unwrap();
    assert!(base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap()
        .is_empty());

    let response = client
        .get(format!(
            "{}/api/spectrogram/{second_display}/image?width=32&height=16",
            server.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    let bytes = response.bytes().await.unwrap();
    let image = image::load_from_memory(&bytes).expect("PNG decode");
    assert_eq!((image.width(), image.height()), (32, 16));

    // Cancel leaves the analysis/display attached until the display closes.
    let _ = client
        .post(format!("{}/api/displays/close", server.base))
        .json(&json!({ "id": first_display }))
        .send()
        .await
        .unwrap();
    let status = client
        .post(format!("{}/api/displays/close", server.base))
        .json(&json!({ "id": second_display }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    assert!(server.runtime().analysis_ids().is_empty());
    assert!(server.runtime().signal_ids().is_empty());

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_emits_analysis_error_and_removes_the_display() {
    let server = start_host(TestHost::held()).await;
    let client = reqwest::Client::new();
    let mut socket = connect_async(format!("ws://{}/ws", server.addr()))
        .await
        .unwrap()
        .0;

    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({
                "displayId": 1,
                "path": "cancel.wav",
                "settings": AnalysisSettings::default(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let display_id = opened["displayId"].as_u64().unwrap();
    let event = next_ws_event(&mut socket).await;
    assert_eq!(event["type"], "display.open");

    let (status, cancelled) = response_json(
        client
            .post(format!("{}/api/analysis/cancel", server.base))
            .json(&json!({ "displayId": display_id }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(cancelled["cancelled"], true);
    let error = next_ws_event(&mut socket).await;
    assert_eq!(error["type"], "analysis.error");
    assert_eq!(error["displayId"], display_id);
    assert_eq!(error["message"], "analysis cancelled");
    let closed = next_ws_event(&mut socket).await;
    assert_eq!(closed["type"], "display.closed");
    assert_eq!(closed["id"], display_id);

    socket.close(None).await.unwrap();
    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_analysis_cleans_up_its_display_and_runtime_entities() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();
    server.app().fail_next_analysis("injected analysis failure");

    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({
                "displayId": 1,
                "path": "bad.wav",
                "settings": AnalysisSettings::default(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(opened["displayId"], 1);

    let (_, list) = response_json(
        client
            .get(format!("{}/api/displays", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        list,
        json!([]),
        "a failed analysis must not retain its display"
    );
    assert!(server.runtime().analysis_ids().is_empty());
    assert!(server.runtime().signal_ids().is_empty());
    assert!(server.runtime().job_ids().is_empty());

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn order_line_cuts_images_csv_and_empty_parent_behavior() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();
    let (_, analysis) = seed_analysis(&server);
    let (_, shown) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": analysis.get() }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let parent = shown["id"].as_u64().unwrap();
    let order_settings = serde_json::to_value(OrderCutSettings::default()).unwrap();

    let (status, order) = response_json(
        client
            .post(format!("{}/api/cuts/order", server.base))
            .json(&json!({ "parentId": parent, "settings": order_settings }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(order["cuts"].as_array().unwrap().len(), 3);
    assert_eq!(order["durationS"], 0.0);
    assert!(order["hopS"].is_number());

    let mut freq_settings = OrderCutSettings::default();
    freq_settings.orders.clear();
    freq_settings.freqs = "120, 440".into();
    let (status, frequency) = response_json(
        client
            .post(format!("{}/api/cuts/order", server.base))
            .json(&json!({ "parentId": parent, "settings": freq_settings }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(frequency["cuts"].as_array().unwrap().len(), 2);
    assert_eq!(frequency["cuts"][0]["kind"]["type"], "freq");

    let response = client
        .post(format!("{}/api/cuts/order/image", server.base))
        .json(&json!({ "parentId": parent, "settings": order_settings }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let order_png = image::load_from_memory(&response.bytes().await.unwrap()).unwrap();
    assert_eq!((order_png.width(), order_png.height()), (800, 400));

    let line_settings = serde_json::to_value(LineCutSettings::default()).unwrap();
    let line = LineSpec {
        t0: 0.0,
        f0: 100.0,
        t1: 1.0,
        f1: 200.0,
        log_f: false,
    };
    let (status, lines) = response_json(
        client
            .post(format!("{}/api/cuts/line", server.base))
            .json(&json!({
                "parentId": parent,
                "settings": line_settings,
                "lines": [line],
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(lines["cuts"].as_array().unwrap().len(), 1);

    let response = client
        .post(format!("{}/api/cuts/csv", server.base))
        .json(&json!({ "cuts": order["cuts"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/csv"
    );
    let csv = response.text().await.unwrap();
    assert_eq!(csv, "kind,value,time_s,rpm,freq_hz,db\n");

    // Unknown parents return a successful empty payload, matching the
    // child-before-parent mount behavior.
    let (status, empty) = response_json(
        client
            .post(format!("{}/api/cuts/order", server.base))
            .json(&json!({ "parentId": 999_999, "settings": OrderCutSettings::default() }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(empty["cuts"], json!([]));
    assert_eq!(empty["durationS"], 0.0);
    let (status, empty) = response_json(
        client
            .post(format!("{}/api/cuts/line", server.base))
            .json(&json!({
                "parentId": 999_999,
                "settings": LineCutSettings::default(),
                "lines": [],
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(empty["cuts"], json!([]));
    let response = client
        .post(format!("{}/api/cuts/order/image", server.base))
        .json(&json!({
            "parentId": 999_999,
            "settings": OrderCutSettings::default(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let empty_png = image::load_from_memory(&response.bytes().await.unwrap()).unwrap();
    assert_eq!((empty_png.width(), empty_png.height()), (800, 400));

    // Runtime cut failures map to a stable 400 error.
    server.app().fail_next_cut("injected cut failure");
    let (status, error) = response_json(
        client
            .post(format!("{}/api/cuts/order", server.base))
            .json(&json!({
                "parentId": parent,
                "settings": OrderCutSettings::default(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("analysis_failed"));

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_session_theme_and_config_round_trip() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();

    let (status, settings) = response_json(
        client
            .get(format!("{}/api/settings", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(settings["appearance"]["theme"], "dark");

    let mut changed = settings.clone();
    changed["appearance"]["theme"] = json!("light");
    changed["appearance"]["ui_scale"] = json!(1.5);
    let status = client
        .put(format!("{}/api/settings", server.base))
        .json(&changed)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    let (_, loaded) = response_json(
        client
            .get(format!("{}/api/settings", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(loaded["appearance"]["theme"], "light");
    assert_eq!(loaded["appearance"]["ui_scale"], 1.5);

    let status = client
        .post(format!("{}/api/settings/defaults", server.base))
        .json(&json!({
            "analysis": settings["analysis"],
            "display": settings["display"],
        }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);

    let (status, session) = response_json(
        client
            .get(format!("{}/api/session", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(session.is_null());
    let session = json!({
        "version": 1,
        "nextId": 3,
        "displays": [],
        "recentFiles": ["clip.wav"],
        "layoutPresets": {},
        "layout": null
    });
    let status = client
        .put(format!("{}/api/session", server.base))
        .json(&session)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    let (_, saved) = response_json(
        client
            .get(format!("{}/api/session", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(saved["recentFiles"][0], "clip.wav");

    let (status, paths) = response_json(
        client
            .get(format!("{}/api/config/paths", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(paths["configDir"]
        .as_str()
        .unwrap()
        .ends_with(server.temp.path().file_name().unwrap().to_str().unwrap()));
    assert!(paths["settingsPath"]
        .as_str()
        .unwrap()
        .ends_with("settings.toml"));
    assert!(paths["sessionPath"]
        .as_str()
        .unwrap()
        .ends_with("session.json"));
    assert!(paths["themesDir"].as_str().unwrap().ends_with("themes"));

    let status = client
        .post(format!("{}/api/open-path", server.base))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);

    let (status, light) = response_json(
        client
            .get(format!("{}/api/theme?name=light", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(light["appearance"], "light");

    let (status, themes) = response_json(
        client
            .get(format!("{}/api/themes", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(themes.as_array().unwrap().len() >= 2);

    let (status, duplicated) = response_json(
        client
            .post(format!("{}/api/themes/duplicate", server.base))
            .json(&json!({ "source": "light", "name": "Ocean Blue!" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let theme_id = duplicated["id"].as_str().unwrap();
    assert_eq!(theme_id, "ocean-blue");
    let (status, _) = response_json(
        client
            .get(format!("{}/api/theme?name={theme_id}", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let status = client
        .post(format!("{}/api/themes/duplicate", server.base))
        .json(&json!({ "source": "light", "name": "Ocean Blue!" }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::CONFLICT);

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_round_trip_and_invalid_upload_cleanup() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();

    let status = client
        .post(format!("{}/api/upload", server.base))
        .multipart(Form::new())
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_dir(&server.uploads_dir).unwrap().count(),
        0,
        "invalid uploads must not leave files"
    );

    let status = client
        .post(format!("{}/api/upload", server.base))
        .header(CONTENT_TYPE, "multipart/form-data; boundary=broken")
        .body("not-a-multipart-body")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read_dir(&server.uploads_dir).unwrap().count(), 0);

    let (status, uploaded) = response_json(
        client
            .post(format!("{}/api/upload", server.base))
            .multipart(
                Form::new().part(
                    "file",
                    Part::bytes(b"hello upload".to_vec())
                        .file_name("clip.wav")
                        .mime_str("audio/wav")
                        .unwrap(),
                ),
            )
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(uploaded["name"], "clip.wav");
    let path = PathBuf::from(uploaded["path"].as_str().unwrap());
    assert!(path.starts_with(&server.uploads_dir));
    assert_eq!(std::fs::read(&path).unwrap(), b"hello upload");
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("clip.wav"));

    // The API layer raises axum's default body limit so real audio files work.
    let large = vec![7u8; 2 * 1024 * 1024 + 1024];
    let (status, uploaded) = response_json(
        client
            .post(format!("{}/api/upload", server.base))
            .multipart(
                Form::new().part(
                    "file",
                    Part::bytes(large.clone())
                        .file_name("large.wav")
                        .mime_str("audio/wav")
                        .unwrap(),
                ),
            )
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let path = PathBuf::from(uploaded["path"].as_str().unwrap());
    assert_eq!(std::fs::read(&path).unwrap().len(), large.len());

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cors_allows_loopback_origins_only() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();
    let info = format!("{}/api/app/info", server.base);

    let response = client
        .get(&info)
        .header(ORIGIN, "http://127.0.0.1:5173")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap()
            .to_str()
            .unwrap(),
        "http://127.0.0.1:5173"
    );

    let response = client
        .get(&info)
        .header(ORIGIN, "https://localhost:5173")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap()
            .to_str()
            .unwrap(),
        "https://localhost:5173"
    );

    let response = client
        .get(&info)
        .header(ORIGIN, "http://evil.example")
        .send()
        .await
        .unwrap();
    assert!(
        response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "only loopback origins may receive CORS permission"
    );

    let response = client
        .request(reqwest::Method::OPTIONS, &info)
        .header(ORIGIN, "http://localhost:5173")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let methods = response
        .headers()
        .get(ACCESS_CONTROL_ALLOW_METHODS)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(methods.contains("POST"));

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn script_routes_are_absent_and_ws_has_no_script_events() {
    let server = start_host(TestHost::held()).await;
    let client = reqwest::Client::new();
    for (method, path) in [
        (reqwest::Method::GET, "/api/script/run"),
        (reqwest::Method::POST, "/api/script/exec"),
        (reqwest::Method::GET, "/api/scripting"),
    ] {
        let response = client
            .request(method, format!("{}{path}", server.base))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }

    let mut socket = connect_async(format!("ws://{}/ws", server.addr()))
        .await
        .unwrap()
        .0;
    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({
                "displayId": 1,
                "path": "first.wav",
                "settings": AnalysisSettings::default(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let first_display = opened["displayId"].as_u64().unwrap();
    let first_analysis = AnalysisId(opened["analysisId"].as_u64().unwrap());

    let event = next_ws_event(&mut socket).await;
    assert_eq!(event["type"], "display.open");
    assert_eq!(event["display"]["id"], first_display);

    let job = server
        .runtime()
        .job_for(EntityId::Analysis(first_analysis))
        .unwrap();
    server.app().complete_job(job).unwrap();
    let progress = next_ws_event(&mut socket).await;
    assert_eq!(progress["type"], "analysis.progress");
    assert_eq!(progress["displayId"], first_display);
    assert_eq!(progress["framesDone"], 1);
    let done = next_ws_event(&mut socket).await;
    assert_eq!(done["type"], "analysis.done");
    assert_eq!(done["displayId"], first_display);

    server
        .app()
        .update_display(
            DisplayId(first_display),
            json!({ "analysisId": first_analysis.get(), "zoom": 2.5 }),
            true,
        )
        .unwrap();
    let updated = next_ws_event(&mut socket).await;
    assert_eq!(updated["type"], "display.updated");
    assert_eq!(updated["display"]["id"], first_display);
    assert_eq!(updated["display"]["state"]["zoom"], 2.5);

    let (status, opened) = response_json(
        client
            .post(format!("{}/api/spectrogram/open", server.base))
            .json(&json!({
                "displayId": 2,
                "path": "second.wav",
                "settings": AnalysisSettings::default(),
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let second_display = opened["displayId"].as_u64().unwrap();
    let second_analysis = AnalysisId(opened["analysisId"].as_u64().unwrap());
    let event = next_ws_event(&mut socket).await;
    assert_eq!(event["type"], "display.open");
    assert_eq!(event["display"]["id"], second_display);

    let job = server
        .runtime()
        .job_for(EntityId::Analysis(second_analysis))
        .unwrap();
    server.app().fail_job(job, "injected failure").unwrap();
    let error = next_ws_event(&mut socket).await;
    assert_eq!(error["type"], "analysis.error");
    assert_eq!(error["displayId"], second_display);
    assert_eq!(error["message"], "injected failure");
    // Failed analyses are cleaned up after the advisory error.
    let failed_closed = next_ws_event(&mut socket).await;
    assert_eq!(failed_closed["type"], "display.closed");
    assert_eq!(failed_closed["id"], second_display);

    let status = client
        .post(format!("{}/api/displays/close", server.base))
        .json(&json!({ "id": first_display }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::NO_CONTENT);
    let closed = next_ws_event(&mut socket).await;
    assert_eq!(closed["type"], "display.closed");
    assert_eq!(closed["id"], first_display);
    assert_eq!(closed["children"], json!([]));

    // Every event above is a display/analysis event; the adapter has no script
    // event variants to translate.
    socket.close(None).await.unwrap();
    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_app_errors_map_to_stable_statuses() {
    let server = start_host(TestHost::held()).await;
    let client = reqwest::Client::new();
    let runtime = server.runtime();
    let _signal = runtime
        .load_signal(LoadSignalRequest::new(vec![0.0; 16], 8_000.0))
        .unwrap();
    let analysis = runtime
        .analyze(_signal, &AnalysisSettings::default())
        .unwrap();

    let (status, error) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": analysis.get() }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("busy"));

    let (status, error) = response_json(
        client
            .get(format!("{}/api/displays/12345", server.base))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert!(error["error"].as_str().unwrap().contains("not found"));

    let (status, error) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": 99_999 }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert!(error["error"].as_str().unwrap().contains("not_found"));

    // A closed app is a 400 typed error rather than a transport 500.
    server.app().shutdown();
    let (status, error) = response_json(
        client
            .post(format!("{}/api/displays/spectrogram", server.base))
            .json(&json!({ "analysisId": analysis.get() }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("closed"));

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn static_index_is_injected_and_missing_assets_are_404() {
    let server = start(TestApp::new()).await;
    std::fs::write(
        server.dist.join("index.html"),
        "<html><head><title>fft</title></head><body>app</body></html>",
    )
    .unwrap();
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let html = response.text().await.unwrap();
    assert!(html.contains("window.__FFT_HEADLESS__=true"));
    assert!(html.contains("<body>app</body>"));

    let response = client
        .get(format!("{}/missing-asset.js", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    let _ = server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plot_load_maps_columns_pairs_and_reports_bad_tables() {
    let server = start(TestApp::new()).await;
    let client = reqwest::Client::new();
    let path = server.temp.path().join("series.csv");
    std::fs::write(&path, "x,a,b\n0,1,10\n1,2,20\n2,3,30\n").unwrap();
    let file = path.to_string_lossy().into_owned();

    let (status, columns) = response_json(
        client
            .post(format!("{}/api/plot", server.base))
            .json(&json!({ "path": file, "layout": "columns" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(columns["xLabel"], "x");
    assert_eq!(columns["rows"], 3);
    assert_eq!(columns["series"].as_array().unwrap().len(), 2);
    assert_eq!(columns["series"][0]["label"], "a");
    assert_eq!(columns["series"][0]["points"][1]["x"], 1.0);
    assert_eq!(columns["series"][0]["points"][1]["y"], 2.0);

    let (status, pairs) = response_json(
        client
            .post(format!("{}/api/plot", server.base))
            .json(&json!({ "path": file, "layout": "pairs" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(pairs["series"].as_array().unwrap().len(), 1);
    assert!(pairs["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap_or("").contains("trailing column 'b'")));

    let (status, _) = response_json(
        client
            .post(format!("{}/api/plot", server.base))
            .json(&json!({ "path": "/does/not/exist.csv", "layout": "columns" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);

    let _ = server.stop().await;
}

// A dropped handle requests shutdown and aborts the task if graceful shutdown
// does not finish promptly; either way the state-owned lease must be released.
#[tokio::test(flavor = "multi_thread")]
async fn lease_is_released_when_the_server_handle_is_dropped() {
    let temp = tempfile::tempdir().unwrap();
    let host = TestHost::new();
    let config = HttpAdapterConfig::new(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        temp.path().join("dist"),
        temp.path(),
    );
    let adapter = HttpAdapter::new(config, host.lease()).unwrap();
    let handle = adapter.serve().await.unwrap();
    assert_eq!(host.active_leases(), 1);
    assert!(handle.addr().ip().is_loopback());
    assert_ne!(handle.addr().port(), 0);
    assert_eq!(handle.addr(), handle.local_addr());
    drop(handle);
    assert!(host.active_leases() <= 1);
    // Dropping the handle requests shutdown; the lease is released once the
    // aborted server task and its request state are dropped.
    for _ in 0..100 {
        if host.active_leases() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    host.assert_all_leases_dropped();
}
