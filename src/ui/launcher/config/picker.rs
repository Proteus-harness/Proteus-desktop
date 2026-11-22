// Адаптировано из Zed: crates/settings_ui/src/components/font_picker.rs (selection and current marker).
//! The catalog is prepared only while its popover is open. The shared Picker
//! owns search, keyboard navigation and virtualization; choosing is explicit.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::Popover;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;

use super::catalog;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::picker::{Choice, Picker};
use crate::ui::prefs;
use crate::ui::settings::configurations::OpenAgent;
use crate::ui::tip::Tip as _;

pub(in crate::ui) fn config_selector(
    current: Option<PathBuf>,
    pick: OpenAgent,
    cx: &App,
) -> Popover {
    Popover::new("config-choose")
        .anchor(Anchor::TopRight)
        .offset(px(4.))
        .trigger(
            kit::button("config-open-agent")
                .settings_control(cx)
                .small()
                .icon(Lucide::ChevronDown)
                .faded_label("Сменить…"),
        )
        .content(move |_, window, cx| {
            let popup = cx.weak_entity();
            let picker = window.use_keyed_state("config-choices", cx, |window, cx| {
                let owner = popup.clone();
                let picker = ConfigPicker::new(current, window, cx, move |path, window, cx| {
                    let _ = owner.update(cx, |popup, cx| popup.dismiss(window, cx));
                    window.refresh();
                    pick(path, window, cx);
                });
                let focus = picker.picker.read(cx).search_focus(cx);
                window.defer(cx, move |window, cx| {
                    if popup
                        .read_with(cx, |popup, _| popup.is_open())
                        .unwrap_or(false)
                    {
                        focus.focus(window, cx);
                    }
                });
                picker
            });
            let width = prefs::scaled(440.)
                .to_pixels(window.rem_size())
                .min((window.viewport_size().width - px(16.)).max(px(0.)));
            kit::popup_surface(cx).w(width).p_2().child(picker)
        })
}

struct ConfigPicker {
    picker: Entity<Picker<catalog::ConfigFile>>,
    pick: OpenAgent,
    problem: Option<String>,
    _load: Task<()>,
}

impl ConfigPicker {
    fn new(
        current: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        pick: impl Fn(Option<PathBuf>, &mut Window, &mut App) + 'static,
    ) -> Self {
        let pick: OpenAgent = std::rc::Rc::new(pick);
        let select = pick.clone();
        let picker = cx.new(|cx| {
            Picker::new(
                Vec::new(),
                "".into(),
                "Загружаю список…".into(),
                window,
                cx,
                move |file: catalog::ConfigFile, window, cx| {
                    select(Some(file.path), window, cx);
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
                    .tip_text(file.path.display().to_string())
                    .into_any_element()
            })
            .search_label("Поиск конфигов")
        });
        let root = catalog::config_home();
        let load = cx
            .background_executor()
            .spawn(async move { catalog::discover(root, current) });
        let task = cx.spawn_in(window, async move |this, cx| {
            let catalog = load.await;
            this.update_in(cx, |this, window, cx| {
                this.problem = catalog.problem;
                this.picker.update(cx, |picker, cx| {
                    picker.set_options(
                        choices(&catalog.files),
                        "".into(),
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
            pick,
            problem: None,
            _load: task,
        }
    }
}

impl Render for ConfigPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
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
                        (this.pick)(None, window, cx);
                    })),
            )
    }
}

fn choices(files: &[catalog::ConfigFile]) -> Vec<Choice<catalog::ConfigFile>> {
    files
        .iter()
        .filter(|file| file.entrypoint)
        .chain(files.iter().filter(|file| !file.entrypoint))
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
    fn helper_configs_follow_main_choices() {
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
            file("child.toml", false, false),
            file("current-helper.toml", false, true),
            file("main.toml", true, false),
        ];
        let primary = choices(&files);
        assert_eq!(
            primary
                .iter()
                .map(|choice| choice.value.name.as_str())
                .collect::<Vec<_>>(),
            ["main.toml", "child.toml", "current-helper.toml"]
        );
        assert!(primary[2].selected);
    }
}
