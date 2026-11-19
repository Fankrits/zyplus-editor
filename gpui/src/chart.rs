//! Native renderer for ```chart fenced blocks (Chart.js config JSON).
//!
//! The web build hands the block to Chart.js. Here the same JSON is parsed into a
//! `ChartSpec`, laid out as a flat list of primitives (`build_scene`, pure and unit
//! tested), and painted with gpui's canvas: quads for bars, gridlines, dots and
//! legend swatches, paths for lines and pie/doughnut wedges, shaped lines for text.

use std::f64::consts::{FRAC_PI_2, TAU};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, BorderStyle, Bounds, FontWeight, Hsla, PathBuilder, Pixels, Point, Rgba,
    SharedString, TextRun, Window, canvas, div, point, px, quad, rgb, size, transparent_black,
};
use serde_json::Value;

use crate::theme::Theme;

/// Height of a rendered chart block, in logical pixels. The width fills the container.
pub const CHART_HEIGHT: f32 = 300.0;

/// Series colours used when a dataset does not set its own, same order as the web.
pub const PALETTE_HEX: [u32; 6] = [0x006fee, 0x7828c8, 0x17c964, 0xf5a524, 0xf31260, 0x06b6d4];

const PAD: f32 = 10.0;
const TICK_FONT: f32 = 12.0;
const TITLE_FONT: f32 = 14.0;
const LABEL_LINE: f32 = 16.0;
const TITLE_ROW: f32 = 22.0;
const LEGEND_ROW: f32 = 18.0;
const X_LABEL_H: f32 = 18.0;
const TARGET_TICKS: usize = 6;

pub fn palette_color(index: usize) -> Rgba {
    rgb(PALETTE_HEX[index % PALETTE_HEX.len()])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
    Doughnut,
    Scatter,
}

#[derive(Clone, Debug, Default)]
pub struct Dataset {
    pub label: String,
    /// Category charts: one entry per label. `None` is a gap (JSON `null`).
    pub values: Vec<Option<f64>>,
    /// Scatter charts only.
    pub points: Vec<(f64, f64)>,
    /// Dataset-level colour (`backgroundColor`, or `borderColor` for lines).
    pub color: Option<Rgba>,
    /// Per-element colours when the JSON gives an array. `None` entries fall back.
    pub colors: Vec<Option<Rgba>>,
}

