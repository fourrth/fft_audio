//! Shipping matrix runner for the `fft_audio` workspace.
//!
//! Reads a TOML manifest of cargo jobs (package + feature set + binaries),
//! runs cargo in the workspace root, and copies built binaries into per-job
//! output folders. It can also run an ad-hoc `--package` job selected entirely
//! from the CLI.

#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Cargo profile used for a job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Debug,
    #[default]
    Release,
}

impl Profile {
    pub const fn cargo_flag(self) -> Option<&'static str> {
        match self {
            Profile::Debug => None,
            Profile::Release => Some("--release"),
        }
    }

    pub const fn artifact_dir(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        })
    }
}

/// One package/feature build in the shipping matrix.
#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub name: String,
    pub package: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub no_default_features: bool,
    #[serde(default)]
    pub all_features: bool,
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub cargo_args: Vec<String>,
    #[serde(default)]
    pub command: Option<String>,
}

/// TOML manifest consumed by the shipping tool.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub profile: Option<Profile>,
    #[serde(default)]
    pub target_dir: Option<PathBuf>,
    #[serde(default)]
    pub out_dir: Option<PathBuf>,
    #[serde(default)]
    pub fetch: Option<bool>,
    #[serde(default)]
    pub copy_binaries: Option<bool>,
    #[serde(default)]
    pub jobs: Vec<Job>,
}

/// CLI-provided options after defaults have been resolved.
#[derive(Clone, Debug)]
pub struct Options {
    pub workspace_root: PathBuf,
    pub manifest_path: PathBuf,
    /// Selected job names. Empty selects every manifest job.
    pub job_names: Vec<String>,
    pub ad_hoc: Option<Job>,
    pub command: Option<String>,
    pub profile: Option<Profile>,
    pub target_dir: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub fetch: Option<bool>,
    pub copy_binaries: Option<bool>,
    pub cargo_args: Vec<String>,
    pub keep_going: bool,
    pub dry_run: bool,
    pub list: bool,
}

/// Run the shipping matrix. All paths in `Options` are resolved relative to
/// `workspace_root` unless absolute.
pub fn run(options: Options) -> Result<(), String> {
    let root = options
        .workspace_root
        .canonicalize()
        .map_err(|error| format!("could not resolve workspace root: {error}"))?;
    if !root.join("Cargo.toml").is_file() {
        return Err(format!(
            "{} does not contain a Cargo.toml; pass --workspace-root",
            root.display()
        ));
    }

    let manifest = if options.ad_hoc.is_some() {
        Manifest::default()
    } else {
        load_manifest(&root, &options.manifest_path)?
    };

    let jobs = if let Some(job) = options.ad_hoc.clone() {
        if options.list {
            return Err("--list cannot be used with an ad-hoc --package".to_string());
        }
        vec![job]
    } else {
        select_jobs(&manifest.jobs, &options.job_names)?
    };

    if options.list {
        list_jobs(&jobs, &manifest);
        return Ok(());
    }
    if jobs.is_empty() {
        return Err("the manifest contains no jobs".to_string());
    }

    let command = options
        .command
        .clone()
        .or(manifest.command.clone())
        .unwrap_or_else(|| "build".to_string());
    if command.trim().is_empty() {
        return Err("cargo command cannot be empty".to_string());
    }
    let profile = options.profile.or(manifest.profile).unwrap_or_default();
    let target_dir = resolve(
        &root,
        options
            .target_dir
            .clone()
            .or(manifest.target_dir.clone())
            .unwrap_or_else(|| PathBuf::from("target/shipping")),
    );
    let out_dir = resolve(
        &root,
        options
            .out_dir
            .clone()
            .or(manifest.out_dir.clone())
            .unwrap_or_else(|| PathBuf::from("dist")),
    );
    let fetch = options.fetch.or(manifest.fetch).unwrap_or(true);
    let copy_binaries = options
        .copy_binaries
        .or(manifest.copy_binaries)
        .unwrap_or(true);

    println!(
        "fft_shipping: {} job(s), command={command}, profile={profile}",
        jobs.len()
    );
    println!("  workspace: {}", root.display());
    println!("  target-dir: {}", target_dir.display());
    println!("  out-dir: {}", out_dir.display());

    if fetch {
        run_cargo(
            &root,
            &["fetch".to_string()],
            options.dry_run,
            "fetch dependencies",
        )?;
    }

    let mut failures = Vec::new();
    for job in &jobs {
        let job_command = job.command.clone().unwrap_or_else(|| command.clone());
        let result = run_job(
            &root,
            job,
            &job_command,
            profile,
            &target_dir,
            &out_dir,
            &options.cargo_args,
            copy_binaries,
            options.dry_run,
        );
        if let Err(error) = result {
            eprintln!("fft_shipping: job `{}` failed: {error}", job.name);
            failures.push(job.name.clone());
            if !options.keep_going {
                break;
            }
        }
    }

    if failures.is_empty() {
        println!("fft_shipping: complete ({} job(s))", jobs.len());
        Ok(())
    } else {
        Err(format!(
            "{} job(s) failed: {}",
            failures.len(),
            failures.join(", ")
        ))
    }
}

