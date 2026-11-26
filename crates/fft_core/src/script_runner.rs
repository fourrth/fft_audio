//! UI-neutral helpers for the Scripting display: example-file validation and
//! writing bundled Python examples under the config directory.

use std::path::{Path, PathBuf};

use crate::traits::Result;

/// Validate an example filename before joining it to the examples directory.
///
/// Only single, plain Python file names are accepted: no directories, no
/// hidden/traversal names, and a `.py` extension.
pub fn safe_script_filename(filename: &str) -> Result<String> {
    let name = filename.trim();
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
        return Err("script filename must not be empty, hidden or a traversal".into());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "._-".contains(ch))
    {
        return Err("script filename may only contain letters, digits, '.', '_' and '-'".into());
    }
    let path = Path::new(name);
    if path.components().count() != 1
        || path.file_name().and_then(|value| value.to_str()) != Some(name)
        || path.file_stem().is_none()
    {
        return Err("script filename must be a single plain file name".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension != "py" {
        return Err("script examples must use a .py extension".into());
    }
    Ok(name.to_string())
}

/// Write a bundled example under `<config_dir>/scripting-examples/`.
/// The filename is validated so traversal and path separators are rejected.
pub fn write_script_example(config_dir: &Path, filename: &str, contents: &str) -> Result<PathBuf> {
    let name = safe_script_filename(filename)?;
    let config_dir = std::path::absolute(config_dir)?;
    let directory = config_dir.join("scripting-examples");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_writes_example_scripts() {
        let dir = std::env::temp_dir().join(format!("fft_script_examples_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        for bad in [
            "../escape.py",
            "nested/run.py",
            "nested\\run.py",
            "run.txt",
            "run.sh",
            ".hidden.py",
            "",
        ] {
            assert!(
                safe_script_filename(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }

        let path = write_script_example(&dir, "demo.py", "print('ok')\n").unwrap();
        assert_eq!(path, dir.join("scripting-examples").join("demo.py"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "print('ok')\n");

        let _ = std::fs::remove_dir_all(dir);
    }
}
