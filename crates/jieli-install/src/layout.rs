use std::path::{Path, PathBuf};

use crate::environment::Environment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    pub source: PathBuf,
    pub target: PathBuf,
    pub mode: u32,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub home: PathBuf,
    pub manifest: PathBuf,
    pub backups: PathBuf,
    files: Vec<FileSpec>,
}

impl Layout {
    pub fn new(environment: &Environment, plugin: Option<&Path>) -> Result<Self, String> {
        let plugin = plugin
            .map(Path::to_path_buf)
            .unwrap_or_else(|| environment.root.join("build/plugin/libgstpipewire.so"));
        let plugin = if plugin.is_absolute() {
            plugin
        } else {
            environment.root.join(plugin)
        };
        let files = vec![
            FileSpec {
                source: environment.root.join("target/release/jieli-bridge"),
                target: environment.home.join(".local/libexec/jieli-bridge"),
                mode: 0o755,
            },
            FileSpec {
                source: environment.root.join("target/release/jieli-publisher"),
                target: environment.home.join(".local/libexec/jieli-publisher"),
                mode: 0o755,
            },
            FileSpec {
                source: environment
                    .root
                    .join("config/jieli-camera-bridge.rust.service"),
                target: environment
                    .config
                    .join("systemd/user/jieli-camera-bridge.service"),
                mode: 0o644,
            },
            FileSpec {
                source: environment.root.join("config/90-jieli-bridge.conf"),
                target: environment
                    .config
                    .join("wireplumber/wireplumber.conf.d/90-jieli-bridge.conf"),
                mode: 0o644,
            },
            FileSpec {
                source: plugin,
                target: environment
                    .home
                    .join(".local/lib/jieli-gstreamer/libgstpipewire.so"),
                mode: 0o755,
            },
        ];
        Ok(Self {
            home: environment.home.clone(),
            manifest: environment.state.join("jieli-camera/installed-files.json"),
            backups: environment.state.join("jieli-camera/backups"),
            files,
        })
    }

    pub fn files(&self) -> &[FileSpec] {
        &self.files
    }

    pub fn target_set(&self) -> std::collections::BTreeSet<String> {
        self.files
            .iter()
            .map(|file| file.target.to_string_lossy().into_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(root: &Path, home: &Path) -> Environment {
        Environment {
            home: home.to_path_buf(),
            config: home.join(".config"),
            state: home.join(".local/state"),
            root: root.to_path_buf(),
        }
    }

    #[test]
    fn resolves_default_and_relative_plugin_paths() {
        let directory = tempfile::tempdir().unwrap();
        let layout = Layout::new(
            &environment(directory.path(), &directory.path().join("home")),
            None,
        )
        .unwrap();
        assert_eq!(
            layout.files().last().unwrap().source,
            directory.path().join("build/plugin/libgstpipewire.so")
        );

        let relative = Layout::new(
            &environment(directory.path(), &directory.path().join("home")),
            Some(Path::new("build/other.so")),
        )
        .unwrap();
        assert_eq!(
            relative.files().last().unwrap().source,
            directory.path().join("build/other.so")
        );
    }

    #[test]
    fn target_set_contains_all_managed_paths() {
        let directory = tempfile::tempdir().unwrap();
        let layout = Layout::new(
            &environment(directory.path(), &directory.path().join("home")),
            None,
        )
        .unwrap();
        assert_eq!(layout.target_set().len(), 5);
    }
}
