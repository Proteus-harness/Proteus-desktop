//! One native window owns its appearance, selection intent and write queue.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{ClientPreferences, Effects, persistence};
use crate::visual_config::{self, SelectionOrigin, VisualConfig, WindowSelection};
use gpui_kit::*;

#[derive(Clone)]
pub(in crate::ui) struct WindowVisualPreferences {
    pub value: Arc<VisualConfig>,
    pub selection: WindowSelection,
    pub agent_path: Option<PathBuf>,
    pub revision: u64,
    pub problem: Option<String>,
    initialized: bool,
    source_generation: u64,
    write_revision: u64,
    chat_font: Option<SharedString>,
    writer: Option<Rc<persistence::Writer<VisualWrite>>>,
}
impl Global for WindowVisualPreferences {}

/// Tokens identify visual state across native windows and moved surfaces.
/// This is identity metadata only, never a process-global visual preference.
fn fresh_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

struct VisualWrite {
    path: PathBuf,
    value: VisualConfig,
    agent: Option<PathBuf>,
}

/// Shutdown-only ownership survives closed windows and replaced file sources.
/// This global never stores preferences, selections, or native window state.
#[derive(Default)]
struct VisualWriters(Vec<persistence::Shutdown<VisualWrite>>);
impl Global for VisualWriters {}

