use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

#[derive(Clone)]
pub enum Target {
    /// Without a profile the server loads its default config.
    Spawn {
        profile: Option<String>,
    },
    Attach {
        origin: String,
        token: String,
    },
}

pub struct LaunchOptions {
    pub workspace: PathBuf,
    pub target: Target,
}

/// Command line: an unset workspace is resolved from recent folders unless
/// the picker was explicitly requested.
pub struct Launch {
    pub workspace: Option<PathBuf>,
    pub target: Target,
    restore_recent: bool,
}

impl Launch {
    pub fn from_args() -> Result<Self> {
        Self::parse(std::env::args().skip(1))
    }

    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut workspace = None;
        let mut choose = false;
        let mut profile = None;
        let mut origin = None;
        let mut token = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .with_context(|| format!("{name} requires a value"))
            };
            match arg.as_str() {
                "--config" => profile = Some(value("--config")?),
                "--origin" => origin = Some(value("--origin")?),
                "--token" => token = Some(value("--token")?),
                "--choose-workspace" => choose = true,
                flag if flag.starts_with("--") => bail!("unknown option {flag}"),
                _ if workspace.is_none() => workspace = Some(PathBuf::from(arg)),
                _ => bail!("unexpected argument {arg}"),
            }
        }

        let workspace = match (workspace, choose) {
            (Some(_), true) => bail!("--choose-workspace cannot be combined with a workspace"),
            (_, true) => None,
            (Some(path), false) => Some(path),
            (None, false) => None,
        };
        let restore_recent = workspace.is_none() && !choose;
        let workspace = workspace
            .map(|path| {
                path.canonicalize()
                    .with_context(|| format!("workspace {}", path.display()))
            })
            .transpose()?;

        let target = match (origin, token) {
            (Some(origin), Some(token)) => {
                if profile.is_some() {
                    bail!("--config cannot be combined with --origin");
                }
                Target::Attach { origin, token }
            }
            (None, None) => Target::Spawn {
                profile: profile.map(anchor_profile),
            },
            _ => bail!("--origin and --token must be given together"),
        };

        Ok(Self {
            workspace,
            target,
            restore_recent,
        })
    }

    /// Restore the server that owns the saved layout before considering recent
    /// project folders: one server can have chats from several folders open.
    /// A forced picker and an explicit workspace always keep their meaning.
    pub fn restore_workspace(&mut self, preferences: &crate::preferences::Preferences) {
        if self.restore_recent {
            self.workspace = preferences
                .tabs_workspace
                .as_ref()
                .filter(|saved| {
                    saved.workspace.is_dir()
                        && crate::preferences::TabWorkspace::new(&saved.workspace, &self.target)
                            .as_ref()
                            == Some(*saved)
                })
                .map(|saved| saved.workspace.clone())
                .or_else(|| {
                    preferences
                        .recent
                        .iter()
                        .find_map(|path| path.is_dir().then(|| path.canonicalize().ok()).flatten())
                });
        }
    }
}

/// A relative config file path keeps naming the same file when the server
/// is started in another folder; bare profile names stay as given.
fn anchor_profile(profile: String) -> String {
    let path = Path::new(&profile);
    if path.is_relative()
        && path.components().count() > 1
        && let Ok(path) = path.canonicalize()
    {
        return path.display().to_string();
    }
    profile
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{Preferences, TabWorkspace};

    fn parse(args: &[&str]) -> Launch {
        Launch::parse(args.iter().map(|arg| (*arg).to_owned())).unwrap()
    }

    #[test]
    fn ordinary_launch_restores_first_existing_recent_not_current_directory() {
        let existing = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let mut prefs = Preferences::default();
        prefs.recent.touch(&existing);
        prefs
            .recent
            .touch(&existing.join("missing-startup-folder-fixture"));
        let mut launch = parse(&[]);
        assert!(launch.workspace.is_none());
        launch.restore_workspace(&prefs);
        assert_eq!(launch.workspace, Some(existing));
    }

    #[test]
    fn no_recent_folder_and_forced_picker_stay_unresolved() {
        let mut launch = parse(&[]);
        launch.restore_workspace(&Preferences::default());
        assert!(launch.workspace.is_none());
        let mut prefs = Preferences::default();
        prefs.recent.touch(Path::new(env!("CARGO_MANIFEST_DIR")));
        prefs.tabs_workspace =
            TabWorkspace::new(Path::new(env!("CARGO_MANIFEST_DIR")), &launch.target);
        let mut picker = parse(&["--choose-workspace"]);
        picker.restore_workspace(&prefs);
        assert!(picker.workspace.is_none());
    }

    #[test]
    fn explicit_workspace_wins_and_config_is_anchored_before_restore() {
        let explicit = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let mut prefs = Preferences::default();
        prefs.recent.touch(explicit.parent().unwrap());
        let mut launch = parse(&[explicit.to_str().unwrap(), "--config", "./Cargo.toml"]);
        prefs.tabs_workspace = TabWorkspace::new(explicit.parent().unwrap(), &launch.target);
        launch.restore_workspace(&prefs);
        assert_eq!(launch.workspace, Some(explicit));
        let Target::Spawn { profile } = launch.target else {
            panic!("spawn expected")
        };
        assert_eq!(
            profile,
            Some(
                Path::new("./Cargo.toml")
                    .canonicalize()
                    .unwrap()
                    .display()
                    .to_string()
            )
        );
        assert!(Launch::parse(["--choose-workspace".to_owned(), ".".to_owned()]).is_err());
    }

    #[test]
    fn layout_owner_wins_when_a_chat_from_another_project_was_opened() {
        let owner = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let other_project = owner.parent().unwrap().to_path_buf();
        let mut launch = parse(&["--config", "fixture"]);
        let mut prefs = Preferences::default();
        prefs.recent.touch(&owner);
        prefs.recent.touch(&other_project);
        prefs.tabs_workspace = TabWorkspace::new(&owner, &launch.target);
        prefs.tabs.open("chat:project-b-session", |_| false);
        launch.restore_workspace(&prefs);
        assert_eq!(launch.workspace, Some(owner.clone()));
        assert!(
            prefs
                .tabs_for(&owner, &launch.target)
                .contains("chat:project-b-session")
        );

        let mut other_profile = parse(&["--config", "other-profile"]);
        other_profile.restore_workspace(&prefs);
        assert_eq!(other_profile.workspace, Some(other_project.clone()));
        assert!(
            !prefs
                .tabs_for(&other_project, &other_profile.target)
                .contains("chat:project-b-session")
        );

        prefs.tabs_workspace.as_mut().unwrap().workspace =
            owner.join("missing-startup-folder-fixture");
        let mut unavailable_owner = parse(&["--config", "fixture"]);
        unavailable_owner.restore_workspace(&prefs);
        assert_eq!(unavailable_owner.workspace, Some(other_project));
    }

    #[test]
    fn explicit_connection_arguments_remain_authoritative() {
        let launch = parse(&["--origin", "http://localhost:9", "--token", "synthetic"]);
        assert!(matches!(launch.target, Target::Attach { .. }));
        assert!(Launch::parse(["--origin".to_owned(), "http://localhost:9".to_owned()]).is_err());
        assert!(
            Launch::parse(
                [
                    "--origin",
                    "http://localhost:9",
                    "--token",
                    "synthetic",
                    "--config",
                    "profile"
                ]
                .map(str::to_owned)
            )
            .is_err()
        );
    }
}
