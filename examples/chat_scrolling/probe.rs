//! Native dispatch drives the production hitboxes; timers never implement
//! scrolling and never force a render, so missing/stuck ticks are observable.
use std::time::Duration;

use super::*;

fn sample(this: &WeakEntity<Fixture>, cx: &mut AsyncApp) -> anyhow::Result<serde_json::Value> {
    this.read_with(cx, |this, cx| this.snapshot(cx))?
}

fn offset(sample: &serde_json::Value) -> f64 {
    sample["physical_offset"].as_f64().expect("physical offset")
}

async fn pause(cx: &AsyncApp, millis: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(millis))
        .await;
}

fn send(window: AnyWindowHandle, cx: &mut AsyncApp, event: impl InputEvent) -> anyhow::Result<()> {
    window.update(cx, |_, window, cx| {
        window.dispatch_event(event.to_platform_input(), cx);
    })?;
    Ok(())
}

fn movement(position: Point<Pixels>) -> MouseMoveEvent {
    MouseMoveEvent {
        position,
        pressed_button: Some(MouseButton::Middle),
        ..Default::default()
    }
}

fn press(position: Point<Pixels>) -> MouseDownEvent {
    MouseDownEvent {
        position,
        button: MouseButton::Middle,
        click_count: 1,
        ..Default::default()
    }
}

async fn stopped(
    name: &str,
    this: &WeakEntity<Fixture>,
    cx: &mut AsyncApp,
    samples: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    // Let the already queued final frame and input modality refresh settle.
    // The native scrollbar also schedules a finite idle-hide notification of
    // its parent (gpui-base scrollbar.rs). Count the actual scroller's
    // notifications separately, so that timer cannot mask a stuck middle loop
    // or make its absence look like a failure.
    pause(cx, 300).await;
    let before = sample(this, cx)?;
    pause(cx, 600).await;
    let after = sample(this, cx)?;
    samples.push(serde_json::json!({"stage":name, "before":before, "after":after}));
    anyhow::ensure!(
        (offset(&after) - offset(&before)).abs() < 0.5,
        "{name}: scroll continued after stop"
    );
    anyhow::ensure!(
        before["scroller_notifications"] == after["scroller_notifications"],
        "{name}: scroller kept notifying after stop"
    );

    // Keep the stronger whole-parent assertion too, after the production
    // scrollbar's configured finite visibility/width transitions can finish.
    // This does not disable or replace the real scrollbar in the fixture.
    let settle_ms = this.read_with(cx, |_, cx| {
        let motion = gpui_kit::base::Theme::global(cx).scrollbar.motion();
        (motion.idle() + motion.enter() + motion.exit() + motion.expand()).as_millis() as u64 + 300
    })?;
    pause(cx, settle_ms).await;
    let settled = sample(this, cx)?;
    pause(cx, 600).await;
    let idle = sample(this, cx)?;
    samples.push(serde_json::json!({
        "stage":format!("{name}-settled"), "scrollbar_settle_ms":settle_ms,
        "before":settled, "after":idle,
    }));
    anyhow::ensure!(
        (offset(&idle) - offset(&before)).abs() < 0.5,
        "{name}: scroll resumed during scrollbar settling"
    );
    anyhow::ensure!(
        before["scroller_notifications"] == idle["scroller_notifications"],
        "{name}: scroller resumed notifying during scrollbar settling"
    );
    anyhow::ensure!(
        settled["parent_renders"] == idle["parent_renders"],
        "{name}: native chat kept rendering after scrollbar settled"
    );
    Ok(())
}

fn end_bottom(sample: &serde_json::Value, thinking: bool) -> anyhow::Result<f64> {
    sample[if thinking {
        "footer_bottom"
    } else {
        "last_row_bottom"
    }]
    .as_f64()
    .ok_or_else(|| anyhow::anyhow!("final row/caption has no native prepaint geometry"))
}

