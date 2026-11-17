// Адаптировано из Zed: crates/settings_ui/src/components/font_picker.rs (selection and current marker).
//! The catalog is prepared only while its dialog is alive. The shared Picker
//! owns search, keyboard navigation and virtualization; choosing is explicit.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::catalog;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::picker::{Choice, Picker};
use crate::ui::prefs;
use crate::ui::tip::Tip as _;

type Pick = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;
type Other = Rc<dyn Fn(&mut Window, &mut App)>;

pub(super) struct ConfigPicker {
    picker: Entity<Picker<catalog::ConfigFile>>,
    files: Vec<catalog::ConfigFile>,
    show_helpers: bool,
    problem: Option<String>,
    other: Other,
    _load: Task<()>,
}

impl ConfigPicker {
    pub(super) fn new(
        root: Option<PathBuf>,
        current: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        pick: impl Fn(PathBuf, &mut Window, &mut App) + 'static,
        other: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        let pick: Pick = Rc::new(pick);
        let picker = cx.new(|cx| {
            Picker::new(
                Vec::new(),
                "Поиск конфигов…".into(),
                "Загружаю список…".into(),
                window,
                cx,
                move |file: catalog::ConfigFile, window, cx| {
                    window.close_dialog(cx);
                    pick(file.path, window, cx);
                },
            )
            .render_label(|option, _, cx| {
                let file = &option.value;
                let caption = file.duplicate_name.then(|| {
                    if file.title.is_some() {
                        file.relative.clone()
                    } else {
                        std::path::Path::new(&file.relative)
                            .parent()
                            .map(|folder| folder.display().to_string())
                            .unwrap_or_default()
                    }
                });
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(faded(file.display_name().to_owned())),
                    )
                    .children(
                        caption
                            .filter(|caption| !caption.is_empty())
                            .map(|caption| {
                                div()
                                    .min_w_0()
                                    .max_w(prefs::scaled(160.))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(faded(caption))
                            }),
                    )
                    .when(option.selected, |row| {
                        row.child(Icon::new(IconName::Check).small())
                    })
                    .tip_text(file.path.display().to_string())
                    .into_any_element()
            })
        });
        let load = cx
            .background_executor()
            .spawn(async move { catalog::discover(root, current) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let catalog = load.await;
            this.update_in(cx, |this, window, cx| {
                this.files = catalog.files;
                this.problem = catalog.problem;
                this.refresh(window, cx);
                cx.notify();
            })
            .ok();
        });
        Self {
            picker,
            files: Vec::new(),
            show_helpers: false,
            problem: None,
            other: Rc::new(other),
            _load: task,
        }
    }

    fn refresh(&self, window: &mut Window, cx: &mut App) {
        let options = choices(&self.files, self.show_helpers);
        self.picker.update(cx, |picker, cx| {
            picker.set_options(
                options,
                "Поиск конфигов…".into(),
                "Конфиги не найдены".into(),
                window,
                cx,
            )
        });
    }

    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).search_focus(cx)
    }
}

impl Render for ConfigPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.picker.clone())
            .when(self.files.iter().any(|file| !file.entrypoint), |body| {
                body.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            kit::switch("config-show-helpers")
                                .checked(self.show_helpers)
                                .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                    this.show_helpers = *checked;
                                    this.refresh(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(div().text_sm().child("Вспомогательные конфиги")),
                )
            })
            .when_some(self.problem.clone(), |body, problem| {
                body.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().warning)
                        .child(problem),
                )
            })
            .child(
                kit::button("config-choose-other-file")
                    .small()
                    .faded_label("Другой файл…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let other = this.other.clone();
                        window.close_dialog(cx);
                        other(window, cx);
                    })),
            )
    }
}

fn choices(files: &[catalog::ConfigFile], show_helpers: bool) -> Vec<Choice<catalog::ConfigFile>> {
    files
        .iter()
        .filter(|file| file.entrypoint || file.current || show_helpers)
        .map(|file| Choice {
            // Search still finds the source filename/path when the row has a
            // friendly title. No path is painted into its primary caption.
            label: format!("{} {} {}", file.display_name(), file.name, file.relative).into(),
            value: file.clone(),
            selected: file.current,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn main_choices_keep_current_helpers_and_toggle_reveals_other_helpers() {
        let file = |name: &str, entrypoint, current| catalog::ConfigFile {
            path: PathBuf::from(name),
            name: name.into(),
            relative: name.into(),
            current,
            title: None,
            entrypoint,
            duplicate_name: false,
        };
        let files = [
            file("main.toml", true, false),
            file("child.toml", false, false),
            file("current-helper.toml", false, true),
        ];
        let primary = choices(&files, false);
        assert_eq!(
            primary
                .iter()
                .map(|choice| choice.value.name.as_str())
                .collect::<Vec<_>>(),
            ["main.toml", "current-helper.toml"]
        );
        assert!(primary[1].selected);
        assert_eq!(choices(&files, true).len(), 3);
    }
}
