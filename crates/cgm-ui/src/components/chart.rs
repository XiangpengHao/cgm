//! The glucose chart — a self-contained, dependency-free SVG line chart with a
//! green/amber/red background zones, hour gridlines, event markers, and
//! touch-first interaction:
//!
//! * **drag** left/right to pan the visible window (mouse drag or touch swipe),
//! * **tap / hover** to pin a value readout (works on touch, where there is no
//!   hover),
//! * **double-click** (desktop) or the **＋ Log event** button (touch) to log an
//!   event.
//!
//! The SVG is fluid (`width:100%` + a viewBox matching the measured container
//! width), so it fills its card on every viewport and keeps pointer hit-testing
//! 1:1; it re-measures on resize/rotation.

use crate::format::icon_for;
use crate::platform::SharedPlatform;
use crate::state::{AppState, EventDraft};
use crate::ui::BTN_GHOST_SM;
use cgm_core::glucose::record_valid;
use cgm_core::ranges::{band_edges, classify};
use cgm_core::stats::{convert, format_value};
use dioxus::prelude::*;

// Plot geometry. The SVG renders at the measured pixel width with viewBox ==
// width (and no preserveAspectRatio), so 1 user unit == 1 rendered px and text
// is never distorted. Height is responsive (see `chart_height`).
const PAD_L: f64 = 44.0;
const PAD_R: f64 = 14.0;
const PAD_T: f64 = 14.0;
const PAD_B: f64 = 26.0;
const HOUR_MS: i64 = 3_600_000;

struct Pt {
    t: i64,
    v: f64,
    valid: bool,
}

/// Nearest valid reading to instant `t`, as `(t, value)`.
fn nearest(pts: &[(i64, f64, bool)], t: i64) -> Option<(i64, f64)> {
    pts.iter()
        .filter(|(_, _, valid)| *valid)
        .min_by_key(|(pt, _, _)| (pt - t).abs())
        .map(|(pt, v, _)| (*pt, *v))
}

