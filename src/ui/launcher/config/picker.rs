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
use crate::ui::tip::Tip as _;

type Pick = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;
type Other = Rc<dyn Fn(&mut Window, &mut App)>;

pub(super) struct ConfigPicker {
    picker: Entity<Picker<PathBuf>>,
    root: String,
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
        let root_label = root.as_ref().map_or_else(
            || "Папка конфигов не определена".into(),
            |path| path.display().to_string(),
        );
        let pick: Pick = Rc::new(pick);
        let picker = cx.new(|cx| {
            Picker::new(
                Vec::new(),
                "Поиск конфигов…".into(),
                "Загружаю список…".into(),
                window,
                cx,
                move |path, window, cx| {
                    window.close_dialog(cx);
                    pick(path, window, cx);
                },
            )
            .render_label(|option, _, _| {
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(faded(option.label.clone())))
                    .when(option.selected, |row| {
                        row.child(Icon::new(IconName::Check).small())
                    })
                    .tip_text(option.value.display().to_string())
                    .into_any_element()
            })
        });
        let load = cx
            .background_executor()
            .spawn(async move { catalog::discover(root, current) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let catalog = load.await;
            this.update_in(cx, |this, window, cx| {
                let options = catalog
                    .files
                    .into_iter()
                    .map(|file| {
                        let folder = std::path::Path::new(&file.relative)
                            .parent()
                            .filter(|folder| !folder.as_os_str().is_empty());
                        let label = folder.map_or(file.name.clone(), |folder| {
                            format!("{} · {}", file.name, folder.display())
                        });
                        Choice {
                            label: label.into(),
                            value: file.path,
                            selected: file.current,
                        }
                    })
                    .collect();
                this.problem = catalog.problem;
                this.picker.update(cx, |picker, cx| {
                    picker.set_options(
                        options,
                        "Поиск конфигов…".into(),
                        "Конфиги не найдены".into(),
                        window,
                        cx,
                    )
                });
                cx.notify();
            })
            .ok();
        });
        Self {
            picker,
            root: root_label,
            problem: None,
            other: Rc::new(other),
            _load: task,
        }
    }

    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.read(cx).search_focus(cx)
    }
}

impl Render for ConfigPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .tip_text(self.root.clone())
                    .child(faded(self.root.clone())),
            )
            .child(self.picker.clone())
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
