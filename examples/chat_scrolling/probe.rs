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
    Ok(())
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