#[component]
pub fn GlucoseChart() -> Element {
    let state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let mut hover = use_signal(|| None::<(f64, f64, String)>);
    // Drag state captured at pointer-down: (start_px, start_t0, start_t1).
    let mut drag = use_signal(|| None::<(f64, i64, i64)>);
    let mut chart_view = state.chart_view;
    let mut event_draft = state.event_draft;
    let mut width_sig = state.chart_width;

    let unit = state.settings.read().unit;
    let mgdl = unit.is_mgdl();
    let offset = platform.clock().local_offset_minutes();
    let now = platform.clock().now_ms();
    // Drives both the viewBox and the x-axis math; must equal the measured CSS
    // width so element_coordinates() (CSS px) maps 1:1 to user units.
    let width = state.chart_width.read().max(1.0);
    // Shorter on phones to fit the glance; taller on desktop.
    let h: f64 = if width < 480.0 {
        240.0
    } else if width < 768.0 {
        280.0
    } else {
        320.0
    };
    let window_hours = *state.window_hours.read();
    let view = *state.chart_view.read();

    let data = state.data.read();
    let start_ms = data.sensor_start_ms();

    let mut pts: Vec<Pt> = Vec::new();
    if let Some(s0) = start_ms {
        for (&i, &(g, rs)) in data.records.iter() {
            pts.push(Pt {
                t: s0 + i as i64 * 60_000,
                v: convert(g, unit),
                valid: record_valid(g, rs, i as i32),
            });
        }
    }

    let latest = pts.last().map(|p| p.t).unwrap_or(now);
    let earliest = pts.first().map(|p| p.t).unwrap_or(latest - 12 * HOUR_MS);
    // Visible window: an explicit panned view, else follow the latest reading.
    let (t0, t1) = match view {
        Some(v) => v,
        None if window_hours == 0 => (earliest.min(latest), latest),
        None => (latest - window_hours as i64 * HOUR_MS, latest),
    };
    let span = (t1 - t0).max(1) as f64;

    // Soft y-domain anchors (the two zone boundaries) so both lines are always
    // in view; they come from the single source of truth.
    let anchor = band_edges(unit);
    let mut ymin = anchor[0];
    let mut ymax = anchor[1];
    for p in pts.iter().filter(|p| p.valid && p.t >= t0 && p.t <= t1) {
        ymin = ymin.min(p.v);
        ymax = ymax.max(p.v);
    }
    if ymax <= ymin {
        ymax = ymin + 1.0;
    }
    let pad = (ymax - ymin) * 0.12;
    ymin -= pad;
    ymax += pad;

    let plot_w = (width - PAD_L - PAD_R).max(1.0);
    let plot_h = h - PAD_T - PAD_B;
    let x = move |t: i64| PAD_L + (t - t0) as f64 / span * plot_w;
    let y = move |v: f64| PAD_T + (ymax - v) / (ymax - ymin) * plot_h;

    // Background glucose zones from the single source of truth: green in-range
    // (≤ 5.6), amber elevated (5.6–7.8), red high (> 7.8). Edges come from
    // ranges::band_edges so they can never disagree with time-in-range or the
    // hero. Drawn as low-alpha bands clipped to the visible domain.
    let edges = band_edges(unit); // [5.6, 7.8] in display units
    let zone_band = move |v_lo: f64, v_hi: f64| -> (f64, f64) {
        let lo = v_lo.max(ymin);
        let hi = v_hi.min(ymax);
        if hi <= lo { (0.0, 0.0) } else { (y(hi), y(lo) - y(hi)) }
    };
    let in_range = zone_band(ymin, edges[0]); // ≤ 5.6 green
    let elevated = zone_band(edges[0], edges[1]); // 5.6–7.8 amber
    let high = zone_band(edges[1], ymax); // > 7.8 red

    // Dashed, labeled boundary lines at both edges (5.6 / 7.8), shown when in view.
    let fmt_edge = move |v: f64| -> String {
        if mgdl {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.1}")
        }
    };
    let edge_lines: Vec<(f64, String)> = edges
        .iter()
        .filter(|&&v| v > ymin && v < ymax)
        .map(|&v| (y(v), fmt_edge(v)))
        .collect();

    let mut line = String::new();
    let mut warm: Vec<(f64, f64)> = Vec::new();
    for p in pts.iter().filter(|p| p.t >= t0 && p.t <= t1) {
        let (px, py) = (x(p.t), y(p.v));
        if p.valid {
            if line.is_empty() {
                line.push_str(&format!("{px:.1},{py:.1}"));
            } else {
                line.push_str(&format!(" {px:.1},{py:.1}"));
            }
        } else {
            warm.push((px, py));
        }
    }

    // Width-responsive label count: ~3–4 on a phone, up to 8 on desktop, and
    // skip any tick that would clip the right edge.
    let mut grid: Vec<(f64, String)> = Vec::new();
    let first_hour = (t0 / HOUR_MS) * HOUR_MS;
    let target_ticks = ((plot_w / 64.0).floor() as i64).clamp(2, 8);
    let span_hours = ((t1 - t0) / HOUR_MS).max(1);
    let step = (span_hours / target_ticks).max(1) * HOUR_MS;
    let mut tk = first_hour;
    while tk <= t1 {
        let gx = x(tk);
        if tk >= t0 && gx <= width - PAD_R - 12.0 {
            grid.push((gx, cgm_core::datetime::format_hm(tk, offset)));
        }
        tk += step;
    }

    let mut yticks: Vec<(f64, String)> = Vec::new();
    for k in 0..=4 {
        let v = ymin + (ymax - ymin) * k as f64 / 4.0;
        let label = if mgdl {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.1}")
        };
        yticks.push((y(v), label));
    }

    let events: Vec<(f64, String)> = data
        .events
        .iter()
        .filter(|e| e.t >= t0 && e.t <= t1)
        .map(|e| {
            let icon = e.icon.clone().unwrap_or_else(|| icon_for(&e.label));
            (x(e.t), format!("{icon} {}", e.label))
        })
        .collect();

    let has_points = !pts.is_empty();

    // Accessible summary of the latest valid reading, including its zone.
    let aria = match data.latest() {
        Some((idx, g, rs)) if record_valid(g, rs, idx as i32) => format!(
            "Glucose chart. Latest {} {}, {}.",
            format_value(g, unit),
            unit.label(),
            classify(g).label()
        ),
        _ => "Glucose chart".to_string(),
    };

    let to_time = move |elem_x: f64| -> i64 {
        let frac = ((elem_x - PAD_L) / plot_w).clamp(0.0, 1.0);
        t0 + (frac * span) as i64
    };

    // Only points in the visible window are pinnable, so the marker never lands
    // off the plot.
    let move_pts: Vec<(i64, f64, bool)> = pts
        .iter()
        .filter(|p| p.t >= t0 && p.t <= t1)
        .map(|p| (p.t, p.v, p.valid))
        .collect();
    // A closure that pins the readout at the nearest valid reading to `elem_x`.
    let pin_pts = move_pts.clone();
    let pin = move |elem_x: f64, mut hover: Signal<Option<(f64, f64, String)>>| {
        if let Some((bt, bv)) = nearest(&pin_pts, to_time(elem_x)) {
            let label = if mgdl {
                format!(
                    "{} mg/dL · {}",
                    bv.round() as i64,
                    cgm_core::datetime::format_hm(bt, offset)
                )
            } else {
                format!("{bv:.1} mmol/L · {}", cgm_core::datetime::format_hm(bt, offset))
            };
            hover.set(Some((x(bt), y(bv), label)));
        }
    };

    let pin_down = pin.clone();
    let on_down = move |evt: PointerEvent| {
        let ex = evt.element_coordinates().x;
        drag.set(Some((ex, t0, t1)));
        pin_down(ex, hover); // tap-to-read (works on touch, where there is no hover)
    };

    let on_move = move |evt: PointerEvent| {
        let ex = evt.element_coordinates().x;
        if let Some((sx, st0, st1)) = drag() {
            let dx = ex - sx;
            if dx.abs() <= 4.0 {
                pin(ex, hover); // a small move is a tap → read, don't pan
                return;
            }
            // Pan: shift the window by the dragged distance, clamped to data.
            let wspan = st1 - st0;
            let dt = (-(dx) / plot_w * wspan as f64) as i64;
            let (mut a, mut b) = (st0 + dt, st1 + dt);
            let range = latest - earliest;
            if wspan >= range {
                a = earliest;
                b = latest;
            } else {
                if a < earliest {
                    a = earliest;
                    b = earliest + wspan;
                }
                if b > latest {
                    b = latest;
                    a = latest - wspan;
                }
            }
            chart_view.set(Some((a, b)));
        } else {
            pin(ex, hover);
        }
    };

    // A normal release keeps the pinned readout; cancel/leave clear it.
    let end_drag = move |_: PointerEvent| drag.set(None);
    let clear_all = move |_: PointerEvent| {
        drag.set(None);
        hover.set(None);
    };

    let on_dbl = move |evt: MouseEvent| {
        let t = to_time(evt.element_coordinates().x);
        let c = evt.client_coordinates();
        event_draft.set(Some(EventDraft {
            t_ms: t,
            x: c.x,
            y: c.y,
            anchored: true,
        }));
    };

    let log_event = move |_| {
        // Touch-friendly path: log at the latest reading, popover centered.
        event_draft.set(Some(EventDraft {
            t_ms: latest,
            x: 0.0,
            y: 0.0,
            anchored: false,
        }));
    };

    let on_mounted = move |evt: MountedEvent| {
        spawn(async move {
            if let Ok(rect) = evt.get_client_rect().await {
                let w = rect.width();
                if w > 50.0 {
                    width_sig.set(w);
                }
            }
        });
    };
    let on_resize = move |evt: Event<ResizeData>| {
        if let Ok(size) = evt.get_content_box_size()
            && size.width > 50.0
        {
            width_sig.set(size.width);
        }
    };

    rsx! {
        div { class: "w-full select-none", onmounted: on_mounted, onresize: on_resize,
            svg {
                width: "{width}",
                height: "{h}",
                view_box: "0 0 {width} {h}",
                // No preserveAspectRatio: viewBox == pixel width, so glyphs never
                // scale (CSS stretches the box via the style width:100%).
                role: "img",
                "aria-label": "{aria}",
                style: "display:block; width:100%; max-width:100%; touch-action: pan-y; cursor: crosshair;",
                onpointerdown: on_down,
                onpointermove: on_move,
                onpointerup: end_drag,
                onpointerleave: clear_all,
                onpointercancel: clear_all,
                ondoubleclick: on_dbl,

                // Background zones (single source of truth): green in-range
                // (≤ 5.6), amber elevated (5.6–7.8), red high (> 7.8).
                // Desaturated so the line stays the focus.
                if in_range.1 > 0.0 {
                    rect { x: "{PAD_L}", y: "{in_range.0}", width: "{plot_w}", height: "{in_range.1}", fill: "rgb(55 211 154 / 0.12)" }
                }
                if elevated.1 > 0.0 {
                    rect { x: "{PAD_L}", y: "{elevated.0}", width: "{plot_w}", height: "{elevated.1}", fill: "rgb(240 169 43 / 0.12)" }
                }
                if high.1 > 0.0 {
                    rect { x: "{PAD_L}", y: "{high.0}", width: "{plot_w}", height: "{high.1}", fill: "rgb(255 93 93 / 0.14)" }
                }
                // Labeled dashed boundary lines at the two zone edges (5.6 / 7.8).
                for (ey , label) in edge_lines.iter() {
                    line {
                        x1: "{PAD_L}", y1: "{ey}", x2: "{width - PAD_R}", y2: "{ey}",
                        stroke: "currentColor", stroke_width: "1", stroke_dasharray: "3 3", opacity: "0.35",
                    }
                    text {
                        x: "{PAD_L + 3.0}", y: "{ey - 2.0}", font_size: "9",
                        fill: "currentColor", opacity: "0.6",
                        "{label}"
                    }
                }

                for (gy , label) in yticks.iter() {
                    line {
                        x1: "{PAD_L}", y1: "{gy}", x2: "{width - PAD_R}", y2: "{gy}",
                        stroke: "currentColor", stroke_width: "0.5", opacity: "0.12",
                    }
                    text {
                        x: "{PAD_L - 6.0}", y: "{gy + 3.0}", text_anchor: "end",
                        font_size: "10", fill: "currentColor", opacity: "0.55",
                        "{label}"
                    }
                }

                for (gx , label) in grid.iter() {
                    line {
                        x1: "{gx}", y1: "{PAD_T}", x2: "{gx}", y2: "{h - PAD_B}",
                        stroke: "currentColor", stroke_width: "0.5", opacity: "0.08",
                    }
                    text {
                        x: "{gx}", y: "{h - PAD_B + 16.0}", text_anchor: "middle",
                        font_size: "10", fill: "currentColor", opacity: "0.55",
                        "{label}"
                    }
                }

                for (ex , label) in events.iter() {
                    line {
                        x1: "{ex}", y1: "{PAD_T}", x2: "{ex}", y2: "{h - PAD_B}",
                        stroke: "currentColor", stroke_width: "1", stroke_dasharray: "2 3", opacity: "0.4",
                    }
                    text {
                        x: "{ex + 3.0}", y: "{PAD_T + 10.0}", font_size: "10",
                        fill: "currentColor", opacity: "0.7", "{label}"
                    }
                }

                for (wx , wy) in warm.iter() {
                    circle { cx: "{wx}", cy: "{wy}", r: "1.6", fill: "rgb(245 158 11 / 0.55)" }
                }

                if !line.is_empty() {
                    polyline {
                        points: "{line}", fill: "none",
                        stroke: "rgb(56 132 255)", stroke_width: "2",
                        stroke_linejoin: "round", stroke_linecap: "round",
                    }
                }

                if let Some((hx , hy , label)) = hover() {
                    circle { cx: "{hx}", cy: "{hy}", r: "3.5", fill: "rgb(56 132 255)" }
                    text {
                        x: "{(hx).min(width - 120.0).max(PAD_L)}", y: "{(hy - 8.0).max(PAD_T + 8.0)}",
                        font_size: "11", font_weight: "600", fill: "currentColor",
                        "{label}"
                    }
                }

                if !has_points {
                    text {
                        x: "{width / 2.0}", y: "{h / 2.0}", text_anchor: "middle",
                        font_size: "13", fill: "currentColor", opacity: "0.5",
                        "No readings yet — connect a sensor to see your graph."
                    }
                }
            }
            div { class: "mt-1 flex items-center justify-between gap-2",
                p { class: "text-xs text-slate-500 dark:text-slate-400",
                    "Drag to pan · tap a point to read it"
                }
                button {
                    class: "{BTN_GHOST_SM}",
                    disabled: !has_points,
                    onclick: log_event,
                    "＋ Log event"
                }
            }
        }
    }
}