fn load_manifest(root: &Path, manifest_path: &Path) -> Result<Manifest, String> {
    let path = resolve(root, manifest_path.to_path_buf());
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read manifest {}: {error}", path.display()))?;
    toml::from_str(&text).map_err(|error| format!("invalid manifest {}: {error}", path.display()))
}

fn select_jobs(jobs: &[Job], names: &[String]) -> Result<Vec<Job>, String> {
    if names.is_empty() {
        return Ok(jobs.to_vec());
    }
    let mut selected = Vec::new();
    for name in names {
        let job = jobs
            .iter()
            .find(|job| &job.name == name)
            .ok_or_else(|| format!("unknown job `{name}`"))?;
        selected.push(job.clone());
    }
    Ok(selected)
}

fn list_jobs(jobs: &[Job], manifest: &Manifest) {
    for job in jobs {
        let mut feature_state = Vec::new();
        if job.no_default_features {
            feature_state.push("no-default-features".to_string());
        }
        if job.all_features {
            feature_state.push("all-features".to_string());
        }
        if !job.features.is_empty() {
            feature_state.push(format!("features={}", job.features.join(",")));
        }
        let bins = if job.bins.is_empty() {
            "-".to_string()
        } else {
            job.bins.join(",")
        };
        let command = job
            .command
            .as_deref()
            .or(manifest.command.as_deref())
            .unwrap_or("build");
        println!(
            "{:<28} package={:<20} command={:<8} bins={:<20} {}",
            job.name,
            job.package,
            command,
            bins,
            feature_state.join(" ")
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn run_job(
    root: &Path,
    job: &Job,
    command: &str,
    profile: Profile,
    target_dir: &Path,
    out_dir: &Path,
    extra_args: &[String],
    copy_binaries: bool,
    dry_run: bool,
) -> Result<(), String> {
    println!(
        "fft_shipping: [{}] {} {} (profile={profile})",
        job.name, job.package, command
    );

    let args = cargo_args_for(job, command, profile, target_dir, extra_args);
    run_cargo(root, &args, dry_run, &format!("job {}", job.name))?;

    if command == "build" && copy_binaries && !job.bins.is_empty() {
        let source_dir = target_dir.join(profile.artifact_dir());
        for bin in &job.bins {
            copy_binary(
                &source_dir.join(format!("{bin}{}", std::env::consts::EXE_SUFFIX)),
                &out_dir
                    .join(&job.name)
                    .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX)),
                dry_run,
            )?;
        }
    }
    Ok(())
}

fn cargo_args_for(
    job: &Job,
    command: &str,
    profile: Profile,
    target_dir: &Path,
    extra_args: &[String],
) -> Vec<String> {
    let mut args = vec![command.to_string()];
    args.push("--package".to_string());
    args.push(job.package.clone());
    if job.no_default_features {
        args.push("--no-default-features".to_string());
    }
    if job.all_features {
        args.push("--all-features".to_string());
    }
    if !job.features.is_empty() {
        args.push("--features".to_string());
        args.push(job.features.join(","));
    }
    if let Some(flag) = profile.cargo_flag() {
        args.push(flag.to_string());
    }
    args.push("--target-dir".to_string());
    args.push(target_dir.to_string_lossy().into_owned());
    for bin in &job.bins {
        args.push("--bin".to_string());
        args.push(bin.clone());
    }
    args.extend(extra_args.iter().cloned());
    args.extend(job.cargo_args.iter().cloned());
    args
}

fn run_cargo(root: &Path, args: &[String], dry_run: bool, label: &str) -> Result<(), String> {
    let rendered = format!("cargo {}", args.join(" "));
    if dry_run {
        println!("{rendered}");
        return Ok(());
    }
    println!("{rendered}");
    let status = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("could not run cargo for {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with {status}"))
    }
}

fn copy_binary(source: &Path, destination: &Path, dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("copy {} -> {}", source.display(), destination.display());
        return Ok(());
    }
    if !source.is_file() {
        return Err(format!(
            "built binary not found at {}; check the job `bins` and profile",
            source.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::copy(source, destination).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    println!("copy {} -> {}", source.display(), destination.display());
    Ok(())
}

fn resolve(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}
