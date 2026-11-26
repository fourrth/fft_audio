use super::*;
use tempfile::tempdir;

fn job() -> Job {
    Job {
        name: "headless".into(),
        package: "fft_desktop".into(),
        features: vec!["http".into(), "python".into()],
        no_default_features: true,
        all_features: false,
        bins: vec!["fft_desktop".into()],
        cargo_args: vec!["--locked".into()],
        command: Some("build".into()),
    }
}

#[test]
fn manifest_parses_defaults_and_jobs() {
    let manifest: Manifest = toml::from_str(
        r#"
        command = "check"
        profile = "debug"
        target_dir = "target/shipping"
        out_dir = "dist"
        fetch = false
        copy_binaries = false

        [[jobs]]
        name = "desktop"
        package = "fft_desktop"
        features = ["tauri_python"]
        bins = ["fft_desktop"]

        [[jobs]]
        name = "headless"
        package = "fft_desktop"
        no_default_features = true
        features = ["http"]
        "#,
    )
    .unwrap();

    assert_eq!(manifest.command.as_deref(), Some("check"));
    assert_eq!(manifest.profile, Some(Profile::Debug));
    assert_eq!(manifest.jobs.len(), 2);
    assert_eq!(manifest.jobs[0].features, ["tauri_python"]);
    assert!(manifest.jobs[1].no_default_features);
    assert!(manifest.jobs[1].bins.is_empty());
}

#[test]
fn job_selection_runs_all_or_named_jobs() {
    let jobs = vec![
        Job {
            name: "a".into(),
            package: "fft_app".into(),
            features: vec![],
            no_default_features: false,
            all_features: false,
            bins: vec![],
            cargo_args: vec![],
            command: None,
        },
        Job {
            name: "b".into(),
            package: "fft_test".into(),
            features: vec![],
            no_default_features: false,
            all_features: false,
            bins: vec![],
            cargo_args: vec![],
            command: None,
        },
    ];

    assert_eq!(select_jobs(&jobs, &[]).unwrap().len(), 2);
    let selected = select_jobs(&jobs, &["b".into()]).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].package, "fft_test");
    assert!(select_jobs(&jobs, &["missing".into()]).is_err());
}

#[test]
fn cargo_args_include_package_features_profile_and_bins() {
    let args = cargo_args_for(
        &job(),
        "build",
        Profile::Release,
        Path::new("/tmp/target"),
        &["--all-targets".into()],
    );
    assert_eq!(
        args,
        vec![
            "build",
            "--package",
            "fft_desktop",
            "--no-default-features",
            "--features",
            "http,python",
            "--release",
            "--target-dir",
            "/tmp/target",
            "--bin",
            "fft_desktop",
            "--all-targets",
            "--locked",
        ]
    );
}

#[test]
fn debug_profile_has_no_release_flag() {
    let args = cargo_args_for(
        &job(),
        "check",
        Profile::Debug,
        Path::new("/tmp/target"),
        &[],
    );
    assert!(!args.iter().any(|arg| arg == "--release"));
    assert_eq!(Profile::Debug.artifact_dir(), "debug");
    assert_eq!(Profile::Release.cargo_flag(), Some("--release"));
}

#[test]
fn dry_run_does_not_execute_cargo_or_copy() {
    let root = tempdir().unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let options = Options {
        workspace_root: root.path().to_path_buf(),
        manifest_path: root.path().join("shipping.toml"),
        job_names: vec![],
        ad_hoc: Some(job()),
        command: Some("build".into()),
        profile: Some(Profile::Release),
        target_dir: Some(root.path().join("target/shipping")),
        out_dir: Some(root.path().join("dist")),
        fetch: Some(true),
        copy_binaries: Some(true),
        cargo_args: vec![],
        keep_going: false,
        dry_run: true,
        list: false,
    };

    run(options).unwrap();
    assert!(!root.path().join("dist").exists());
}

#[test]
fn copy_binary_copies_and_creates_output_dirs() {
    let root = tempdir().unwrap();
    let source = root.path().join("target/release/fft_desktop");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, b"binary").unwrap();
    let destination = root.path().join("dist/desktop/fft_desktop");

    copy_binary(&source, &destination, false).unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"binary");
}

#[test]
fn missing_binary_is_an_error_without_dry_run() {
    let root = tempdir().unwrap();
    let error = copy_binary(
        &root.path().join("missing"),
        &root.path().join("dist/missing"),
        false,
    )
    .unwrap_err();
    assert!(error.contains("built binary not found"));
}
