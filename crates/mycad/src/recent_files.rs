//! Drawings the user opened or saved, newest first.
//!
//! Stored under its own key so Settings export does not carry local paths,
//! and so Settings → Apply cannot overwrite a list that changed while the
//! dialog was open.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const RECENT_FILES_KEY: &str = "mycad_recent_files";
pub const MAX_RECENT_FILES: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut files: Self = storage
            .and_then(|storage| eframe::get_value(storage, RECENT_FILES_KEY))
            .unwrap_or_default();
        files.normalize();
        files
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, RECENT_FILES_KEY, self);
    }

    /// Move `path` to the front, or insert it. Returns whether the list changed.
    pub fn touch(&mut self, path: &Path) -> bool {
        if path.as_os_str().is_empty() {
            return false;
        }
        if self
            .paths
            .first()
            .is_some_and(|existing| paths_equal(existing, path))
        {
            return false;
        }
        self.paths.retain(|existing| !paths_equal(existing, path));
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENT_FILES);
        true
    }

    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.paths.len();
        self.paths.retain(|existing| !paths_equal(existing, path));
        self.paths.len() != before
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.paths
    }

    /// File name, or `name - parent` when another entry has the same name.
    pub fn label(&self, index: usize) -> String {
        let Some(path) = self.paths.get(index) else {
            return String::new();
        };
        let name = file_label(path);
        let collides = self
            .paths
            .iter()
            .enumerate()
            .any(|(other, candidate)| other != index && names_equal(candidate, &name));
        if !collides {
            return name;
        }
        match path.parent().and_then(|parent| parent.file_name()) {
            Some(parent) if !parent.is_empty() => {
                format!("{name} - {}", parent.to_string_lossy())
            }
            _ => name,
        }
    }

    fn normalize(&mut self) {
        let mut kept: Vec<PathBuf> = Vec::new();
        for path in self.paths.drain(..) {
            if path.as_os_str().is_empty() {
                continue;
            }
            if kept.iter().any(|existing| paths_equal(existing, &path)) {
                continue;
            }
            kept.push(path);
            if kept.len() == MAX_RECENT_FILES {
                break;
            }
        }
        self.paths = kept;
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "drawing".into())
}

fn names_equal(path: &Path, name: &str) -> bool {
    let label = file_label(path);
    #[cfg(windows)]
    {
        label.eq_ignore_ascii_case(name)
    }
    #[cfg(not(windows))]
    {
        label == name
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_an_existing_path_to_the_front() {
        let mut files = RecentFiles::default();
        assert!(files.touch(Path::new("a/one.dwg")));
        assert!(files.touch(Path::new("b/two.dwg")));
        assert!(files.touch(Path::new("a/one.dwg")));
        assert!(!files.touch(Path::new("a/one.dwg")));
        assert_eq!(
            files.entries(),
            [
                PathBuf::from("a/one.dwg"),
                PathBuf::from("b/two.dwg"),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn touch_treats_windows_paths_as_case_insensitive() {
        let mut files = RecentFiles::default();
        files.touch(Path::new("C:/Drawings/Plan.dwg"));
        assert!(!files.touch(Path::new("c:/drawings/plan.dwg")));
        assert_eq!(files.entries().len(), 1);
        assert_eq!(
            files.entries()[0],
            PathBuf::from("C:/Drawings/Plan.dwg")
        );
    }

    #[test]
    fn the_list_keeps_the_ten_newest() {
        let mut files = RecentFiles::default();
        for index in 0..MAX_RECENT_FILES + 1 {
            files.touch(Path::new(&format!("drawings/{index}.dwg")));
        }
        assert_eq!(files.entries().len(), MAX_RECENT_FILES);
        assert_eq!(
            files.entries()[0],
            PathBuf::from(format!("drawings/{}.dwg", MAX_RECENT_FILES))
        );
        assert!(files
            .entries()
            .iter()
            .all(|path| path != Path::new("drawings/0.dwg")));
    }

    #[test]
    fn remove_and_clear_drop_entries() {
        let mut files = RecentFiles::default();
        files.touch(Path::new("a.dwg"));
        files.touch(Path::new("b.dwg"));
        assert!(files.remove(Path::new("a.dwg")));
        assert!(!files.remove(Path::new("a.dwg")));
        assert_eq!(files.entries(), [PathBuf::from("b.dwg")]);
        files.clear();
        assert!(files.entries().is_empty());
    }

    #[test]
    fn label_adds_the_parent_when_names_collide() {
        let mut files = RecentFiles::default();
        files.touch(Path::new("alpha/plan.dwg"));
        files.touch(Path::new("beta/plan.dwg"));
        files.touch(Path::new("only/unique.dwg"));
        assert_eq!(files.label(0), "unique.dwg");
        assert_eq!(files.label(1), "plan.dwg - beta");
        assert_eq!(files.label(2), "plan.dwg - alpha");
    }

    #[test]
    fn json_round_trip_keeps_the_order() {
        let mut files = RecentFiles::default();
        files.touch(Path::new("a/one.dwg"));
        files.touch(Path::new("b/two.dwg"));
        let text = serde_json::to_string(&files).expect("serialize");
        let loaded: RecentFiles = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(loaded.entries(), files.entries());
    }

    #[test]
    fn normalize_drops_empty_and_duplicate_paths() {
        let mut files = RecentFiles {
            paths: vec![
                PathBuf::new(),
                PathBuf::from("a/Plan.dwg"),
                PathBuf::from("a/Plan.dwg"),
                PathBuf::from("b/other.dwg"),
            ],
        };
        #[cfg(windows)]
        files.paths.insert(2, PathBuf::from("A/plan.dwg"));
        files.normalize();
        assert_eq!(files.entries()[0], PathBuf::from("a/Plan.dwg"));
        assert_eq!(*files.entries().last().unwrap(), PathBuf::from("b/other.dwg"));
        assert_eq!(files.entries().len(), 2);
    }
}