pub(super) fn init(cx: &mut App) {
    let value = VisualConfig::from_preferences(super::current(cx));
    let chat_font = super::super::theme::resolve(value.fonts.chat.as_deref(), cx);
    cx.set_global(WindowVisualPreferences {
        value: Arc::new(value),
        selection: WindowSelection::default(),
        agent_path: None,
        revision: 0,
        problem: None,
        initialized: false,
        source_generation: 0,
        write_revision: 0,
        chat_font,
        writer: None,
    });
    cx.set_global(VisualWriters::default());
    cx.register_window_global::<WindowVisualPreferences>();
    super::super::extensions::register_visual_globals(cx);
    super::appearance::register(cx);
    super::super::motion::register_window_globals(cx);
    if cx.has_global::<gpui_kit::component::Theme>() {
        cx.register_window_global::<gpui_kit::component::Theme>();
    }
    if cx.has_global::<gpui_kit::base::Theme>() {
        cx.register_window_global::<gpui_kit::base::Theme>();
    }
    if cx.has_global::<gpui_kit::base::TextViewDefaults>() {
        cx.register_window_global::<gpui_kit::base::TextViewDefaults>();
    }
    cx.on_app_quit(|cx| {
        let pending = std::mem::take(&mut cx.global_mut::<VisualWriters>().0);
        for shutdown in &pending {
            shutdown.begin();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        for shutdown in pending {
            shutdown.wait(deadline.saturating_duration_since(std::time::Instant::now()));
        }
        async {}
    })
    .detach();
}

pub(super) fn update_application_default(value: &crate::preferences::Preferences, cx: &mut App) {
    let value = Arc::new(VisualConfig::from_preferences(value));
    let chat_font = super::super::theme::resolve(value.fonts.chat.as_deref(), cx);
    let state = cx.global_mut::<WindowVisualPreferences>();
    state.value = value;
    state.chat_font = chat_font;
    state.revision = fresh_revision();
}

pub(in crate::ui) fn current(cx: &App) -> &VisualConfig {
    &cx.global::<WindowVisualPreferences>().value
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct VisualIntent {
    source: u64,
    revision: u64,
}

pub(in crate::ui) fn intent(cx: &App) -> VisualIntent {
    let state = cx.global::<WindowVisualPreferences>();
    VisualIntent {
        source: state.source_generation,
        revision: state.revision,
    }
}

pub(in crate::ui) fn select_if_current(
    expected: VisualIntent,
    path: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if intent(cx) != expected {
        return false;
    }
    select(path, window, cx);
    true
}

pub(in crate::ui) fn revision(cx: &App) -> u64 {
    cx.global::<WindowVisualPreferences>().revision
}

pub(in crate::ui) fn chat_font(cx: &App) -> Option<SharedString> {
    cx.global::<WindowVisualPreferences>().chat_font.clone()
}

pub(in crate::ui) fn snapshot(cx: &App) -> WindowVisualPreferences {
    let mut snapshot = cx.global::<WindowVisualPreferences>().clone();
    snapshot.writer = None;
    snapshot
}

/// Detached windows start with the source's values, then evolve independently.
pub(in crate::ui) fn inherit(mut snapshot: WindowVisualPreferences, cx: &mut App) {
    snapshot.writer = None;
    snapshot.initialized = true;
    snapshot.revision = fresh_revision();
    snapshot.source_generation = snapshot.source_generation.wrapping_add(1);
    cx.set_global(snapshot);
    apply(cx);
}

fn registry(cx: &App) -> crate::visual_config::Registry {
    let mut registry = super::current(cx).visual_configs.clone();
    // Headless in-memory contexts must not inspect the user's default file.
    if !cx.global::<ClientPreferences>().persistent {
        return registry;
    }
    if registry.default_config.is_none() {
        if let Some(path) =
            crate::preferences::config_dir().map(|root| root.join("desktop/default.json"))
        {
            if path.is_file() {
                registry.default_config = Some(path);
            }
        }
    }
    registry
}

pub(in crate::ui) fn initialize(window: &mut Window, cx: &mut App) {
    if cx.global::<WindowVisualPreferences>().initialized {
        return;
    }
    {
        let state = cx.global_mut::<WindowVisualPreferences>();
        state.initialized = true;
        state.revision = fresh_revision();
    }
    apply(cx);
    let registry = registry(cx);
    match registry.resolve(None) {
        Ok(selection) => load_selection(selection, window, cx),
        Err(error) => report(format!("Визуальный конфиг: {error:#}"), cx),
    }
}

pub(in crate::ui) fn set_agent(path: &Path, window: &mut Window, cx: &mut App) {
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => return,
    };
    let current = cx.global::<WindowVisualPreferences>();
    if current.agent_path.as_ref() == Some(&path) {
        return;
    }
    let mut selection = current.selection.clone();
    let registry = registry(cx);
    cx.global_mut::<WindowVisualPreferences>().agent_path = Some(path.clone());
    match selection.apply_agent_binding(&registry, Some(&path)) {
        Ok(true) => load_selection(selection, window, cx),
        Ok(false) => {}
        Err(error) => report(format!("Привязка визуального конфига: {error:#}"), cx),
    }
}

pub(in crate::ui) fn reset_auto(window: &mut Window, cx: &mut App) {
    let agent = cx.global::<WindowVisualPreferences>().agent_path.clone();
    match registry(cx).resolve(agent.as_deref()) {
        Ok(selection) => load_selection(selection, window, cx),
        Err(error) => report(format!("Визуальная привязка не применена: {error:#}"), cx),
    }
}

pub(in crate::ui) fn reset_default(window: &mut Window, cx: &mut App) {
    let selection = WindowSelection {
        path: registry(cx).default_config,
        origin: SelectionOrigin::Manual,
    };
    load_selection(selection, window, cx);
}

pub(in crate::ui) fn select(path: PathBuf, window: &mut Window, cx: &mut App) {
    load_selection(
        WindowSelection {
            path: Some(path),
            origin: SelectionOrigin::Manual,
        },
        window,
        cx,
    );
}

fn load_selection(selection: WindowSelection, window: &mut Window, cx: &mut App) {
    let generation = {
        let state = cx.global_mut::<WindowVisualPreferences>();
        if selection.origin == SelectionOrigin::Manual {
            state.selection.origin = SelectionOrigin::Manual;
        }
        state.source_generation = state.source_generation.wrapping_add(1);
        // The user's new source intent immediately invalidates commands that
        // were issued by the previous source, even while file I/O is pending.
        state.revision = fresh_revision();
        state.source_generation
    };
    let agent = cx.global::<WindowVisualPreferences>().agent_path.clone();
    let builtin = VisualConfig::from_preferences(super::current(cx));
    let handle = window.window_handle();
    let load = cx.background_executor().spawn(async move {
        let value = match &selection.path {
            Some(path) => visual_config::load(path, agent.as_deref())?,
            None => builtin,
        };
        anyhow::Ok((selection, value))
    });
    cx.spawn(async move |cx| {
        let loaded = load.await;
        let _ = handle.update(cx, |_, window, cx| {
            if cx.global::<WindowVisualPreferences>().source_generation != generation {
                return;
            }
            match loaded {
                Ok((selection, value)) => install(selection, value, window, cx),
                Err(error) => report(format!("Визуальный конфиг не применён: {error:#}"), cx),
            }
        });
    })
    .detach();
}

fn install(selection: WindowSelection, value: VisualConfig, _window: &mut Window, cx: &mut App) {
    let state = cx.global_mut::<WindowVisualPreferences>();
    state.selection = selection;
    state.value = Arc::new(value);
    state.revision = fresh_revision();
    state.problem = None;
    state.writer = None;
    apply(cx);
}

pub(in crate::ui) fn update(cx: &mut App, change: impl FnOnce(&mut VisualConfig)) {
    let previous = cx.global::<WindowVisualPreferences>().value.clone();
    let mut value = (*previous).clone();
    change(&mut value);
    if value == *previous {
        return;
    }
    if let Err(error) = value.validate() {
        return report(format!("Визуальный конфиг: {error:#}"), cx);
    }
    let state = cx.global_mut::<WindowVisualPreferences>();
    state.value = Arc::new(value);
    state.revision = fresh_revision();
    state.selection.origin = SelectionOrigin::Manual;
    state.source_generation = state.source_generation.wrapping_add(1);
    apply(cx);
    persist(cx);
}

fn apply(cx: &mut App) {
    let value = current(cx).clone();
    let mut projection = crate::preferences::Preferences::default();
    value.apply_to_preferences(&mut projection);
    let chat_font = super::super::theme::resolve(value.fonts.chat.as_deref(), cx);
    cx.global_mut::<WindowVisualPreferences>().chat_font = chat_font;
    super::super::extensions::bump_visual_render_generation(cx);
    super::apply(
        &projection,
        Effects {
            shortcuts: false,
            renderers: false,
            ..Effects::initial()
        },
        cx,
    );
    // Notify dependent chats/renderers only with their explicit owner window.
}

fn persist(cx: &mut App) {
    if !cx.global::<ClientPreferences>().persistent {
        return;
    }
    let state = cx.global::<WindowVisualPreferences>();
    let promote = state.selection.path.is_none();
    let path =
        match state.selection.path.clone().or_else(|| {
            crate::preferences::config_dir().map(|path| path.join("desktop/default.json"))
        }) {
            Some(path) => path,
            None => return report("Папка визуальных конфигов недоступна".into(), cx),
        };
    let handle = match cx.current_execution_window_id().and_then(|id| {
        cx.windows()
            .into_iter()
            .find(|handle| handle.window_id() == id)
    }) {
        Some(handle) => handle,
        None => {
            return report(
                "Нет владельца окна для сохранения визуального конфига".into(),
                cx,
            );
        }
    };
    if state.writer.is_none() {
        let created = persistence::Writer::new(|write: &VisualWrite| {
            if let Some(parent) = write.path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            visual_config::save(&write.path, &write.value, write.agent.as_deref())
                .map(|_| ())
                .map_err(|error| format!("{error:#}"))
        });
        let (writer, receiver) = match created {
            Ok(pair) => pair,
            Err(error) => {
                return report(
                    format!("Не удалось сохранять визуальный конфиг: {error}"),
                    cx,
                );
            }
        };
        let shutdown = writer.shutdown_handle();
        let registry = cx.global_mut::<VisualWriters>();
        registry.0.retain(|pending| !pending.is_finished());
        registry.0.push(shutdown);
        let writer = Rc::new(writer);
        let identity = Rc::as_ptr(&writer) as usize;
        cx.global_mut::<WindowVisualPreferences>().writer = Some(writer);
        let promoted_path = path.clone();
        cx.spawn(async move |cx| {
            while receiver.recv().await.is_ok() {
                if handle
                    .update(cx, |_, _, cx| {
                        let state = cx.global::<WindowVisualPreferences>();
                        if !state
                            .writer
                            .as_ref()
                            .is_some_and(|writer| Rc::as_ptr(writer) as usize == identity)
                        {
                            return;
                        }
                        let Some(completion) =
                            state.writer.as_ref().and_then(|writer| writer.completion())
                        else {
                            return;
                        };
                        if completion.result.is_ok() && promote {
                            cx.global_mut::<WindowVisualPreferences>().selection.path =
                                Some(promoted_path.clone());
                            super::update(cx, |prefs| {
                                if prefs.visual_configs.default_config.is_none() {
                                    prefs.visual_configs.default_config =
                                        Some(promoted_path.clone());
                                }
                            });
                        }
                        if let Some(problem) = completion
                            .problem_for(cx.global::<WindowVisualPreferences>().write_revision)
                        {
                            cx.global_mut::<WindowVisualPreferences>().problem = problem;
                            cx.refresh_windows();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    let state = cx.global::<WindowVisualPreferences>();
    let write = Arc::new(VisualWrite {
        path,
        value: (*state.value).clone(),
        agent: state.agent_path.clone(),
    });
    match state.writer.as_ref().unwrap().submit(write) {
        Ok(revision) => cx.global_mut::<WindowVisualPreferences>().write_revision = revision,
        Err(error) => report(error.into(), cx),
    }
}

pub(in crate::ui) fn report(message: String, cx: &mut App) {
    cx.global_mut::<WindowVisualPreferences>().problem = Some(message);
    cx.refresh_windows();
}

pub(in crate::ui) fn package_update(
    package: &str,
    expected_revision: u64,
    values: serde_json::Map<String, serde_json::Value>,
    cx: &mut App,
) -> Result<u64, String> {
    if revision(cx) != expected_revision {
        return Err("Визуальная конфигурация уже изменилась".into());
    }
    if package.is_empty()
        || !package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("Некорректный пакет визуальных настроек".into());
    }
    let mut next = current(cx).clone();
    next.package_visual_settings
        .insert(package.to_owned(), values);
    next.validate().map_err(|error| format!("{error:#}"))?;
    update(cx, |config| *config = next);
    Ok(revision(cx))
}
