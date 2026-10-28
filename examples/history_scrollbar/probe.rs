//! Frame-settled native regression checks for the handle used by the chat.
use super::*;

/// Timers only poll progress. A sample requires four real render/layout frames
/// and unchanged geometry in two consecutive frames, including fractional seek
/// settlement. This also works when a compositor renders slower than 30 ms.
async fn settled(this: &WeakEntity<Fixture>, cx: &mut AsyncApp) -> serde_json::Value {
    let initial = this
        .update(cx, |this, cx| {
            this.probe_frames = true;
            cx.notify();
            this.render_generation
        })
        .unwrap();
    let mut generation = initial;
    let mut previous = None;
    let mut stable = 0;
    for _ in 0..1000 {
        cx.background_executor()
            .timer(Duration::from_millis(5))
            .await;
        let sample = this.read_with(cx, |this, _| this.verify()).unwrap();
        let current = sample["render_generation"].as_u64().unwrap();
        if current == generation {
            continue;
        }
        generation = current;
        let mut geometry = sample.clone();
        geometry
            .as_object_mut()
            .unwrap()
            .remove("render_generation");
        if previous.as_ref() == Some(&geometry) {
            stable += 1;
        } else {
            stable = 0;
        }
        previous = Some(geometry);
        if current >= initial + 4 && stable >= 2 && sample["viewport_height"].as_f64().unwrap() > 0.
        {
            this.update(cx, |this, _| this.probe_frames = false)
                .unwrap();
            return sample;
        }
    }
    panic!("native scrollbar geometry did not settle after render {initial}");
}

fn steps(samples: &[serde_json::Value], field: &str) -> Vec<f64> {
    samples
        .windows(2)
        .map(|pair| pair[1][field].as_f64().unwrap() - pair[0][field].as_f64().unwrap())
        .collect()
}

fn summary(steps: &[f64]) -> serde_json::Value {
    let min = steps.iter().copied().fold(f64::INFINITY, f64::min);
    let max = steps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    serde_json::json!({"min": min, "max": max, "spread": max / min})
}

fn uniform(steps: &[f64]) -> bool {
    let min = steps.iter().copied().fold(f64::INFINITY, f64::min);
    let max = steps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // Absolute slack covers f32 rounding of a virtual offset above 1,000,000.
    min > 0. && max <= min * 1.25 + 0.25
}