#[derive(Clone, Debug)]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub title: Option<String>,
    pub labels: Vec<String>,
    pub datasets: Vec<Dataset>,
    /// `options.plugins.legend.display` (default true).
    pub legend: bool,
    /// `options.scales.{x,y}.stacked` on either axis.
    pub stacked: bool,
    /// `options.scales.y.beginAtZero` (bars always start at zero).
    pub begin_at_zero: bool,
    pub y_min: Option<f64>,
    pub y_max: Option<f64>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse the body of a ```chart block. Errors are short, readable messages for the
/// error box; this never panics.
pub fn parse(src: &str) -> Result<ChartSpec, String> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return Err("The chart block is empty.".to_string());
    }
    let root: Value = serde_json::from_str(trimmed).map_err(|e| json_error(&e))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "The chart must be a JSON object with \"type\" and \"data\".".to_string())?;

    let kind = parse_kind(obj.get("type"))?;

    let data = obj
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "Missing \"data\" object.".to_string())?;
    let labels = parse_labels(data.get("labels"))?;
    let raw_sets = match data.get("datasets") {
        Some(Value::Array(items)) => items,
        _ => return Err("\"data.datasets\" must be an array.".to_string()),
    };
    let mut datasets = Vec::with_capacity(raw_sets.len());
    for (index, raw) in raw_sets.iter().enumerate() {
        datasets.push(parse_dataset(raw, index, kind)?);
    }

    let options = obj.get("options").and_then(Value::as_object);
    let plugins = options
        .and_then(|o| o.get("plugins"))
        .and_then(Value::as_object);

    let legend = plugins
        .and_then(|p| p.get("legend"))
        .and_then(Value::as_object)
        .and_then(|l| l.get("display"))
        .and_then(Value::as_bool)
        .unwrap_or(true);

    // Chart.js only shows a title when `plugins.title.display` is true.
    let title = plugins
        .and_then(|p| p.get("title"))
        .and_then(Value::as_object)
        .filter(|t| t.get("display").and_then(Value::as_bool) == Some(true))
        .and_then(|t| title_text(t.get("text")));

    let scales = options
        .and_then(|o| o.get("scales"))
        .and_then(Value::as_object);
    let scale = |name: &str| {
        scales
            .and_then(|s| s.get(name))
            .and_then(Value::as_object)
    };
    let stacked = [scale("x"), scale("y")]
        .iter()
        .flatten()
        .any(|s| s.get("stacked").and_then(Value::as_bool) == Some(true));
    let y_scale = scale("y");
    let y_min = y_scale.and_then(|s| s.get("min")).and_then(number_value);
    let y_max = y_scale.and_then(|s| s.get("max")).and_then(number_value);
    let begin_at_zero = y_scale
        .and_then(|s| s.get("beginAtZero"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(ChartSpec {
        kind,
        title,
        labels,
        datasets,
        legend,
        stacked,
        begin_at_zero,
        y_min,
        y_max,
    })
}

fn json_error(e: &serde_json::Error) -> String {
    // serde's Display ends with " at line N column M"; keep the detail and report the
    // position in a fixed form.
    let full = e.to_string();
    let detail = match full.rfind(" at line ") {
        Some(i) => &full[..i],
        None => full.as_str(),
    };
    format!(
        "Invalid JSON at line {}, column {}: {}",
        e.line(),
        e.column(),
        detail
    )
}

fn parse_kind(value: Option<&Value>) -> Result<ChartKind, String> {
    match value {
        None | Some(Value::Null) => {
            Err("Missing \"type\" (expected bar, line, pie, doughnut or scatter).".to_string())
        }
        Some(Value::String(name)) => match name.as_str() {
            "bar" => Ok(ChartKind::Bar),
            "line" => Ok(ChartKind::Line),
            "pie" => Ok(ChartKind::Pie),
            "doughnut" => Ok(ChartKind::Doughnut),
            "scatter" => Ok(ChartKind::Scatter),
            other => Err(format!(
                "Unsupported chart type \"{other}\" (expected bar, line, pie, doughnut or scatter)."
            )),
        },
        Some(_) => Err("\"type\" must be a string (bar, line, pie, doughnut or scatter).".to_string()),
    }
}

fn parse_labels(value: Option<&Value>) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.iter().map(label_text).collect()),
        Some(_) => Err("\"data.labels\" must be an array.".to_string()),
    }
}

fn label_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        // Chart.js multi-line labels are arrays; this renderer shows them on one line.
        Value::Array(items) => items.iter().map(label_text).collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    }
}

fn title_text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(label_text).collect::<Vec<_>>().join(" "),
        _ => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn parse_dataset(raw: &Value, index: usize, kind: ChartKind) -> Result<Dataset, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| format!("Dataset {} must be an object.", index + 1))?;
    let label = obj
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Dataset {}", index + 1));
    let items = match obj.get("data") {
        Some(Value::Array(items)) => items,
        _ => return Err(format!("Dataset \"{label}\" needs a \"data\" array.")),
    };

    let mut dataset = Dataset {
        label: label.clone(),
        ..Dataset::default()
    };

    if kind == ChartKind::Scatter {
        for (j, item) in items.iter().enumerate() {
            let point = match item {
                Value::Null => continue,
                Value::Object(p) => match (p.get("x").and_then(number_value), p.get("y").and_then(number_value)) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => None,
                },
                Value::Array(p) if p.len() == 2 => match (number_value(&p[0]), number_value(&p[1])) {
                    (Some(x), Some(y)) => Some((x, y)),
                    _ => None,
                },
                _ => None,
            };
            match point {
                Some(p) => dataset.points.push(p),
                None => {
                    return Err(format!(
                        "Dataset \"{label}\": point {} must be {{\"x\": number, \"y\": number}}.",
                        j + 1
                    ));
                }
            }
        }
    } else {
        for (j, item) in items.iter().enumerate() {
            let value = match item {
                Value::Null => None,
                other => match number_value(other) {
                    Some(v) => Some(v),
                    None => {
                        return Err(format!(
                            "Dataset \"{label}\": value {} at index {} is not a number.",
                            other,
                            j + 1
                        ));
                    }
                },
            };
            dataset.values.push(value);
        }
    }

    // Line charts take the stroke colour first; bars and pies take the fill colour.
    let (primary, secondary) = if matches!(kind, ChartKind::Line | ChartKind::Scatter) {
        (obj.get("borderColor"), obj.get("backgroundColor"))
    } else {
        (obj.get("backgroundColor"), obj.get("borderColor"))
    };
    let colour_value = primary.or(secondary);
    match colour_value {
        Some(Value::String(s)) => dataset.color = parse_color(s),
        Some(Value::Array(items)) => {
            dataset.colors = items
                .iter()
                .map(|c| c.as_str().and_then(parse_color))
                .collect();
        }
        _ => {}
    }

    Ok(dataset)
}

/// A finite number from a JSON number, or from a numeric string (Chart.js accepts both).
fn number_value(value: &Value) -> Option<f64> {
    let v = match value {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// Parse a CSS-style colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`.
pub fn parse_color(input: &str) -> Option<Rgba> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<Vec<u8>>>()?;
        let (r, g, b, a) = match digits.len() {
            3 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, 255),
            4 => (digits[0] * 17, digits[1] * 17, digits[2] * 17, digits[3] * 17),
            6 => (
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                255,
            ),
            8 => (
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                digits[6] * 16 + digits[7],
            ),
            _ => return None,
        };
        return Some(Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        });
    }

    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<f64> = inner
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok())
        .collect::<Option<Vec<f64>>>()?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    Some(Rgba {
        r: (parts[0].clamp(0.0, 255.0) / 255.0) as f32,
        g: (parts[1].clamp(0.0, 255.0) / 255.0) as f32,
        b: (parts[2].clamp(0.0, 255.0) / 255.0) as f32,
        a: parts.get(3).copied().unwrap_or(1.0).clamp(0.0, 1.0) as f32,
    })
}

// ---------------------------------------------------------------------------
// Axis maths (pure)
// ---------------------------------------------------------------------------

fn nice_num(range: f64, round: bool) -> f64 {
    let exp = range.log10().floor();
    let fraction = range / 10f64.powf(exp);
    let nice = if round {
        if fraction < 1.5 {
            1.0
        } else if fraction < 3.0 {
            2.0
        } else if fraction < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if fraction <= 1.0 {
        1.0
    } else if fraction <= 2.0 {
        2.0
    } else if fraction <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * 10f64.powf(exp)
}

/// Round-number tick values covering `[min, max]`, aiming for about `count` intervals.
/// Always returns at least two ticks, in ascending order, with `last > first`.
pub fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    let count = count.max(2);
    let (mut lo, mut hi) = if min <= max { (min, max) } else { (max, min) };
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if hi - lo < 1e-12 {
        if lo.abs() < 1e-12 {
            hi = 1.0;
        } else {
            let pad = lo.abs() * 0.1;
            lo -= pad;
            hi += pad;
        }
    }
    let range = nice_num(hi - lo, false);
    let step = nice_num(range / (count - 1) as f64, true);
    let k0 = (lo / step).floor() as i64;
    let mut k1 = (hi / step).ceil() as i64;
    if k1 <= k0 {
        k1 = k0 + 1;
    }
    let decimals = (-step.log10().floor()).max(0.0) as i32 + 3;
    let factor = 10f64.powi(decimals);
    (k0..=k1)
        .map(|k| ((k as f64 * step) * factor).round() / factor)
        .collect()
}

