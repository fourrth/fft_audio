//! PNG helpers shared by the spectrogram/order image routes.
//!
//! These are transport formatting helpers with no app or runtime state. Cut
//! points are rebuilt from the runtime's `CutData` because the object-safe
//! runtime trait has no concrete-runtime `cut_points` helper.

use std::io::Cursor;

use fft_core::orders::{self, Cut, CutPoint, RpmProfile};
use fft_core::settings::{CutAxis, OrderCutSettings};
use fft_core::spectrogram::Spectrogram;

const CUT_PLOT_WIDTH: u32 = 800;
const CUT_PLOT_HEIGHT: u32 = 400;
const CUT_PLOT_LEFT: f32 = 40.0;
const CUT_PLOT_RIGHT: f32 = 20.0;
const CUT_PLOT_TOP: f32 = 20.0;
const CUT_PLOT_BOTTOM: f32 = 40.0;
const CUT_PLOT_CURVE_WIDTH: f32 = 3.0;

/// colorblind-safe fallback palette, shared with `web/src/cuts/model.ts`.
const CUT_PALETTE: [[u8; 3]; 8] = [
    [0x56, 0xb4, 0xe9],
    [0xe6, 0x9f, 0x00],
    [0x00, 0x9e, 0x73],
    [0xcc, 0x79, 0xa7],
    [0xf0, 0xe4, 0x42],
    [0x00, 0x72, 0xb2],
    [0xd5, 0x5e, 0x00],
    [0xe0, 0xe0, 0xe0],
];