pub(super) fn start(report: PathBuf, cx: &mut Context<Fixture>) {
    cx.spawn(async move |this, cx| {
        settled(&this, cx).await;
        this.update(cx, |this, cx| {
            this.list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            cx.notify();
        })
        .unwrap();
        let mut wheel = vec![settled(&this, cx).await];
        for _ in 0..384 {
            this.update(cx, |this, cx| {
                // This uses the native list's wheel path, excluding OS input
                // acceleration, so every requested displacement is identical.
                this.scrollbar.cancel_target();
                this.list.scroll_by(px(8.));
                cx.notify();
            })
            .unwrap();
            wheel.push(settled(&this, cx).await);
        }
        let drag_start = this
            .read_with(cx, |this, _| -this.scrollbar.offset().y)
            .unwrap();
        this.update(cx, |this, _| this.scrollbar.start_drag())
            .unwrap();
        let mut drag = vec![settled(&this, cx).await];
        for step in 1..=96 {
            this.update(cx, |this, cx| {
                this.scrollbar
                    .set_offset(point(px(0.), -(drag_start + px(step as f32 * 16.))));
                cx.notify();
            })
            .unwrap();
            drag.push(settled(&this, cx).await);
        }
        this.update(cx, |this, cx| {
            this.scrollbar.end_drag();
            cx.notify();
        })
        .unwrap();
        let released = settled(&this, cx).await;
        let mut endpoints = Vec::new();
        for share in [0., 1., 0.5] {
            this.update(cx, |this, cx| {
                let max =
                    this.scrollbar.content_size().height - this.list.viewport_bounds().size.height;
                this.scrollbar.set_offset(point(px(0.), -max * share));
                cx.notify();
            })
            .unwrap();
            endpoints.push(settled(&this, cx).await);
        }
        // First populate the measured-height cache, then emulate a font/width
        // change and seek before the new layout has been measured. A cached
        // old height must not settle the new fractional position prematurely.
        let target_row = this
            .update(cx, |this, cx| {
                let row = (this.start + this.rows / 2 + 3) / 4 * 4;
                this.scrollbar
                    .scroll_to(ScrollTarget { row, fraction: 0.5 });
                cx.notify();
                row
            })
            .unwrap();
        let before_remeasure = settled(&this, cx).await;
        let expected_row_offset = this
            .update(cx, |this, cx| {
                this.height_scale = 0.5;
                this.list.remeasure();
                this.scrollbar.invalidate(0..this.rows);
                this.scrollbar.scroll_to(ScrollTarget {
                    row: target_row,
                    fraction: 0.5,
                });
                cx.notify();
                row_height(target_row, this.mixed_heights) * this.height_scale * 0.5
            })
            .unwrap();
        let after_remeasure = settled(&this, cx).await;
        let wheel_virtual = steps(&wheel, "offset");
        let wheel_physical = steps(&wheel, "physical_offset");
        let drag_virtual = steps(&drag, "offset");
        let drag_physical = steps(&drag, "physical_offset");
        let height = wheel[0]["content_height"].as_f64().unwrap();
        let mut failures = Vec::new();
        if before_remeasure["global_anchor"].as_u64().unwrap() != target_row as u64
            || after_remeasure["global_anchor"].as_u64().unwrap() != target_row as u64
            || (after_remeasure["row_offset"].as_f64().unwrap() - f64::from(expected_row_offset))
                .abs()
                > 0.1
        {
            failures.push("fractional seek uses stale row height after native remeasurement");
        }
        if wheel_physical.iter().any(|step| (step - 8.).abs() > 0.05) {
            failures.push("native wheel displacement differs from 8 px");
        }
        if !uniform(&wheel_virtual) {
            failures.push("thumb speed changes across row heights during wheel motion");
        }
        if !uniform(&drag_physical) {
            failures.push("content stalls or changes speed across row heights during thumb drag");
        }
        if drag_virtual.iter().any(|step| (step - 16.).abs() > 0.5) {
            failures.push("thumb does not follow requested drag position");
        }
        if wheel
            .iter()
            .chain(&drag)
            .chain(&endpoints)
            .any(|sample| (sample["content_height"].as_f64().unwrap() - height).abs() > 0.1)
        {
            failures.push("thumb extent changes during the gesture or distant seek");
        }
        if (released["offset"].as_f64().unwrap() - drag.last().unwrap()["offset"].as_f64().unwrap())
            .abs()
            > 0.5
        {
            failures.push("thumb jumps after release");
        }
        if endpoints[0]["global_anchor"].as_u64().unwrap() != 0 {
            failures.push("top of full history is unreachable");
        }
        let bottom = &endpoints[1];
        let max = height - bottom["viewport_height"].as_f64().unwrap();
        if (bottom["offset"].as_f64().unwrap() - max).abs() > 0.5 {
            failures.push("bottom of full history is unreachable");
        }
        if let Some(parent) = report.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(
            &report,
            serde_json::to_vec_pretty(&serde_json::json!({
                "status": if failures.is_empty() { "passed" } else { "failed" },
                "scope": "frame-settled native pixel steps and scrollbar drag",
                "failures": failures,
                "metrics": {
                    "wheel_virtual": summary(&wheel_virtual),
                    "wheel_physical": summary(&wheel_physical),
                    "drag_virtual": summary(&drag_virtual),
                    "drag_physical": summary(&drag_physical),
                },
                "wheel": wheel, "drag": drag, "released": released, "endpoints": endpoints,
                "remeasure": {
                    "target_row": target_row,
                    "fraction": 0.5,
                    "expected_row_offset": expected_row_offset,
                    "before": before_remeasure,
                    "after": after_remeasure,
                },
            }))
            .unwrap(),
        )
        .unwrap();
        eprintln!("Native scrollbar probe: {}", report.display());
        if !failures.is_empty() {
            eprintln!("FAIL: {}", failures.join("; "));
            std::process::exit(1);
        }
        this.update(cx, |_, cx| cx.quit()).ok();
    })
    .detach();
}
