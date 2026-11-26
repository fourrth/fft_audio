//! Build the Vite frontend and Tauri resources for the product.
//!
//! The web asset build is deliberately gated and incremental:
//!
//! - only runs when the `webui` feature is enabled (`http` and the Tauri GUI
//!   features);
//! - only runs when `web/dist` is missing or older than the web inputs;
//! - can be skipped with `FFT_SKIP_WEB_BUILD=1` for Rust-only work.
//!
//! It never runs `npm install`; missing dependencies fail with a clear message
//! instead of silently downloading packages. Tauri's build step runs after the
//! web assets are ready and only for GUI-feature builds.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

const WEB_INPUTS: &[&str] = &[
    "src",
    "index.html",
    "package.json",
    "package-lock.json",
    "vite.config.ts",
    "tsconfig.json",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FFT_SKIP_WEB_BUILD");

    build_web_assets();
    build_tauri_resources();
}

#[cfg(any(feature = "tauri", feature = "tauri_python"))]
fn build_tauri_resources() {
    tauri_build::build();
}

#[cfg(not(any(feature = "tauri", feature = "tauri_python")))]
fn build_tauri_resources() {}

fn build_web_assets() {
    if env::var_os("CARGO_FEATURE_WEBUI").is_none() {
        return;
    }

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"),
    );
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("fft_desktop lives under crates/");
    let web = repo_root.join("web");
    if !web.is_dir() {
        println!(
            "cargo:warning=web directory not found at {}; skipping web asset build",
            web.display()
        );
        return;
    }

    let dist_index = web.join("dist").join("index.html");
    for input in WEB_INPUTS {
        println!("cargo:rerun-if-changed={}", web.join(input).display());
    }
    println!("cargo:rerun-if-changed={}", dist_index.display());

    if env::var_os("FFT_SKIP_WEB_BUILD").is_some() {
        println!("cargo:warning=FFT_SKIP_WEB_BUILD is set; skipping web asset build");
        return;
    }

    if web_assets_are_fresh(&web, &dist_index) {
        return;
    }

    if !web.join("node_modules").is_dir() {
        panic!(
            "web assets are required for the `webui` feature but {}/node_modules is missing; \
             run `npm --prefix web install` or set FFT_SKIP_WEB_BUILD=1",
            web.display()
        );
    }

    println!("cargo:warning=building web assets because web/dist is missing or stale");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let status = Command::new(npm)
        .args(["run", "build"])
        .current_dir(&web)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "could not run `{npm} run build` in {}: {error}; \
                 install Node/npm or set FFT_SKIP_WEB_BUILD=1",
                web.display()
            )
        });
    if !status.success() {
        panic!(
            "`{npm} run build` failed in {}; fix the frontend or set FFT_SKIP_WEB_BUILD=1",
            web.display()
        );
    }
}

fn web_assets_are_fresh(web: &Path, dist_index: &Path) -> bool {
    let Ok(index_mtime) = fs::metadata(dist_index).and_then(|metadata| metadata.modified()) else {
        return false;
    };

    let mut newest: Option<SystemTime> = None;
    for input in WEB_INPUTS {
        let path = web.join(input);
        if path.is_dir() {
            scan_newest(&path, &mut newest);
        } else {
            record_newest(&path, &mut newest);
        }
    }

    newest.is_none_or(|newest| newest <= index_mtime)
}

fn scan_newest(path: &Path, newest: &mut Option<SystemTime>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_newest(&path, newest);
        } else {
            record_newest(&path, newest);
        }
    }
}

fn record_newest(path: &Path, newest: &mut Option<SystemTime>) {
    let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return;
    };
    if newest.is_none_or(|current| modified > current) {
        *newest = Some(modified);
    }
}
