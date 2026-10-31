//! Fixed composer contributions use the ordinary Badge/Popup process roles.
//! Their owning chat retains the summary probe only while it is visible;
//! disclosure details mount on demand and share that exact bound runtime.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::*;

use super::view::DeclarativeView;
use super::{Host, SurfaceKind, embedded_surface, installed, runtime};
use crate::declarative::presentation::Face;
use crate::ui::extensions::SurfaceView;

pub(in crate::ui) struct ActivitySummary {
    pub label: String,
    pub icon: Lucide,
    /// The first presentation line is the package's compact contextual hint.
    pub current: Option<String>,
}

pub(in crate::ui) struct ActivityContribution {
    host: Host,
    summary: Entity<DeclarativeView>,
    details: Option<SurfaceView>,
    popup_epoch: u64,
    _summary: Subscription,
}

impl ActivityContribution {
    pub(in crate::ui) fn new<T: 'static>(
        host: Host,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let runtime = runtime(&host, window, cx).expect("activity comes from the catalog");
        let summary = cx.new(|cx| {
            DeclarativeView::new(runtime, host.clone(), SurfaceKind::Badge, None, window, cx)
        });
        Self {
            _summary: cx.observe(&summary, |_, _, cx| cx.notify()),
            host,
            summary,
            details: None,
            popup_epoch: 0,
        }
    }

    pub(in crate::ui) fn close_requested(&mut self, cx: &App) -> bool {
        let epoch = self.summary.read(cx).popup_closed(cx);
        let changed = epoch != self.popup_epoch;
        self.popup_epoch = epoch;
        changed
    }

    pub(in crate::ui) fn summary(&self, cx: &App) -> Option<ActivitySummary> {
        let extension = installed(self.host.extension())?;
        if !super::super::is_enabled(self.host.extension(), cx)
            || extension.manifest.surfaces.activity.is_none()
        {
            return None;
        }
        let view = self.summary.read(cx);
        let error = view.error(cx);
        let presentation = view.presentation.as_ref();
        if error.is_none() && presentation.is_none_or(|presentation| presentation.hidden) {
            return None;
        }
        let label = presentation
            .map(|presentation| match &presentation.face {
                Face::Text { text, .. } if !text.is_empty() => text.clone(),
                _ => presentation.title.clone(),
            })
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| extension.manifest.title.clone());
        let current = error.or_else(|| {
            presentation
                .and_then(|presentation| presentation.lines.first())
                .filter(|line| !line.is_empty())
                .cloned()
        });
        Some(ActivitySummary {
            label,
            icon: super::icon(extension.manifest.icon.as_deref()),
            current,
        })
    }

    /// Keep a closing disclosure alive only until its shared reveal settles.
    pub(in crate::ui) fn details(
        &mut self,
        keep: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<SurfaceView> {
        if !keep {
            self.details = None;
            return None;
        }
        if self.details.is_none()
            && installed(self.host.extension())?
                .manifest
                .surfaces
                .popup
                .is_some()
        {
            self.details = Some(embedded_surface(
                self.host.clone(),
                SurfaceKind::Popup,
                None,
                window,
                cx,
            ));
        }
        self.details.clone()
    }
}
