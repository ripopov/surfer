//! Turns the VDB elaboration record into a language-server launch plan.
//!
//! The simulator recorded which files it elaborated, from which directory, with which
//! include directories and defines. That is exactly the input a second frontend needs to
//! elaborate the same design, so no user configuration is required.

use camino::{Utf8Path, Utf8PathBuf};
use std::fmt::Write as _;

/// Everything needed to start `slang-server` for one design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Directory the server runs in and reports as its workspace folder.
    pub workspace: Utf8PathBuf,
    /// Contents of the build file handed to `slang.setBuildFile`.
    pub build_file: String,
    /// Absolute design files, in elaboration order.
    pub files: Vec<Utf8PathBuf>,
    /// Top module name.
    pub top: String,
}

impl LaunchPlan {
    /// `companion_dir` holds the VDB; relative paths in the record resolve against the
    /// recorded working directory when it still exists, else against the companion.
    pub fn from_elaboration(
        elaboration: &vtr_vdb::Elaboration,
        top: &str,
        companion_dir: &Utf8Path,
    ) -> Self {
        let work_dir = Utf8PathBuf::from(&elaboration.work_dir);
        let workspace = if work_dir.is_absolute() {
            work_dir
        } else {
            normalize(&companion_dir.join(work_dir))
        };
        let workspace = if workspace.is_dir() {
            workspace
        } else {
            companion_dir.to_owned()
        };
        let resolve = |path: &str| -> Utf8PathBuf {
            let path = Utf8PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                normalize(&workspace.join(path))
            }
        };
        let files: Vec<_> = elaboration.files.iter().map(|f| resolve(f)).collect();
        let mut build_file = String::new();
        for dir in &elaboration.include_dirs {
            let _ = writeln!(build_file, "+incdir+{}", resolve(dir));
        }
        let exts: Vec<_> = elaboration
            .library_exts
            .iter()
            .filter(|ext| !ext.is_empty())
            .cloned()
            .collect();
        if !exts.is_empty() && !elaboration.include_dirs.is_empty() {
            for dir in &elaboration.include_dirs {
                let _ = writeln!(build_file, "-y {}", resolve(dir));
            }
            let _ = writeln!(build_file, "+libext+{}", exts.join("+"));
        }
        for define in &elaboration.defines {
            if define.value.is_empty() {
                let _ = writeln!(build_file, "+define+{}", define.name);
            } else {
                let _ = writeln!(build_file, "+define+{}={}", define.name, define.value);
            }
        }
        for library in &elaboration.library_files {
            let _ = writeln!(build_file, "-v {}", resolve(library));
        }
        let _ = writeln!(build_file, "--top {top}");
        for file in &files {
            let _ = writeln!(build_file, "{file}");
        }
        Self {
            workspace,
            build_file,
            files,
            top: top.to_owned(),
        }
    }

    /// Writes the build file into `dir`, named after the design identity.
    pub fn write_build_file(
        &self,
        dir: &Utf8Path,
        design_id: &str,
    ) -> std::io::Result<Utf8PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "surfer-{}.f",
            &design_id[..design_id.len().min(16)]
        ));
        std::fs::write(&path, &self.build_file)?;
        Ok(path)
    }
}

/// Removes `.` and `..` components without touching the filesystem.
pub fn normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            camino::Utf8Component::CurDir => {}
            camino::Utf8Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elaboration() -> vtr_vdb::Elaboration {
        serde_json::from_value(serde_json::json!({
            "work_dir": "/definitely/missing/work/dir",
            "language": "1800-2023",
            "files": ["rtl/top.sv", "/abs/lib.sv"],
            "library_files": ["cells.v"],
            "include_dirs": ["include"],
            "library_exts": ["", ".v", ".sv"],
            "defines": [{"name": "WIDTH", "value": "8"}, {"name": "SIM", "value": ""}],
        }))
        .unwrap()
    }

    #[test]
    fn missing_work_dir_falls_back_to_companion_and_resolves_relative_paths() {
        let plan = LaunchPlan::from_elaboration(&elaboration(), "top", Utf8Path::new("/vdb/dir"));
        assert_eq!(plan.workspace, Utf8PathBuf::from("/vdb/dir"));
        assert_eq!(
            plan.files,
            vec![
                Utf8PathBuf::from("/vdb/dir/rtl/top.sv"),
                Utf8PathBuf::from("/abs/lib.sv")
            ]
        );
        assert_eq!(
            plan.build_file,
            "+incdir+/vdb/dir/include\n-y /vdb/dir/include\n+libext+.v+.sv\n+define+WIDTH=8\n+define+SIM\n-v /vdb/dir/cells.v\n--top top\n/vdb/dir/rtl/top.sv\n/abs/lib.sv\n"
        );
    }

    #[test]
    fn relative_work_dir_is_resolved_from_the_companion() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        std::fs::create_dir_all(root.join("design/vdb")).unwrap();
        let mut record = elaboration();
        record.work_dir = "..".into();
        let plan = LaunchPlan::from_elaboration(&record, "top", &root.join("design/vdb"));
        assert_eq!(plan.workspace, root.join("design"));
        assert_eq!(plan.files[0], root.join("design/rtl/top.sv"));
        let written = plan
            .write_build_file(&root.join("out"), "abcdef0123456789ffff")
            .unwrap();
        assert_eq!(written.file_name(), Some("surfer-abcdef0123456789.f"));
        assert_eq!(std::fs::read_to_string(written).unwrap(), plan.build_file);
    }

    #[test]
    fn normalize_collapses_dot_components() {
        assert_eq!(
            normalize(Utf8Path::new("/a/b/../c/./d")),
            Utf8PathBuf::from("/a/c/d")
        );
        assert_eq!(normalize(Utf8Path::new("../x")), Utf8PathBuf::from("../x"));
    }
}