async fn smooth_end(
    name: &str,
    thinking: bool,
    this: &WeakEntity<Fixture>,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
    samples: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    this.update(cx, |this, cx| this.reset_tail(ROWS, thinking, false, cx))?;
    pause(cx, 400).await;
    // A whole initial splice uses the list's natural end anchor. This is an
    // independent geometry oracle, not another copy of ChatScroller::gap.
    let initial = sample(this, cx)?;
    let expected = end_bottom(&initial, thinking)?;
    let viewport = this.read_with(cx, |this, _| this.geometry.viewport.get())?;
    let origin = point(
        viewport.left() + px(350.),
        viewport.top() + viewport.size.height / 2.,
    );
    send(
        window,
        cx,
        ScrollEvent {
            position: origin,
            delta: ScrollDelta::Pixels(point(px(0.), px(180.))),
            ..Default::default()
        },
    )?;
    pause(cx, 300).await;
    let above = sample(this, cx)?;
    anyhow::ensure!(
        (offset(&above) - offset(&initial) + 180.).abs() < 2.,
        "{name}: setup did not scroll up 180 physical pixels"
    );
    this.update(cx, |this, cx| {
        this.scroller
            .update(cx, |scroller, cx| scroller.scroll_to_end(cx))
    })?;
    // The real motion tick must reach the original anchor. Merely fitting
    // the caption inside the viewport would miss the 16px under-scroll.
    pause(cx, 1800).await;
    let after = sample(this, cx)?;
    let error = end_bottom(&after, thinking)? - expected;
    samples.push(
        serde_json::json!({"stage":name, "initial":initial, "scrolled_up":above,
        "after":after, "end_anchor_error_px":error}),
    );
    anyhow::ensure!(
        error.abs() <= 0.5,
        "{name}: smooth return missed natural native end anchor by {error}px"
    );
    stopped(name, this, cx, samples).await
}

async fn tail_cases(
    this: &WeakEntity<Fixture>,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
    samples: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    crate::ui::motion::set_plays(true);
    let result = async {
        smooth_end("smooth-end-row", false, this, window, cx, samples).await?;
        smooth_end("smooth-end-thinking", true, this, window, cx, samples).await?;
        for (name, count) in [("short-end-idle", 2), ("empty-end-idle", 0)] {
            this.update(cx, |this, cx| this.reset_tail(count, false, true, cx))?;
            pause(cx, 400).await;
            let initial = sample(this, cx)?;
            this.update(cx, |this, cx| {
                this.scroller
                    .update(cx, |scroller, cx| scroller.scroll_to_end(cx))
            })?;
            stopped(name, this, cx, samples).await?;
            let after = sample(this, cx)?;
            samples.push(serde_json::json!({"stage":name, "initial":initial, "after":after}));
            anyhow::ensure!(
                initial["last_row_bottom"] == after["last_row_bottom"],
                "{name}: a fitting list acquired phantom scroll room"
            );
        }
        anyhow::Ok(())
    }
    .await;
    crate::ui::motion::set_plays(false);
    result
}

