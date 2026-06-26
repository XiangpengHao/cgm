//! The glucose chart — a self-contained, dependency-free SVG line chart with
//! green/amber/red background zones, hour gridlines, event markers, and an
//! Apple-Health-style touch interaction:
//!
//! * **scrub** — touch (or hover, on desktop) anywhere to move a selection: a
//!   full-height vertical rule, a zone-colored halo + hollow dot snapped to the
//!   nearest reading, and a floating value/time callout pinned to the top.
//! * **＋ Log event** — logs at the scrubbed time, or at the latest reading when
//!   nothing is selected; the button relabels to "＋ Log at HH:MM" while a point
//!   is selected.
//!
//! Time ranging is owned by the segmented window control (3h/6h/12h/24h/All), so
//! the chart body has no free-pan — every touch is an unambiguous read.
//!
//! The SVG is fluid (`width:100%` + a viewBox matching the measured container
//! width), so it fills its card on every viewport and keeps pointer hit-testing
//! 1:1; it re-measures on resize/rotation.

use crate::format::icon_for;
use crate::platform::SharedPlatform;
use crate::state::{AppState, EventDraft};
use crate::ui::BTN_GHOST_SM;
use cgm_core::glucose::record_valid;
use cgm_core::ranges::{band_edges, classify, Zone};
use cgm_core::stats::{convert, format_value};
use dioxus::prelude::*;

// Plot geometry. The SVG renders at the measured pixel width with viewBox ==
// width (and no preserveAspectRatio), so 1 user unit == 1 rendered px and text
// is never distorted. Height is responsive (see `chart_height`). The gutters are
// tiny so the trace runs nearly edge-to-edge (Apple Stocks/Health style); the
// y-axis is just the two self-labeling zone-boundary lines, so no reserved
// left column is needed.
const PAD_L: f64 = 6.0;
const PAD_R: f64 = 6.0;
const PAD_T: f64 = 14.0;
const PAD_B: f64 = 20.0;
const HOUR_MS: i64 = 3_600_000;

struct Pt {
    t: i64,
    v: f64,
    valid: bool,
    /// Raw stored mg/dL (zone classification is unit-blind).
    g: u16,
}

/// A pinned selection produced by scrubbing the chart.
#[derive(Clone, PartialEq)]
struct Sel {
    /// Plot x/y of the selected reading (user units == px).
    sx: f64,
    sy: f64,
    /// Display value (already unit-converted) and its unit label.
    val: String,
    unit: &'static str,
    /// Local HH:MM of the reading.
    time: String,
    /// Zone band color for the reading.
    color: &'static str,
    /// The reading's instant (epoch ms), for logging an event at this point.
    t_ms: i64,
}

/// Nearest valid reading to instant `t`, as `(t, display_value, raw_mgdl)`.
fn nearest(pts: &[(i64, f64, bool, u16)], t: i64) -> Option<(i64, f64, u16)> {
    pts.iter()
        .filter(|(_, _, valid, _)| *valid)
        .min_by_key(|(pt, _, _, _)| (pt - t).abs())
        .map(|(pt, v, _, g)| (*pt, *v, *g))
}

/// The zone band color for a raw mg/dL reading (matches the background bands).
fn zone_color(mgdl: u16) -> &'static str {
    match classify(mgdl) {
        Zone::InRange => "rgb(55 211 154)",
        Zone::Elevated => "rgb(240 169 43)",
        Zone::High => "rgb(255 93 93)",
    }
}