#[derive(Debug)]
pub(crate) enum RenderError {
    BadSize,
    Png(image::ImageError),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::BadSize => formatter.write_str("bad image size"),
            RenderError::Png(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for RenderError {}

fn encode_rgb_png(png: image::RgbImage) -> Result<Vec<u8>, RenderError> {
    let mut buffer = Cursor::new(Vec::new());
    png.write_to(&mut buffer, image::ImageFormat::Png)
        .map_err(RenderError::Png)?;
    Ok(buffer.into_inner())
}

pub(crate) fn encode_pixels(
    width: usize,
    height: usize,
    pixels: &[u32],
) -> Result<Vec<u8>, RenderError> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for &pixel in pixels {
        rgb.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
    }
    let png =
        image::RgbImage::from_raw(width as u32, height as u32, rgb).ok_or(RenderError::BadSize)?;
    encode_rgb_png(png)
}

/// Fill every pixel whose centre is within `thickness / 2` of the segment.
/// Drawing exact palette colors keeps the PNG deterministic and easy to
/// verify without anti-aliasing heuristics.
fn draw_thick_line(
    canvas: &mut image::RgbImage,
    from: (f32, f32),
    to: (f32, f32),
    color: [u8; 3],
    thickness: f32,
) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length_sq = dx * dx + dy * dy;
    let radius = (thickness * 0.5).max(0.5);
    let min_x = (x0.min(x1) - radius).floor() as i64;
    let max_x = (x0.max(x1) + radius).ceil() as i64;
    let min_y = (y0.min(y1) - radius).floor() as i64;
    let max_y = (y0.max(y1) + radius).ceil() as i64;
    let width = canvas.width() as i64;
    let height = canvas.height() as i64;
    for py in min_y.max(0)..=max_y.min(height - 1) {
        for px in min_x.max(0)..=max_x.min(width - 1) {
            let qx = px as f32 + 0.5;
            let qy = py as f32 + 0.5;
            let t = if length_sq > 1e-12 {
                (((qx - x0) * dx + (qy - y0) * dy) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let cx = x0 + dx * t;
            let cy = y0 + dy * t;
            let dist_sq = (qx - cx) * (qx - cx) + (qy - cy) * (qy - cy);
            if dist_sq <= radius * radius {
                canvas.put_pixel(px as u32, py as u32, image::Rgb(color));
            }
        }
    }
}

pub(crate) fn render_order_cut_png(cuts: &[Cut], axis: CutAxis) -> Result<Vec<u8>, RenderError> {
    let mut image =
        image::RgbImage::from_pixel(CUT_PLOT_WIDTH, CUT_PLOT_HEIGHT, image::Rgb([18, 19, 24]));

    let left = CUT_PLOT_LEFT as u32;
    let right = CUT_PLOT_WIDTH - CUT_PLOT_RIGHT as u32;
    let top = CUT_PLOT_TOP as u32;
    let bottom = CUT_PLOT_HEIGHT - CUT_PLOT_BOTTOM as u32;
    let axis_color = image::Rgb([88, 94, 106]);
    for y in top..=bottom {
        image.put_pixel(left, y, axis_color);
    }
    for x in left..=right {
        image.put_pixel(x, bottom, axis_color);
    }

    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut min_db, mut max_db) = (f64::INFINITY, f64::NEG_INFINITY);
    for cut in cuts {
        for point in &cut.points {
            let x = orders::cut_x(point, axis);
            let db = point.db as f64;
            if x.is_finite() && db.is_finite() {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_db = min_db.min(db);
                max_db = max_db.max(db);
            }
        }
    }
    if !(min_x.is_finite() && max_x.is_finite() && min_db.is_finite() && max_db.is_finite()) {
        return encode_rgb_png(image);
    }

    if max_x > min_x {
        let pad = (max_x - min_x) * 0.03;
        min_x -= pad;
        max_x += pad;
    } else {
        min_x -= 1.0;
        max_x += 1.0;
    }
    if max_db > min_db {
        let pad = (max_db - min_db) * 0.10;
        min_db -= pad;
        max_db += pad;
    } else {
        min_db -= 1.0;
        max_db += 1.0;
    }

    let plot_w = (CUT_PLOT_WIDTH - CUT_PLOT_LEFT as u32 - CUT_PLOT_RIGHT as u32) as f64;
    let plot_h = (CUT_PLOT_HEIGHT - CUT_PLOT_TOP as u32 - CUT_PLOT_BOTTOM as u32) as f64;
    let map_x = |x: f64| CUT_PLOT_LEFT as f64 + (x - min_x) / (max_x - min_x) * plot_w;
    let map_y = |db: f64| CUT_PLOT_TOP as f64 + (1.0 - (db - min_db) / (max_db - min_db)) * plot_h;

    for (index, cut) in cuts.iter().enumerate() {
        let color = CUT_PALETTE[index % CUT_PALETTE.len()];
        let mut previous: Option<(f32, f32)> = None;
        for point in &cut.points {
            let x = orders::cut_x(point, axis);
            let db = point.db as f64;
            if !x.is_finite() || !db.is_finite() {
                continue;
            }
            let current = (map_x(x) as f32, map_y(db) as f32);
            if let Some(previous) = previous {
                draw_thick_line(&mut image, previous, current, color, CUT_PLOT_CURVE_WIDTH);
            }
            previous = Some(current);
        }
    }

    encode_rgb_png(image)
}

/// Full order-cut response for a runtime order matrix.
///
/// The runtime stores order cells as a matrix but computes fixed-frequency
/// cuts and time smoothing in the core `OrderCutSession`. Rebuild the wire
/// cuts the same way so `/cuts/order` and `/cuts/order/image` keep their old
/// shape without a concrete `CoreRuntime`.
pub(crate) fn order_cuts_from_data(
    data: &fft_runtime::CutData,
    settings: &OrderCutSettings,
    spectrogram: &Spectrogram,
    rpm: &RpmProfile,
) -> Vec<Cut> {
    let mut cuts = cuts_from_data(data);
    if matches!(data, fft_runtime::CutData::Order(_)) {
        for frequency in orders::parse_freqs(&settings.freqs) {
            cuts.push(orders::freq_cut(
                spectrogram,
                rpm,
                frequency,
                settings.freq_width_hz,
            ));
        }
    }
    for cut in &mut cuts {
        orders::smooth_cut(cut, settings.smooth_s);
    }
    cuts
}

/// Rebuild wire `Cut` points from a runtime cut result.
///
/// `CoreRuntime::cut_points` is an inherent helper, not part of
/// [`fft_runtime::FftRuntime`], so adapters reconstruct the wire shape from
/// `CutData` instead of holding a concrete runtime. Order cells that the
/// runtime marked unavailable (NaN) are skipped, matching the core cut builder.
pub(crate) fn cuts_from_data(data: &fft_runtime::CutData) -> Vec<Cut> {
    use fft_runtime::CutData;

    match data {
        CutData::Order(order) => (0..order.order_count)
            .filter_map(|column| {
                let order_value = *order.orders.get(column)?;
                let mut points = Vec::new();
                for frame in 0..order.frames {
                    let cell = frame * order.order_count + column;
                    let time_s = *order.times.get(frame)?;
                    let freq_hz = *order.frequencies.get(cell)? as f64;
                    let db = *order.values.get(cell)?;
                    if !time_s.is_finite() || !freq_hz.is_finite() || !db.is_finite() {
                        continue;
                    }
                    let rpm = if order_value > 0.0 {
                        freq_hz * 60.0 / order_value
                    } else {
                        0.0
                    };
                    points.push(CutPoint {
                        time_s,
                        rpm,
                        freq_hz,
                        db,
                    });
                }
                Some(Cut {
                    kind: fft_core::orders::CutKind::Order(order_value),
                    points,
                })
            })
            .collect(),
        CutData::Line(tracks) => tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                let mut points = Vec::new();
                for point in 0..track.len() {
                    let time_s = track.times[point];
                    let freq_hz = track.frequencies[point];
                    let db = track.db[point];
                    if !time_s.is_finite() || !freq_hz.is_finite() || !db.is_finite() {
                        continue;
                    }
                    points.push(CutPoint {
                        time_s,
                        rpm: 0.0,
                        freq_hz,
                        db,
                    });
                }
                Cut {
                    kind: fft_core::orders::CutKind::Line(index + 1),
                    points,
                }
            })
            .collect(),
    }
}