async fn verify(
    this: &WeakEntity<Fixture>,
    window: AnyWindowHandle,
    cx: &mut AsyncApp,
    samples: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    let mut ready = false;
    for _ in 0..100 {
        pause(cx, 100).await;
        if sample(this, cx).is_ok()
            && window.update(cx, |_, window, _| window.is_window_active())?
        {
            ready = true;
            break;
        }
    }
    anyhow::ensure!(
        ready,
        "native window did not receive a real layout and activation"
    );
    pause(cx, 300).await;
    let viewport = this.read_with(cx, |this, _| this.geometry.viewport.get())?;
    let origin = point(
        viewport.left() + px(350.),
        viewport.top() + viewport.size.height / 2.,
    );
    send(
        window,
        cx,
        MouseMoveEvent {
            position: origin,
            ..Default::default()
        },
    )?;
    pause(cx, 100).await;

    // Unknown rows are deliberately taller than the measured overdraw. All
    // events arrive before another real layout; no direct list calls occur.
    let before = sample(this, cx)?;
    let first_frame = this.read_with(cx, |this, _| this.geometry.frames.borrow().len())?;
    window.update(cx, |_, window, cx| {
        for _ in 0..18 {
            window.dispatch_event(
                ScrollEvent {
                    position: origin,
                    delta: ScrollDelta::Pixels(point(px(0.), px(-180.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    })?;
    let mut after = sample(this, cx)?;
    for _ in 0..100 {
        pause(cx, 50).await;
        after = sample(this, cx)?;
        if (offset(&after) - offset(&before) - 3240.).abs() < 2. {
            break;
        }
    }
    samples.push(
        serde_json::json!({"stage":"burst-wheel", "requested_pixels":3240,
        "before":before, "after":after}),
    );
    anyhow::ensure!(
        (offset(&after) - offset(&before) - 3240.).abs() < 2.,
        "18 wheel events did not travel exactly 3240 physical pixels: {}",
        offset(&after) - offset(&before)
    );
    let frames = this.read_with(cx, |this, _| {
        this.geometry.frames.borrow()[first_frame..].to_vec()
    })?;
    let mut previous = offset(&before);
    let mut largest_step: f64 = 0.;
    for frame in &frames {
        largest_step = largest_step.max((offset(frame) - previous).abs());
        previous = offset(frame);
    }
    anyhow::ensure!(
        frames.len() >= 17 && (previous - offset(&after)).abs() < 2.,
        "physical frame trace missed wheel layouts"
    );
    samples.push(serde_json::json!({"stage":"burst-wheel-real-layouts",
        "largest_physical_step":largest_step, "frames":frames}));
    anyhow::ensure!(
        largest_step <= 200.5,
        "wheel crossed unmeasured rows in one layout: {largest_step} physical pixels"
    );

    // A click/hold and ten-pixel wobble remain within the deadzone.
    send(window, cx, press(origin))?;
    pause(cx, 80).await;
    let before = sample(this, cx)?;
    send(window, cx, movement(origin + point(px(20.), px(10.))))?;
    pause(cx, 400).await;
    let after = sample(this, cx)?;
    samples.push(serde_json::json!({"stage":"middle-deadzone", "before":before, "after":after}));
    anyhow::ensure!(
        (offset(&after) - offset(&before)).abs() < 0.5,
        "middle deadzone moved chat"
    );

    send(window, cx, movement(origin + point(px(0.), px(50.))))?;
    let before = sample(this, cx)?;
    pause(cx, 400).await;
    let near = sample(this, cx)?;
    let near_distance = offset(&near) - offset(&before);
    send(window, cx, movement(origin + point(px(0.), px(160.))))?;
    pause(cx, 400).await;
    let far = sample(this, cx)?;
    let far_distance = offset(&far) - offset(&near);
    samples.push(
        serde_json::json!({"stage":"middle-speed", "before":before, "near":near,
        "far":far, "near_distance":near_distance, "far_distance":far_distance}),
    );
    anyhow::ensure!(
        near_distance > 25.,
        "held pointer below origin did not move down"
    );
    anyhow::ensure!(
        far_distance > near_distance * 1.5,
        "greater pointer displacement did not increase speed"
    );

    send(window, cx, movement(origin - point(px(0.), px(120.))))?;
    pause(cx, 400).await;
    let above = sample(this, cx)?;
    samples.push(serde_json::json!({"stage":"middle-reverse", "before":far, "after":above}));
    anyhow::ensure!(
        offset(&above) < offset(&far) - 25.,
        "held pointer above origin did not move up"
    );

    send(
        window,
        cx,
        MouseUpEvent {
            position: point(origin.x, viewport.top() - px(30.)),
            button: MouseButton::Middle,
            click_count: 1,
            ..Default::default()
        },
    )?;
    stopped("release-outside-chat", this, cx, samples).await?;

    send(window, cx, press(origin))?;
    pause(cx, 80).await;
    send(window, cx, movement(origin + point(px(0.), px(120.))))?;
    let before = sample(this, cx)?;
    pause(cx, 300).await;
    let after = sample(this, cx)?;
    anyhow::ensure!(
        offset(&after) > offset(&before) + 25.,
        "second middle gesture did not start"
    );
    send(
        window,
        cx,
        KeyDownEvent {
            keystroke: Keystroke::parse("escape")?,
            is_held: false,
            prefer_character_input: false,
        },
    )?;
    stopped("escape", this, cx, samples).await?;

    // No final MouseUp is needed after Escape; ordinary wheel remains usable.
    let before = sample(this, cx)?;
    send(
        window,
        cx,
        ScrollEvent {
            position: origin,
            delta: ScrollDelta::Pixels(point(px(0.), px(180.))),
            ..Default::default()
        },
    )?;
    pause(cx, 300).await;
    let after = sample(this, cx)?;
    samples.push(serde_json::json!({"stage":"wheel-after-escape", "before":before, "after":after}));
    anyhow::ensure!(
        (offset(&after) - offset(&before) + 180.).abs() < 2.,
        "wheel was captured after Escape"
    );
    tail_cases(this, window, cx, samples).await
}

pub(super) fn start(report: PathBuf, window: AnyWindowHandle, cx: &mut Context<Fixture>) {
    cx.spawn(async move |this, cx| {
        let mut samples = Vec::new();
        let result = verify(&this, window, cx, &mut samples).await;
        let output = serde_json::json!({
            "status":if result.is_ok() { "passed" } else { "failed" },
            "scope":"native production ChatScroller/ChatList platform input and physical geometry",
            "fixture_rows":ROWS, "samples":samples,
            "error":result.as_ref().err().map(|error| format!("{error:#}")),
        });
        if let Some(parent) = report.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&report, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
        eprintln!(
            "{}: native chat scrolling; {}",
            output["status"],
            report.display()
        );
        this.update(cx, |_, cx| cx.quit()).ok();
    })
    .detach();
}
