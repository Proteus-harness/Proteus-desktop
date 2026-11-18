//! Project items of the top bar menus: recent folders and the folder
//! dialog, routed to whoever owns the window content.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use crate::ui::prefs;

/// Recent folders listed in a menu; the picker screen shows all of them.
const RECENT_SHOWN: usize = 8;

/// The workspace asks the window to show another folder in its place.
pub(in crate::ui) enum WorkspaceRequest {
    Open(PathBuf),
    ChooseFolder,
    OpenConfig,
}

type OpenFolder = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;
type ChooseFolder = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub(in crate::ui) struct Projects {
    current: Option<PathBuf>,
    open: OpenFolder,
    choose: ChooseFolder,
}

impl Projects {
    pub(in crate::ui) fn new(
        current: Option<PathBuf>,
        open: impl Fn(PathBuf, &mut Window, &mut App) + 'static,
        choose: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            current,
            open: Rc::new(open),
            choose: Rc::new(choose),
        }
    }

    /// Recent folders other than the open one, then the folder dialog.
    pub(in crate::ui) fn items(&self, mut menu: PopupMenu, cx: &App) -> PopupMenu {
        let recent: Vec<&Path> = prefs::current(cx)
            .recent
            .iter()
            .filter(|path| Some(*path) != self.current.as_deref())
            .take(RECENT_SHOWN)
            .collect();
        if !recent.is_empty() {
            menu = menu.label("Недавние проекты");
        }
        let labels = recent_labels(&recent);
        for (path, label) in recent.into_iter().zip(labels) {
            let path = path.to_path_buf();
            let open = self.open.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .icon(Icon::new(Lucide::Folder))
                    .disabled(!path.is_dir())
                    .on_click(move |_, window, cx| open(path.clone(), window, cx)),
            );
        }
        let choose = self.choose.clone();
        menu.item(
            PopupMenuItem::new("Открыть папку…")
                .icon(Icon::new(Lucide::FolderOpen))
                .on_click(move |_, window, cx| choose(window, cx)),
        )
    }
}

pub(in crate::ui) fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Use the shortest parent suffix that separates each namesake from its peers.
/// Only compare the rows in this menu; callbacks still receive their original paths.
fn recent_labels(paths: &[&Path]) -> Vec<String> {
    let names: Vec<_> = paths.iter().map(|path| folder_name(path)).collect();
    let mut labels = names.clone();
    for (i, &path) in paths.iter().enumerate() {
        let mut peers = paths
            .iter()
            .enumerate()
            .filter_map(|(j, &other)| (other != path && names[i] == names[j]).then_some(other))
            .peekable();
        if peers.peek().is_none() {
            continue;
        }
        let suffix = path
            .ancestors()
            .skip(2)
            .filter_map(|base| path.strip_prefix(base).ok())
            .find(|suffix| !peers.clone().any(|other| other.ends_with(suffix)))
            .unwrap_or(path);
        labels[i] = suffix
            .to_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{suffix:?}"));
    }
    // A quoted non-UTF-8 spelling can also be someone else's literal filename.
    // Keep the literal name; distinguish only a changed caption when necessary.
    for i in 0..labels.len() {
        if labels[i] != names[i]
            && labels
                .iter()
                .enumerate()
                .any(|(j, label)| paths[i] != paths[j] && label == &labels[i])
        {
            let mut label = format!("{}. {}", i + 1, labels[i]);
            while labels.contains(&label) {
                label.insert(0, '#');
            }
            labels[i] = label;
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn recent_names_keep_only_the_distinguishing_directories() {
        let paths = [
            Path::new("/work/team/a/shared/app"),
            Path::new("/work/team/b/shared/app"),
            Path::new("/other/app"),
            Path::new("/work/notes"),
        ];
        assert_eq!(
            recent_labels(&paths),
            ["a/shared/app", "b/shared/app", "other/app", "notes"]
        );
    }

    #[cfg(unix)]
    #[::core::prelude::v1::test]
    fn literal_name_stays_distinct_from_escaped_native_name() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let paths = [
            Path::new(OsStr::from_bytes(b"app\xFF")),
            Path::new(OsStr::from_bytes(b"app\xFE")),
            Path::new(r#""app\xFF""#),
        ];
        let labels = recent_labels(&paths);
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[0], labels[2]);
        assert_eq!(labels[2], r#""app\xFF""#);
    }
}