/// Tick label text: up to four decimals, no trailing zeros.
pub fn format_tick(value: f64) -> String {
    let s = format!("{value:.4}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Debug)]
struct Scale {
    min: f64,
    max: f64,
    ticks: Vec<f64>,
}

impl Scale {
    fn from_range(lo: f64, hi: f64, fixed_min: Option<f64>, fixed_max: Option<f64>) -> Self {
        let mut lo = fixed_min.unwrap_or(lo);
        let mut hi = fixed_max.unwrap_or(hi);
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let ticks = nice_ticks(lo, hi, TARGET_TICKS);
        let min = ticks.first().copied().unwrap_or(0.0);
        let max = ticks.last().copied().unwrap_or(1.0);
        Self { min, max, ticks }
    }

    /// Position of `value` along the scale as a fraction 0..1.
    fn frac(&self, value: f64) -> f32 {
        ((value - self.min) / (self.max - self.min)) as f32
    }

    /// Vertical pixel for `value`: the max sits at `top`, the min at `bottom`.
    fn px_y(&self, value: f64, top: f32, bottom: f32) -> f32 {
        bottom - self.frac(value) * (bottom - top)
    }

    /// Horizontal pixel for `value`: the min sits at `left`, the max at `right`.
    fn px_x(&self, value: f64, left: f32, right: f32) -> f32 {
        left + self.frac(value) * (right - left)
    }
}

// ---------------------------------------------------------------------------
// Scene (pure layout)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A paint primitive in chart-local coordinates (origin at the top-left of the block).
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: Rgba,
    },
    Polygon {
        points: Vec<(f32, f32)>,
        color: Rgba,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        width: f32,
        color: Rgba,
    },
    /// `y` is the top of the line box; `x` is the anchor for `align`.
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        align: Align,
        color: Rgba,
    },
}

