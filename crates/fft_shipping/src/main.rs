//! Shipping matrix CLI for `fft_audio`.
//!
//! Examples:
//!
//! ```text
//! fft_shipping --list
//! fft_shipping --dry-run
//! fft_shipping --job desktop-headless-python
//! fft_shipping --command check --no-fetch
//! fft_shipping --package fft_desktop --no-default-features \
//!   --features http,python --bin fft_desktop --release
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use fft_shipping::{Job, Options, Profile};

#[derive(Debug, Parser)]
#[command(
    name = "fft_shipping",
    version,
    about = "Run fft_audio shipping builds and cargo commands"
)]
struct Cli {
    /// Workspace root. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    workspace_root: Option<PathBuf>,

    /// Shipping manifest. Defaults to <workspace-root>/shipping.toml.
    #[arg(long, value_name = "FILE")]
    manifest: Option<PathBuf>,

    /// Select a named manifest job (repeatable). No --job runs every job.
    #[arg(long = "job", value_name = "NAME")]
    jobs: Vec<String>,

    /// Cargo subcommand to run for each job. Defaults to build.
    #[arg(long, value_name = "SUBCOMMAND")]
    command: Option<String>,

    /// Build with the release profile. This is the manifest default.
    #[arg(long, conflicts_with = "debug")]
    release: bool,

    /// Build with the debug profile.
    #[arg(long)]
    debug: bool,

    /// Override the cargo target directory.
    #[arg(long, value_name = "DIR")]
    target_dir: Option<PathBuf>,

    /// Override the binary output directory.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Run cargo fetch before the jobs.
    #[arg(long, conflicts_with = "no_fetch")]
    fetch: bool,

    /// Do not run cargo fetch before the jobs.
    #[arg(long)]
    no_fetch: bool,

    /// Extra argument passed to every cargo invocation (repeatable).
    #[arg(long = "cargo-arg", value_name = "ARG")]
    cargo_args: Vec<String>,

    /// Continue running later jobs after a failure.
    #[arg(long)]
    keep_going: bool,

    /// Print commands and copy operations without running them.
    #[arg(long)]
    dry_run: bool,

    /// List the selected jobs and exit.
    #[arg(long)]
    list: bool,

    /// Do not copy built binaries into the output directory.
    #[arg(long)]
    no_copy: bool,

    /// Ad-hoc package to build instead of using a manifest job.
    #[arg(long, value_name = "PACKAGE", conflicts_with = "jobs")]
    package: Option<String>,

    /// Name for the ad-hoc job/output directory.
    #[arg(long, value_name = "NAME", requires = "package")]
    name: Option<String>,

    /// Ad-hoc cargo features (comma-separated or repeatable).
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "FEATURES",
        requires = "package"
    )]
    features: Vec<String>,

    /// Disable default features for the ad-hoc job.
    #[arg(long, requires = "package")]
    no_default_features: bool,

    /// Enable all features for the ad-hoc job.
    #[arg(long, requires = "package")]
    all_features: bool,

    /// Ad-hoc binary target to build and copy (repeatable).
    #[arg(long = "bin", value_name = "BIN", requires = "package")]
    bins: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fft_shipping: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    if cli.package.is_some() && cli.manifest.is_some() {
        return Err("--manifest cannot be combined with an ad-hoc --package".to_string());
    }

    let workspace_root = cli
        .workspace_root
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let profile = if cli.release {
        Some(Profile::Release)
    } else if cli.debug {
        Some(Profile::Debug)
    } else {
        None
    };
    let fetch = if cli.fetch {
        Some(true)
    } else if cli.no_fetch {
        Some(false)
    } else {
        None
    };
    let copy_binaries = if cli.no_copy { Some(false) } else { None };

    let ad_hoc = cli.package.map(|package| {
        let name = cli.name.unwrap_or_else(|| package.clone());
        Job {
            name,
            package,
            features: cli.features,
            no_default_features: cli.no_default_features,
            all_features: cli.all_features,
            bins: cli.bins,
            cargo_args: Vec::new(),
            command: None,
        }
    });

    fft_shipping::run(Options {
        workspace_root,
        manifest_path: cli
            .manifest
            .unwrap_or_else(|| PathBuf::from("shipping.toml")),
        job_names: cli.jobs,
        ad_hoc,
        command: cli.command,
        profile,
        target_dir: cli.target_dir,
        out_dir: cli.out_dir,
        fetch,
        copy_binaries,
        cargo_args: cli.cargo_args,
        keep_going: cli.keep_going,
        dry_run: cli.dry_run,
        list: cli.list,
    })
}