#[component]
pub fn GlucoseChart() -> Element {
    let state = use_context::<AppState>();
    let platform = use_context::<SharedPlatform>();
    let mut hover = use_signal(|| None::<Sel>);
    let mut event_draft = state.event_draft;
    let mut width_sig = state.chart_width;

    // A pinned selection caches its on-plot position + value against the current
    // domain and unit, so clear it whenever the visible window or unit changes —
    // otherwise the marker/callout would render against a stale domain until the
    // next scrub. (Per-minute data updates shift the domain imperceptibly.)
    use_effect(move || {
        let _ = state.window_hours.read();
        let _ = state.chart_view.read();
        let _ = state.settings.read().unit;
        hover.set(None);
    });

    let unit = state.settings.read().unit;
    let mgdl = unit.is_mgdl();
    // Surface color for the callout chip, the hollow selection dot, and the
    // y-label backings — matches the card (white / slate-900) so they read
    // cleanly over the colored zone bands in both themes.
    let dark = state.settings.read().theme == cgm_core::model::Theme::Dark;
    let surface = if dark { "#0f172a" } else { "#ffffff" };
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
    // `chart_view` is reset to None by every code path now that drag-to-pan is
    // gone (the window control owns ranging); kept for a possible future pan.
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
                g,
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

    // Dashed boundary lines at both edges (5.6 / 7.8), labeled inline at the
    // quiet left edge (the live data and callout live on the right). The third
    // tuple field is a backing-pill width that keeps the digits legible where
    // they cross a colored band.
    let fmt_edge = move |v: f64| -> String {
        if mgdl {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.1}")
        }
    };
    let edge_lines: Vec<(f64, String, f64)> = edges
        .iter()
        .filter(|&&v| v > ymin && v < ymax)
        .map(|&v| {
            let label = fmt_edge(v);
            let bw = label.len() as f64 * 5.5 + 6.0;
            (y(v), label, bw)
        })
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
    let move_pts: Vec<(i64, f64, bool, u16)> = pts
        .iter()
        .filter(|p| p.t >= t0 && p.t <= t1)
        .map(|p| (p.t, p.v, p.valid, p.g))
        .collect();
    // A closure that pins the readout at the nearest valid reading to `elem_x`.
    let pin_pts = move_pts.clone();
    let pin = move |elem_x: f64, mut hover: Signal<Option<Sel>>| {
        if let Some((bt, bv, g)) = nearest(&pin_pts, to_time(elem_x)) {
            let (val, unit_label) = if mgdl {
                (format!("{}", bv.round() as i64), "mg/dL")
            } else {
                (format!("{bv:.1}"), "mmol/L")
            };
            hover.set(Some(Sel {
                sx: x(bt),
                sy: y(bv),
                val,
                unit: unit_label,
                time: cgm_core::datetime::format_hm(bt, offset),
                color: zone_color(g),
                t_ms: bt,
            }));
        }
    };

    // Scrub is the only plot gesture: pointerdown/move pin the nearest reading,
    // leave/cancel clear it, and pointerup keeps it pinned so the user can lift
    // a finger and read (Apple Health behavior). On desktop, plain hover scrubs.
    let pin_down = pin.clone();
    let on_down = move |evt: PointerEvent| pin_down(evt.element_coordinates().x, hover);
    let on_move = move |evt: PointerEvent| pin(evt.element_coordinates().x, hover);
    let clear_sel = move |_: PointerEvent| hover.set(None);

    // The ＋ button logs at the scrubbed instant when a point is selected, else
    // at the latest reading. Centered (touch-friendly) popover either way.
    let log_event = move |_| {
        let t_ms = hover().map(|s| s.t_ms).unwrap_or(latest);
        event_draft.set(Some(EventDraft {
            t_ms,
            x: 0.0,
            y: 0.0,
            anchored: false,
        }));
    };

    let (helper, log_label) = match hover() {
        Some(s) => (
            format!("Reading {} · tap ＋ to log here", s.time),
            format!("＋ Log at {}", s.time),
        ),
        None => (
            "Slide to read · use the buttons above to change range · ＋ logs a note".to_string(),
            "＋ Log event".to_string(),
        ),
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
                onpointerleave: clear_sel,
                onpointercancel: clear_sel,

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
                for (ey , label , bw) in edge_lines.iter() {
                    line {
                        x1: "{PAD_L}", y1: "{ey}", x2: "{width - PAD_R}", y2: "{ey}",
                        stroke: "currentColor", stroke_width: "1", stroke_dasharray: "3 3", opacity: "0.35",
                    }
                    rect {
                        x: "{PAD_L}", y: "{ey - 11.0}", width: "{bw}", height: "11", rx: "2",
                        fill: "{surface}", opacity: "0.7",
                    }
                    text {
                        x: "{PAD_L + 3.0}", y: "{ey - 2.5}", font_size: "9",
                        fill: "currentColor", opacity: "0.7",
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

                // Selection scrubber + floating callout, drawn last so it is
                // never occluded by the trace.
                {
                    if let Some(s) = hover() {
                        let cw = ((s.val.len() as f64) * 9.0 + (s.unit.len() as f64) * 6.0 + 20.0)
                            .clamp(60.0, (width - 2.0 * PAD_L).max(60.0));
                        let lo = PAD_L;
                        let hi = (width - PAD_R - cw).max(lo);
                        let cx = (s.sx - cw / 2.0).clamp(lo, hi);
                        // Pin the callout to the top, but flip it below the point
                        // when the reading sits high in the plot so the chip never
                        // hides the very dot it describes.
                        let cy = if s.sy < PAD_T + 48.0 {
                            (s.sy + 14.0).min(h - PAD_B - 34.0)
                        } else {
                            PAD_T
                        };
                        rsx! {
                            line {
                                x1: "{s.sx}", y1: "{PAD_T}", x2: "{s.sx}", y2: "{h - PAD_B}",
                                stroke: "currentColor", stroke_width: "1", opacity: "0.25",
                            }
                            circle { cx: "{s.sx}", cy: "{s.sy}", r: "11", fill: "{s.color}", opacity: "0.15" }
                            circle {
                                cx: "{s.sx}", cy: "{s.sy}", r: "5",
                                fill: "{surface}", stroke: "{s.color}", stroke_width: "3",
                            }
                            rect { x: "{cx}", y: "{cy}", width: "{cw}", height: "34", rx: "7", fill: "{surface}" }
                            rect {
                                x: "{cx}", y: "{cy}", width: "{cw}", height: "34", rx: "7",
                                fill: "none", stroke: "currentColor", stroke_width: "1", opacity: "0.12",
                            }
                            text {
                                x: "{cx + 8.0}", y: "{cy + 16.0}", font_size: "15", font_weight: "700",
                                fill: "{s.color}", style: "font-variant-numeric: tabular-nums;",
                                "{s.val}"
                                tspan { font_size: "9", font_weight: "500", opacity: "0.6", fill: "currentColor", " {s.unit}" }
                            }
                            text {
                                x: "{cx + 8.0}", y: "{cy + 28.0}", font_size: "10", opacity: "0.6",
                                fill: "currentColor", style: "font-variant-numeric: tabular-nums;", "{s.time}"
                            }
                        }
                    } else {
                        rsx! {}
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
                p { class: "text-xs text-slate-500 dark:text-slate-400", "{helper}" }
                button {
                    class: "{BTN_GHOST_SM}",
                    disabled: !has_points,
                    onclick: log_event,
                    "{log_label}"
                }
            }
        }
    }
}