#[derive(Clone, Copy, Debug)]
struct Area {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

/// Lay out a chart for a canvas of `width` x `height` pixels.
pub fn build_scene(spec: &ChartSpec, width: f32, height: f32, fg: Rgba, grid: Rgba) -> Vec<Prim> {
    let mut out = Vec::new();
    let mut top = PAD;

    if let Some(title) = &spec.title {
        out.push(Prim::Text {
            x: width / 2.0,
            y: top,
            text: title.clone(),
            size: TITLE_FONT,
            align: Align::Center,
            color: fg,
        });
        top += TITLE_ROW;
    }

    if spec.legend {
        let items = legend_items(spec);
        if !items.is_empty() {
            top = push_legend(&mut out, &items, top, width, fg);
        }
    }

    let area = Area {
        left: PAD,
        top,
        right: width - PAD,
        bottom: height - PAD,
    };
    match spec.kind {
        ChartKind::Pie | ChartKind::Doughnut => draw_pie(&mut out, spec, area, fg),
        ChartKind::Scatter => draw_scatter(&mut out, spec, area, fg, grid),
        ChartKind::Bar | ChartKind::Line => draw_category(&mut out, spec, area, fg, grid),
    }
    out
}

fn dataset_color(ds: &Dataset, index: usize) -> Rgba {
    ds.color.unwrap_or_else(|| palette_color(index))
}

/// Colour of one bar or line point, from the element array first, then the dataset.
fn element_color(ds: &Dataset, element: usize, index: usize) -> Rgba {
    ds.colors
        .get(element)
        .copied()
        .flatten()
        .or(ds.color)
        .unwrap_or_else(|| palette_color(index))
}

/// Pie and doughnut colour per slice, since the slices are the series.
fn slice_color(ds: &Dataset, slice: usize) -> Rgba {
    element_color(ds, slice, slice)
}

fn est_width(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.55
}

fn legend_items(spec: &ChartSpec) -> Vec<(String, Rgba)> {
    match spec.kind {
        ChartKind::Pie | ChartKind::Doughnut => {
            let Some(ds) = spec.datasets.first() else {
                return Vec::new();
            };
            let n = spec.labels.len().max(ds.values.len());
            (0..n)
                .map(|i| {
                    (
                        spec.labels.get(i).cloned().unwrap_or_default(),
                        slice_color(ds, i),
                    )
                })
                .collect()
        }
        _ => spec
            .datasets
            .iter()
            .enumerate()
            .map(|(i, ds)| (ds.label.clone(), dataset_color(ds, i)))
            .collect(),
    }
}

/// Lay the legend out in wrapping rows. Returns the y just below the legend.
fn push_legend(out: &mut Vec<Prim>, items: &[(String, Rgba)], top: f32, width: f32, fg: Rgba) -> f32 {
    let mut x = PAD;
    let mut y = top;
    for (label, color) in items {
        let item_w = 10.0 + 6.0 + est_width(label, TICK_FONT) + 16.0;
        if x > PAD && x + item_w > width - PAD {
            x = PAD;
            y += LEGEND_ROW;
        }
        out.push(Prim::Rect {
            x,
            y: y + 3.0,
            w: 10.0,
            h: 10.0,
            radius: 2.0,
            color: *color,
        });
        if !label.is_empty() {
            out.push(Prim::Text {
                x: x + 16.0,
                y,
                text: label.clone(),
                size: TICK_FONT,
                align: Align::Left,
                color: fg,
            });
        }
        x += item_w;
    }
    y + LEGEND_ROW + 4.0
}

fn draw_no_data(out: &mut Vec<Prim>, area: Area, fg: Rgba) {
    out.push(Prim::Text {
        x: (area.left + area.right) / 2.0,
        y: (area.top + area.bottom) / 2.0 - LABEL_LINE / 2.0,
        text: "No data".to_string(),
        size: TICK_FONT,
        align: Align::Center,
        color: fg,
    });
}

/// Plot rectangle `(left, top, right, bottom)` for a cartesian chart with a y scale.
fn cartesian_plot(area: Area, y: &Scale) -> (f32, f32, f32, f32) {
    let label_w = y
        .ticks
        .iter()
        .map(|t| est_width(&format_tick(*t), TICK_FONT))
        .fold(0.0_f32, f32::max);
    (
        area.left + label_w + 8.0,
        area.top + 6.0,
        area.right - 8.0,
        area.bottom - X_LABEL_H,
    )
}

fn draw_y_axis(out: &mut Vec<Prim>, y: &Scale, pl: f32, pr: f32, pt: f32, pb: f32, fg: Rgba, grid: Rgba) {
    for tick in &y.ticks {
        let py = y.px_y(*tick, pt, pb);
        out.push(Prim::Rect {
            x: pl,
            y: py,
            w: (pr - pl).max(0.0),
            h: 1.0,
            radius: 0.0,
            color: grid,
        });
        out.push(Prim::Text {
            x: pl - 6.0,
            y: py - LABEL_LINE / 2.0,
            text: format_tick(*tick),
            size: TICK_FONT,
            align: Align::Right,
            color: fg,
        });
    }
}

fn category_count(spec: &ChartSpec) -> usize {
    spec.datasets
        .iter()
        .map(|d| d.values.len())
        .max()
        .unwrap_or(0)
        .max(spec.labels.len())
}

/// Per dataset, per category: `(base, top)` in data units after stacking, or `None`
/// for a gap. Unstacked bars run from zero; stacked bars stack positives and negatives
/// separately; stacked lines accumulate.
fn element_extents(spec: &ChartSpec, n: usize) -> Vec<Vec<Option<(f64, f64)>>> {
    let mut pos = vec![0.0_f64; n];
    let mut neg = vec![0.0_f64; n];
    let mut cum = vec![0.0_f64; n];
    let mut out = Vec::with_capacity(spec.datasets.len());
    for ds in &spec.datasets {
        let mut row: Vec<Option<(f64, f64)>> = vec![None; n];
        for (j, slot) in row.iter_mut().enumerate() {
            let Some(v) = ds.values.get(j).copied().flatten() else {
                continue;
            };
            *slot = if !spec.stacked {
                Some((0.0, v))
            } else if spec.kind == ChartKind::Line {
                let before = cum[j];
                cum[j] += v;
                Some((before, cum[j]))
            } else if v >= 0.0 {
                let base = pos[j];
                pos[j] += v;
                Some((base, pos[j]))
            } else {
                let base = neg[j];
                neg[j] += v;
                Some((base, neg[j]))
            };
        }
        out.push(row);
    }
    out
}

fn category_range(spec: &ChartSpec, ext: &[Vec<Option<(f64, f64)>>]) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for row in ext {
        for &(base, top) in row.iter().flatten() {
            lo = lo.min(base.min(top));
            hi = hi.max(base.max(top));
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if spec.kind == ChartKind::Bar || spec.begin_at_zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    Some((lo, hi))
}

fn draw_category(out: &mut Vec<Prim>, spec: &ChartSpec, area: Area, fg: Rgba, grid: Rgba) {
    let n = category_count(spec);
    let ext = element_extents(spec, n);
    let Some((lo, hi)) = category_range(spec, &ext) else {
        draw_no_data(out, area, fg);
        return;
    };
    let y = Scale::from_range(lo, hi, spec.y_min, spec.y_max);
    let (pl, pt, pr, pb) = cartesian_plot(area, &y);
    draw_y_axis(out, &y, pl, pr, pt, pb, fg, grid);

    let cat_w = (pr - pl) / n as f32;
    let cx = |j: usize| -> f32 {
        match spec.kind {
            ChartKind::Line if n > 1 => pl + (pr - pl) * j as f32 / (n - 1) as f32,
            ChartKind::Line => (pl + pr) / 2.0,
            _ => pl + (j as f32 + 0.5) * cat_w,
        }
    };

    for j in 0..n {
        if let Some(label) = spec.labels.get(j).filter(|l| !l.is_empty()) {
            out.push(Prim::Text {
                x: cx(j),
                y: pb + 4.0,
                text: label.clone(),
                size: TICK_FONT,
                align: Align::Center,
                color: fg,
            });
        }
    }
    out.push(Prim::Rect {
        x: pl,
        y: pb,
        w: (pr - pl).max(0.0),
        h: 1.0,
        radius: 0.0,
        color: grid,
    });

    if spec.kind == ChartKind::Bar {
        let series = spec.datasets.len().max(1) as f32;
        for (i, ds) in spec.datasets.iter().enumerate() {
            for j in 0..n {
                let Some((base, top)) = ext[i][j] else {
                    continue;
                };
                let y1 = y.px_y(base, pt, pb);
                let y2 = y.px_y(top, pt, pb);
                let (x0, w) = if spec.stacked {
                    let w = cat_w * 0.6;
                    (cx(j) - w / 2.0, w)
                } else {
                    let group = cat_w * 0.8;
                    let slot = group / series;
                    (cx(j) - group / 2.0 + i as f32 * slot, (slot - 1.0).max(1.0))
                };
                out.push(Prim::Rect {
                    x: x0,
                    y: y1.min(y2),
                    w,
                    h: (y1 - y2).abs(),
                    radius: 0.0,
                    color: element_color(ds, j, i),
                });
            }
        }
    }

    if spec.kind == ChartKind::Line {
        for (i, ds) in spec.datasets.iter().enumerate() {
            let color = dataset_color(ds, i);
            let mut segment: Vec<(f32, f32)> = Vec::new();
            let mut dots: Vec<Prim> = Vec::new();
            for j in 0..n {
                match ext[i][j] {
                    Some((_, top)) => {
                        let p = (cx(j), y.px_y(top, pt, pb));
                        segment.push(p);
                        dots.push(dot(p, 3.0, color));
                    }
                    None => flush_segment(out, &mut segment, color),
                }
            }
            flush_segment(out, &mut segment, color);
            out.extend(dots);
        }
    }
}

fn flush_segment(out: &mut Vec<Prim>, segment: &mut Vec<(f32, f32)>, color: Rgba) {
    if segment.len() >= 2 {
        out.push(Prim::Polyline {
            points: std::mem::take(segment),
            width: 2.0,
            color,
        });
    } else {
        segment.clear();
    }
}

fn dot(center: (f32, f32), radius: f32, color: Rgba) -> Prim {
    Prim::Rect {
        x: center.0 - radius,
        y: center.1 - radius,
        w: radius * 2.0,
        h: radius * 2.0,
        radius,
        color,
    }
}

fn draw_scatter(out: &mut Vec<Prim>, spec: &ChartSpec, area: Area, fg: Rgba, grid: Rgba) {
    let all: Vec<(f64, f64)> = spec
        .datasets
        .iter()
        .flat_map(|d| d.points.iter().copied())
        .collect();
    if all.is_empty() {
        draw_no_data(out, area, fg);
        return;
    }
    let (mut x_lo, mut x_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y_lo, mut y_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in &all {
        x_lo = x_lo.min(x);
        x_hi = x_hi.max(x);
        y_lo = y_lo.min(y);
        y_hi = y_hi.max(y);
    }
    let xs = Scale::from_range(x_lo, x_hi, None, None);
    let ys = Scale::from_range(y_lo, y_hi, spec.y_min, spec.y_max);
    let (pl, pt, pr, pb) = cartesian_plot(area, &ys);
    draw_y_axis(out, &ys, pl, pr, pt, pb, fg, grid);

    for tick in &xs.ticks {
        let px_x = xs.px_x(*tick, pl, pr);
        out.push(Prim::Rect {
            x: px_x,
            y: pt,
            w: 1.0,
            h: (pb - pt).max(0.0),
            radius: 0.0,
            color: grid,
        });
        out.push(Prim::Text {
            x: px_x,
            y: pb + 4.0,
            text: format_tick(*tick),
            size: TICK_FONT,
            align: Align::Center,
            color: fg,
        });
    }
    out.push(Prim::Rect {
        x: pl,
        y: pb,
        w: (pr - pl).max(0.0),
        h: 1.0,
        radius: 0.0,
        color: grid,
    });

    for (i, ds) in spec.datasets.iter().enumerate() {
        let color = dataset_color(ds, i);
        for &(x, y_val) in &ds.points {
            let p = (xs.px_x(x, pl, pr), ys.px_y(y_val, pt, pb));
            out.push(dot(p, 4.0, color));
        }
    }
}

/// Points along an arc from `a0` sweeping `sweep` radians, about 3 degrees per step.
fn arc_points(cx: f32, cy: f32, radius: f32, a0: f64, sweep: f64) -> Vec<(f32, f32)> {
    let steps = ((sweep.abs().to_degrees() / 3.0).ceil() as usize).clamp(2, 720);
    (0..=steps)
        .map(|k| {
            let a = a0 + sweep * k as f64 / steps as f64;
            (cx + radius * a.cos() as f32, cy + radius * a.sin() as f32)
        })
        .collect()
}

fn draw_pie(out: &mut Vec<Prim>, spec: &ChartSpec, area: Area, fg: Rgba) {
    let cx = (area.left + area.right) / 2.0;
    let cy = (area.top + area.bottom) / 2.0;
    let radius = ((area.right - area.left).min(area.bottom - area.top) / 2.0 - 4.0).max(1.0);

    let Some(ds) = spec.datasets.first() else {
        draw_no_data(out, area, fg);
        return;
    };
    let values: Vec<f64> = ds.values.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        draw_no_data(out, area, fg);
        return;
    }

    let inner = if spec.kind == ChartKind::Doughnut {
        radius * 0.55
    } else {
        0.0
    };
    let mut start = -FRAC_PI_2;
    for (i, value) in values.iter().enumerate() {
        if *value <= 0.0 {
            continue;
        }
        let sweep = value / total * TAU;
        let points = if inner > 0.0 {
            let mut pts = arc_points(cx, cy, radius, start, sweep);
            pts.extend(arc_points(cx, cy, inner, start + sweep, -sweep));
            pts
        } else {
            let mut pts = vec![(cx, cy)];
            pts.extend(arc_points(cx, cy, radius, start, sweep));
            pts
        };
        out.push(Prim::Polygon {
            points,
            color: slice_color(ds, i),
        });
        start += sweep;
    }
}

// ---------------------------------------------------------------------------
// Painting (gpui)
// ---------------------------------------------------------------------------

/// A chart block as an element: fixed `CHART_HEIGHT`, width fills the container.
pub fn render_chart(spec: &ChartSpec, theme: &Theme) -> AnyElement {
    let spec = spec.clone();
    let fg = theme.fg;
    let grid = theme.border;
    div()
        .w_full()
        .h(px(CHART_HEIGHT))
        .my(px(8.0))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, _, window, cx| {
                    let width = f32::from(bounds.size.width);
                    let scene = build_scene(&spec, width, CHART_HEIGHT, fg, grid);
                    paint_scene(&scene, bounds.origin, window, cx);
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// Parse a chart block and render it; a parse error becomes a readable error box.
pub fn render(src: &str, theme: &Theme) -> AnyElement {
    match parse(src) {
        Ok(spec) => render_chart(&spec, theme),
        Err(message) => render_error(&message, theme),
    }
}

pub fn render_error(message: &str, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .p(px(10.0))
        .my(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.danger)
        .bg(theme.surface)
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_color(theme.danger)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.0))
                .child("Chart error"),
        )
        .child(
            div()
                .text_color(theme.muted)
                .text_size(px(12.0))
                .child(message.to_string()),
        )
        .into_any_element()
}

/// A zoom and a pan applied to a scene when it is painted: a scene point
/// `(x, y)` lands at `(x * zoom + dx, y * zoom + dy)` from the paint origin.
/// Everything is drawn from vectors, so a zoomed chart stays sharp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub zoom: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Xf {
    pub const IDENTITY: Xf = Xf { zoom: 1.0, dx: 0.0, dy: 0.0 };
}

fn to_point(origin: Point<Pixels>, xf: Xf, x: f32, y: f32) -> Point<Pixels> {
    point(origin.x + px(x * xf.zoom + xf.dx), origin.y + px(y * xf.zoom + xf.dy))
}

fn paint_scene(scene: &[Prim], origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
    paint_scene_xf(scene, origin, Xf::IDENTITY, window, cx);
}

/// Paints `scene` with `xf` applied. Clip it with a content mask (an
/// `overflow_hidden` parent) when it can leave its box.
pub fn paint_scene_xf(scene: &[Prim], origin: Point<Pixels>, xf: Xf, window: &mut Window, cx: &mut App) {
    let z = xf.zoom;
    let style = window.text_style();
    let font = style.font();
    for prim in scene {
        match prim {
            Prim::Rect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => {
                let bounds = Bounds::new(to_point(origin, xf, *x, *y), size(px(*w * z), px(*h * z)));
                window.paint_quad(quad(
                    bounds,
                    px(*radius * z),
                    *color,
                    px(0.0),
                    transparent_black(),
                    BorderStyle::default(),
                ));
            }
            Prim::Polygon { points, color } => {
                if points.len() < 3 {
                    continue;
                }
                let mut builder = PathBuilder::fill();
                builder.move_to(to_point(origin, xf, points[0].0, points[0].1));
                for &(x, y) in &points[1..] {
                    builder.line_to(to_point(origin, xf, x, y));
                }
                builder.close();
                if let Ok(path) = builder.build() {
                    window.paint_path(path, *color);
                }
            }
            Prim::Polyline {
                points,
                width,
                color,
            } => {
                if points.len() < 2 {
                    continue;
                }
                let mut builder = PathBuilder::stroke(px(*width * z));
                builder.move_to(to_point(origin, xf, points[0].0, points[0].1));
                for &(x, y) in &points[1..] {
                    builder.line_to(to_point(origin, xf, x, y));
                }
                if let Ok(path) = builder.build() {
                    window.paint_path(path, *color);
                }
            }
            Prim::Text {
                x,
                y,
                text,
                size: font_size,
                align,
                color,
            } => {
                let text: SharedString = text.clone().into();
                let runs = [TextRun {
                    len: text.len(),
                    font: font.clone(),
                    color: Hsla::from(*color),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
                let shaped = window
                    .text_system()
                    .shape_line(text, px(*font_size * z), &runs, None);
                let width = f32::from(shaped.width);
                let dx = match align {
                    Align::Left => 0.0,
                    Align::Center => -width / 2.0,
                    Align::Right => -width,
                };
                let at = to_point(origin, xf, *x, *y);
                let _ = shaped.paint(point(at.x + px(dx), at.y), px(LABEL_LINE * z), window, cx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (pure: no gpui window needed)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const FG: fn() -> Rgba = || rgb(0x111111);
    const GRID: fn() -> Rgba = || rgb(0xdddddd);

    fn scene_of(src: &str, w: f32, h: f32) -> Vec<Prim> {
        let spec = parse(src).expect("valid chart");
        build_scene(&spec, w, h, FG(), GRID())
    }

    /// Count of rects with the given corner radius and colour (bars use radius 0,
    /// scatter dots use radius 4, legend swatches use radius 2).
    fn count_rects_radius(scene: &[Prim], color: Rgba, radius: f32) -> usize {
        scene
            .iter()
            .filter(|p| matches!(p, Prim::Rect { color: c, radius: r, .. } if *c == color && *r == radius))
            .count()
    }

    #[test]
    fn nice_ticks_round_numbers() {
        assert_eq!(nice_ticks(0.0, 10.0, 5), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        let t = nice_ticks(0.0, 97.0, 5);
        assert_eq!(t.first().copied(), Some(0.0));
        assert_eq!(t.last().copied(), Some(100.0));
        assert!(t.windows(2).all(|w| (w[1] - w[0] - 20.0).abs() < 1e-9));
        let unit = nice_ticks(0.0, 1.0, 5);
        assert_eq!(unit.len(), 6);
        assert!((unit[1] - 0.2).abs() < 1e-12);
        assert_eq!(unit[5], 1.0);
    }

    #[test]
    fn nice_ticks_cover_negative_and_degenerate_ranges() {
        let t = nice_ticks(-3.0, 7.0, 5);
        assert!(t[0] <= -3.0 && *t.last().unwrap() >= 7.0);
        assert_eq!(nice_ticks(0.0, 0.0, 5).first().copied(), Some(0.0));
        assert_eq!(nice_ticks(0.0, 0.0, 5).last().copied(), Some(1.0));
        let flat = nice_ticks(5.0, 5.0, 5);
        assert!(flat.len() >= 2 && flat[0] < 5.0 && *flat.last().unwrap() > 5.0);
        let reversed = nice_ticks(10.0, 0.0, 5);
        assert_eq!(reversed.first().copied(), Some(0.0));
        assert_eq!(reversed.last().copied(), Some(10.0));
    }

    #[test]
    fn format_tick_trims_zeros() {
        assert_eq!(format_tick(100.0), "100");
        assert_eq!(format_tick(0.2), "0.2");
        assert_eq!(format_tick(-0.0), "0");
        assert_eq!(format_tick(1.25), "1.25");
    }

    #[test]
    fn parses_each_kind() {
        let bar = parse(r#"{"type":"bar","data":{"labels":["a","b"],"datasets":[{"label":"x","data":[1,2]}]}}"#).unwrap();
        assert_eq!(bar.kind, ChartKind::Bar);
        assert_eq!(bar.labels, vec!["a", "b"]);
        assert_eq!(bar.datasets[0].values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(bar.datasets[0].color, None);
        assert!(bar.legend);
        assert!(bar.title.is_none());

        for (ty, kind) in [
            ("line", ChartKind::Line),
            ("pie", ChartKind::Pie),
            ("doughnut", ChartKind::Doughnut),
        ] {
            let src = format!(r#"{{"type":"{ty}","data":{{"datasets":[{{"data":[3]}}]}}}}"#);
            assert_eq!(parse(&src).unwrap().kind, kind);
        }
    }

    #[test]
    fn scatter_accepts_objects_and_pairs() {
        let spec = parse(
            r#"{"type":"scatter","data":{"datasets":[{"label":"p","data":[{"x":1,"y":2},[3,4],null]}]}}"#,
        )
        .unwrap();
        assert_eq!(spec.datasets[0].points, vec![(1.0, 2.0), (3.0, 4.0)]);
    }

    #[test]
    fn invalid_json_reports_line_and_column() {
        let err = parse("{\n  \"type\": bar\n}").unwrap_err();
        assert!(err.starts_with("Invalid JSON at line 2, column "), "{err}");
        let err = parse("not json").unwrap_err();
        assert!(err.contains("line 1, column"), "{err}");
    }

    #[test]
    fn empty_source_is_an_error() {
        assert!(parse("   \n ").unwrap_err().contains("empty"));
    }

    #[test]
    fn unsupported_or_missing_type_is_an_error() {
        let err = parse(r#"{"type":"radar","data":{"datasets":[]}}"#).unwrap_err();
        assert_eq!(
            err,
            "Unsupported chart type \"radar\" (expected bar, line, pie, doughnut or scatter)."
        );
        assert!(parse(r#"{"data":{"datasets":[]}}"#).unwrap_err().contains("Missing \"type\""));
        assert!(parse(r#"{"type":3,"data":{"datasets":[]}}"#).unwrap_err().contains("must be a string"));
        assert!(parse("[1,2]").unwrap_err().contains("JSON object"));
    }

    #[test]
    fn missing_or_bad_data_is_an_error() {
        assert!(parse(r#"{"type":"bar"}"#).unwrap_err().contains("\"data\""));
        assert!(parse(r#"{"type":"bar","data":{}}"#).unwrap_err().contains("data.datasets"));
        assert!(parse(r#"{"type":"bar","data":{"datasets":{}}}"#).unwrap_err().contains("data.datasets"));
        assert!(parse(r#"{"type":"bar","data":{"labels":"x","datasets":[]}}"#).unwrap_err().contains("labels"));
        assert!(parse(r#"{"type":"bar","data":{"datasets":[{"label":"q"}]}}"#).unwrap_err().contains("\"data\" array"));
    }

    #[test]
    fn empty_datasets_are_valid_and_render_no_data() {
        let spec = parse(r#"{"type":"bar","data":{"datasets":[]}}"#).unwrap();
        assert!(spec.datasets.is_empty());
        let scene = build_scene(&spec, 400.0, 300.0, FG(), GRID());
        assert!(scene.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "No data")));

        let pie = parse(r#"{"type":"pie","data":{"datasets":[{"data":[0,null]}]}}"#).unwrap();
        let scene = build_scene(&pie, 400.0, 300.0, FG(), GRID());
        assert!(scene.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "No data")));
    }

    #[test]
    fn non_numeric_values_are_errors_but_null_is_a_gap() {
        let err = parse(r#"{"type":"bar","data":{"datasets":[{"label":"s","data":[1,"abc"]}]}}"#).unwrap_err();
        assert!(err.contains("\"s\"") && err.contains("not a number") && err.contains("index 2"), "{err}");
        assert!(parse(r#"{"type":"line","data":{"datasets":[{"data":[true]}]}}"#).is_err());

        let spec = parse(r#"{"type":"line","data":{"datasets":[{"data":[1,null,"3",4]}]}}"#).unwrap();
        assert_eq!(spec.datasets[0].values, vec![Some(1.0), None, Some(3.0), Some(4.0)]);
    }

    #[test]
    fn options_title_legend_stacking_and_colours() {
        let spec = parse(
            r##"{"type":"bar","data":{"datasets":[{"data":[1],"backgroundColor":"#f00"}]},
               "options":{"plugins":{"title":{"display":true,"text":"Sales"},"legend":{"display":false}},
               "scales":{"y":{"stacked":true,"beginAtZero":true,"min":-2}}}}"##,
        )
        .unwrap();
        assert_eq!(spec.title.as_deref(), Some("Sales"));
        assert!(!spec.legend);
        assert!(spec.stacked);
        assert!(spec.begin_at_zero);
        assert_eq!(spec.y_min, Some(-2.0));
        assert_eq!(spec.datasets[0].color, parse_color("#f00"));

        let hidden = parse(r#"{"type":"bar","data":{"datasets":[]},"options":{"plugins":{"title":{"text":"x"}}}}"#).unwrap();
        assert!(hidden.title.is_none(), "Chart.js hides titles unless display is true");
    }

    #[test]
    fn colour_parsing() {
        assert_eq!(parse_color("#006fee"), Some(rgb(0x006fee)));
        assert_eq!(parse_color("#fff"), Some(rgb(0xffffff)));
        assert_eq!(parse_color("#00000080").map(|c| (c.a * 255.0).round() as u8), Some(128));
        let c = parse_color("rgba(255, 0, 0, 0.5)").unwrap();
        assert!((c.r - 1.0).abs() < 1e-6 && c.g == 0.0 && (c.a - 0.5).abs() < 1e-6);
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#12345"), None);
    }

    #[test]
    fn bars_default_to_palette_and_are_grouped() {
        let scene = scene_of(
            r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[{"data":[1,2,3]},{"data":[4,null,6]}]}}"#,
            400.0,
            300.0,
        );
        assert_eq!(count_rects_radius(&scene, palette_color(0), 0.0), 3);
        assert_eq!(count_rects_radius(&scene, palette_color(1), 0.0), 2);
        // Bars grow from zero: the tallest bar is the highest-valued one.
        let heights: Vec<f32> = scene
            .iter()
            .filter_map(|p| match p {
                Prim::Rect { h, color, .. } if *color == palette_color(0) => Some(*h),
                _ => None,
            })
            .collect();
        assert!(heights[2] > heights[1] && heights[1] > heights[0]);
    }

    #[test]
    fn stacked_bars_share_a_column() {
        let scene = scene_of(
            r#"{"type":"bar","data":{"labels":["a"],"datasets":[{"data":[1]},{"data":[2]}]},"options":{"scales":{"x":{"stacked":true}}}}"#,
            400.0,
            300.0,
        );
        let bars: Vec<(f32, f32)> = scene
            .iter()
            .filter_map(|p| match p {
                Prim::Rect { x, w, h, radius, .. } if *radius == 0.0 && *h > 1.5 => Some((*x, *w)),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 2);
        assert!((bars[0].0 - bars[1].0).abs() < 1e-4, "stacked bars share x");
    }

    #[test]
    fn line_breaks_at_gaps() {
        let scene = scene_of(
            r#"{"type":"line","data":{"labels":["a","b","c","d"],"datasets":[{"data":[1,null,3,4]}]}}"#,
            400.0,
            300.0,
        );
        let lines: Vec<&Vec<(f32, f32)>> = scene
            .iter()
            .filter_map(|p| match p {
                Prim::Polyline { points, .. } => Some(points),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 1, "the lone point before the gap is not a segment");
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn pie_has_one_wedge_per_positive_slice() {
        let scene = scene_of(
            r#"{"type":"pie","data":{"labels":["a","b","c","d"],"datasets":[{"data":[1,0,3,null]}]}}"#,
            400.0,
            300.0,
        );
        let wedges = scene.iter().filter(|p| matches!(p, Prim::Polygon { .. })).count();
        assert_eq!(wedges, 2);
        let legend = scene.iter().filter(|p| matches!(p, Prim::Text { .. })).count();
        assert_eq!(legend, 4, "one legend entry per label");
    }

    #[test]
    fn doughnut_wedges_are_closed_rings() {
        let scene = scene_of(
            r#"{"type":"doughnut","data":{"datasets":[{"data":[1,1]}]}}"#,
            400.0,
            300.0,
        );
        let wedges: Vec<&Vec<(f32, f32)>> = scene
            .iter()
            .filter_map(|p| match p {
                Prim::Polygon { points, .. } => Some(points),
                _ => None,
            })
            .collect();
        assert_eq!(wedges.len(), 2);
        assert!(wedges.iter().all(|w| w.len() > 4));
    }

    #[test]
    fn scatter_draws_each_point() {
        let scene = scene_of(
            r#"{"type":"scatter","data":{"datasets":[{"data":[{"x":0,"y":0},{"x":5,"y":10}]},{"data":[[2,3]]}]}}"#,
            400.0,
            300.0,
        );
        assert_eq!(count_rects_radius(&scene, palette_color(0), 4.0), 2);
        assert_eq!(count_rects_radius(&scene, palette_color(1), 4.0), 1);
    }

    #[test]
    fn every_primitive_is_finite_and_inside_the_canvas() {
        let samples = [
            r#"{"type":"bar","data":{"labels":["jan","feb","mar"],"datasets":[{"label":"a","data":[-5,10,3]},{"label":"b","data":[2,null,-7]}]},"options":{"plugins":{"title":{"display":true,"text":"T"}},"scales":{"x":{"stacked":true}}}}"#,
            r#"{"type":"line","data":{"labels":["a","b"],"datasets":[{"data":[1000000,2000000]}]}}"#,
            r#"{"type":"line","data":{"labels":["a"],"datasets":[{"data":[0.001]}]}}"#,
            r#"{"type":"pie","data":{"labels":["x","y"],"datasets":[{"data":[1,2]}]},"options":{"plugins":{"legend":{"display":true}}}}"#,
            r#"{"type":"doughnut","data":{"datasets":[{"data":[5]}]}}"#,
            r#"{"type":"scatter","data":{"datasets":[{"data":[{"x":-1,"y":-1},{"x":1,"y":1}]}]}}"#,
            r#"{"type":"bar","data":{"datasets":[]}}"#,
        ];
        for src in samples {
            let spec = parse(src).unwrap();
            let (w, h) = (360.0_f32, CHART_HEIGHT);
            let scene = build_scene(&spec, w, h, FG(), GRID());
            for prim in &scene {
                let pts: Vec<(f32, f32)> = match prim {
                    Prim::Rect { x, y, w, h, .. } => vec![(*x, *y), (x + w, y + h)],
                    Prim::Polygon { points, .. } | Prim::Polyline { points, .. } => points.clone(),
                    Prim::Text { x, y, .. } => vec![(*x, *y)],
                };
                for (x, y) in pts {
                    assert!(x.is_finite() && y.is_finite(), "{src}: {prim:?}");
                    assert!(x >= -1.0 && x <= w + 1.0 && y >= -1.0 && y <= h + 1.0, "{src}: {prim:?}");
                }
            }
        }
    }
}
