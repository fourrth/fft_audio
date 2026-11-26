//! End-to-end checks of the cut maths on synthetic audio with known answers:
//! WAV file -> `pipeline::analyze` -> cuts.

use crate::orders::{self, Cut, RpmProfile};
use crate::pipeline;
use crate::settings::{AnalysisSettings, CutAxis, LineAxis};
use crate::spectrogram::Spectrogram;
use crate::testutil::{chirp, tones, write_wav};

const SR: u32 = 16_000;

fn analyse(name: &str, samples: &[f32]) -> Spectrogram {
    let path = write_wav(name, SR, samples);
    let a = AnalysisSettings {
        frame: 4096,
        ..Default::default()
    };
    pipeline::analyze(&path, &a).expect("analyze").spectrogram
}

/// dB values of the points whose time is in `[t0, t1]` (skips partial edge frames).
fn interior(cut: &Cut, t0: f64, t1: f64) -> Vec<f32> {
    cut.points
        .iter()
        .filter(|p| p.time_s >= t0 && p.time_s <= t1)
        .map(|p| p.db)
        .collect()
}

fn spread(v: &[f32]) -> f32 {
    v.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
        - v.iter().cloned().fold(f32::INFINITY, f32::min)
}

fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len() as f32
}

fn argmax_time(cut: &Cut) -> f64 {
    cut.points
        .iter()
        .max_by(|a, b| a.db.total_cmp(&b.db))
        .unwrap()
        .time_s
}

fn argmax_freq(cut: &Cut) -> f64 {
    cut.points
        .iter()
        .max_by(|a, b| a.db.total_cmp(&b.db))
        .unwrap()
        .freq_hz
}

/// A steady drive at exactly 2400 rpm: order 1 = 40 Hz (amp 0.5), order 2 =
/// 80 Hz (amp 0.25), nothing at order 3.
#[test]
fn steady_rpm_order_cuts_over_time() {
    let secs = 4.0;
    let spec = analyse("steady", &tones(SR, secs, &[(40.0, 0.5), (80.0, 0.25)]));
    let rpm = RpmProfile::constant(2400.0);
    assert!(rpm.is_constant());

    let o1 = orders::order_cut(&spec, &rpm, 1.0, 0.2);
    let o2 = orders::order_cut(&spec, &rpm, 2.0, 0.2);
    let o3 = orders::order_cut(&spec, &rpm, 3.0, 0.2);
    assert!(o1.points.len() > 30);

    let (a, b) = (0.5, secs - 0.5);
    let (m1, m2, m3) = (
        mean(&interior(&o1, a, b)),
        mean(&interior(&o2, a, b)),
        mean(&interior(&o3, a, b)),
    );
    eprintln!("steady: order1 {m1:.1} dB, order2 {m2:.1} dB, order3 {m3:.1} dB");
    // amplitude ratio 2 = 6.02 dB
    assert!(
        (m1 - m2 - 6.02).abs() < 1.5,
        "order1 - order2 = {}",
        m1 - m2
    );
    assert!(
        m1 - m3 > 30.0,
        "order 3 should be empty, order1 - order3 = {}",
        m1 - m3
    );
    // steady state: the level over time is flat
    assert!(spread(&interior(&o1, a, b)) < 1.5, "level vs time not flat");

    // x axis: constant RPM -> "vs RPM" / "vs order freq" collapse, time is used instead
    let hop = spec.hop_s();
    let bin = spec.bin_hz() as f64;
    assert!(orders::is_degenerate(&o1, CutAxis::Rpm, hop, bin));
    assert!(orders::is_degenerate(&o1, CutAxis::Freq, hop, bin));
    assert!(!orders::is_degenerate(&o1, CutAxis::Time, hop, bin));
    for requested in [CutAxis::Rpm, CutAxis::Time, CutAxis::Freq] {
        let (axis, note) = orders::resolve_axis(requested, rpm.is_constant());
        assert_eq!(axis, CutAxis::Time);
        assert_eq!(note.is_some(), requested != CutAxis::Time);
    }
    // x values are real times
    let xs: Vec<f64> = o1
        .points
        .iter()
        .map(|p| orders::cut_x(p, CutAxis::Time))
        .collect();
    assert!(xs.windows(2).all(|w| w[1] > w[0]));
    assert!(xs[0] < 0.5 && *xs.last().unwrap() > secs - 0.5);
}

/// A chirp that is exactly order 1 of a linear RPM ramp (2400 -> 7200 rpm =
/// 40 -> 120 Hz over 4 s). Order cut must be flat vs RPM; a fixed 80 Hz cut
/// must peak once, at t = 2 s.
#[test]
fn sweep_order_vs_frequency_cuts_and_axes() {
    let secs = 4.0;
    let spec = analyse("sweep", &chirp(SR, secs, 40.0, 120.0, 0.5));
    let rpm = RpmProfile::ramp(0.0, secs, 2400.0, 7200.0);
    assert!(!rpm.is_constant());
    let (hop, bin) = (spec.hop_s(), spec.bin_hz() as f64);
    let (a, b) = (0.6, secs - 0.6);

    // order 1 follows the chirp -> flat
    let o1 = orders::order_cut(&spec, &rpm, 1.0, 0.2);
    let o1_db = interior(&o1, a, b);
    eprintln!(
        "sweep: order-1 cut spread {:.2} dB over {} frames",
        spread(&o1_db),
        o1_db.len()
    );
    assert!(spread(&o1_db) < 3.0);
    // order 2 (80 -> 240 Hz) has no energy
    let o2 = orders::order_cut(&spec, &rpm, 2.0, 0.2);
    assert!(mean(&o1_db) - mean(&interior(&o2, a, b)) > 25.0);

    // fixed 80 Hz: tone passes through at t = 2 s
    let f80 = orders::freq_cut(&spec, &rpm, 80.0, 5.0);
    let f80_db: Vec<f32> = f80.points.iter().map(|p| p.db).collect();
    eprintln!(
        "sweep: 80 Hz cut peaks at t = {:.2} s, range {:.1} dB",
        argmax_time(&f80),
        spread(&f80_db)
    );
    assert!(spread(&f80_db) > 25.0);
    assert!((argmax_time(&f80) - 2.0).abs() < 0.4);

    // ---- "cuts vs": x values on each axis
    let rpm_x: Vec<f64> = o1
        .points
        .iter()
        .map(|p| orders::cut_x(p, CutAxis::Rpm))
        .collect();
    let f_x: Vec<f64> = o1
        .points
        .iter()
        .map(|p| orders::cut_x(p, CutAxis::Freq))
        .collect();
    let t_x: Vec<f64> = o1
        .points
        .iter()
        .map(|p| orders::cut_x(p, CutAxis::Time))
        .collect();
    for xs in [&rpm_x, &f_x, &t_x] {
        assert!(
            xs.windows(2).all(|w| w[1] > w[0]),
            "x must increase with time on a ramp"
        );
    }
    assert!((rpm_x[0] - 2400.0).abs() < 400.0 && (rpm_x.last().unwrap() - 7200.0).abs() < 400.0);
    // order frequency = rpm / 60 for order 1
    for (r, f) in rpm_x.iter().zip(&f_x) {
        assert!((r / 60.0 - f).abs() < 1e-6);
    }
    // the axis is kept as requested on a real ramp, and nothing is hidden for order cuts
    for axis in [CutAxis::Rpm, CutAxis::Time, CutAxis::Freq] {
        assert_eq!(orders::resolve_axis(axis, false), (axis, None));
        assert!(!orders::is_degenerate(&o1, axis, hop, bin));
    }
    // fixed-frequency cuts have no extent on the order-frequency axis (hidden there),
    // but are fine against RPM and time
    assert!(orders::is_degenerate(&f80, CutAxis::Freq, hop, bin));
    assert!(!orders::is_degenerate(&f80, CutAxis::Rpm, hop, bin));
    assert!(!orders::is_degenerate(&f80, CutAxis::Time, hop, bin));
}

/// Line cuts at various angles, on the same chirp.
#[test]
fn line_cuts_at_any_angle() {
    let secs = 4.0;
    let spec = analyse("lines", &chirp(SR, secs, 40.0, 120.0, 0.5));
    let rpm = RpmProfile::ramp(0.0, secs, 2400.0, 7200.0);
    let (hop, bin) = (spec.hop_s(), spec.bin_hz() as f64);
    let (a, b) = (0.6, secs - 0.6);

    // sloped line drawn along the chirp == order 1
    let slope = orders::LineSpec {
        t0: 0.0,
        f0: 40.0,
        t1: secs,
        f1: 120.0,
        log_f: false,
    };
    let sl = orders::line_cut(&spec, &rpm, &slope, 0.0, 1);
    let o1 = orders::order_cut(&spec, &rpm, 1.0, 0.2);
    let sl_db = interior(&sl, a, b);
    eprintln!(
        "lines: sloped spread {:.2} dB, mean {:.1} vs order-1 mean {:.1}",
        spread(&sl_db),
        mean(&sl_db),
        mean(&interior(&o1, a, b))
    );
    assert!(spread(&sl_db) < 4.0);
    assert!((mean(&sl_db) - mean(&interior(&o1, a, b))).abs() < 3.0);
    // the same line pushed 30 Hz off the ridge sees nothing
    let off = orders::LineSpec {
        f0: 70.0,
        f1: 150.0,
        ..slope.clone()
    };
    let off_cut = orders::line_cut(&spec, &rpm, &off, 0.0, 2);
    assert!(mean(&sl_db) - mean(&interior(&off_cut, a, b)) > 25.0);
    // peak-window mode finds the ridge even when the line is 6 Hz off
    let near = orders::LineSpec {
        f0: 46.0,
        f1: 126.0,
        ..slope.clone()
    };
    let near_w = orders::line_cut(&spec, &rpm, &near, 8.0, 3);
    assert!(spread(&interior(&near_w, a, b)) < 4.0);

    // vertical line at t = 2 s = spectrum of that moment: peak at 80 Hz
    let vert = orders::LineSpec {
        t0: 2.0,
        f0: 0.0,
        t1: 2.0,
        f1: 200.0,
        log_f: false,
    };
    let v = orders::line_cut(&spec, &rpm, &vert, 0.0, 4);
    eprintln!(
        "lines: vertical cut has {} points, peak at {:.1} Hz",
        v.points.len(),
        argmax_freq(&v)
    );
    assert!(v.points.len() > 90, "every bin must be sampled");
    assert!((argmax_freq(&v) - 80.0).abs() < 3.0);
    assert!(v.points.iter().all(|p| (p.time_s - 2.0).abs() < 1e-9));

    // horizontal line at 80 Hz = fixed-frequency cut: peaks at t = 2 s
    let hor = orders::LineSpec {
        t0: 0.0,
        f0: 80.0,
        t1: secs,
        f1: 80.0,
        log_f: false,
    };
    let h = orders::line_cut(&spec, &rpm, &hor, 0.0, 5);
    assert!((argmax_time(&h) - 2.0).abs() < 0.4);
    let f80 = orders::freq_cut(&spec, &rpm, 80.0, 0.0);
    assert!((argmax_time(&h) - argmax_time(&f80)).abs() < 0.3);

    // log-space line: geometric from 40 to 120 Hz is *not* the chirp (which is linear in Hz)
    let geo = orders::LineSpec {
        log_f: true,
        ..slope.clone()
    };
    let g = orders::line_cut(&spec, &rpm, &geo, 0.0, 6);
    assert!(spread(&interior(&g, a, b)) > 6.0);
    let (mid_t, mid_f) = geo.at(0.5);
    assert!((mid_t - 2.0).abs() < 1e-9 && (mid_f - (40.0f64 * 120.0).sqrt()).abs() < 1e-9);

    // line outside the data is empty, not a panic
    let outside = orders::LineSpec {
        t0: 100.0,
        f0: 10.0,
        t1: 120.0,
        f1: 20.0,
        log_f: false,
    };
    assert!(orders::line_cut(&spec, &rpm, &outside, 0.0, 7)
        .points
        .is_empty());

    // axes: vertical only fits frequency, horizontal only time, slope fits both
    assert!(orders::is_degenerate(&v, CutAxis::Time, hop, bin));
    assert!(!orders::is_degenerate(&v, CutAxis::Freq, hop, bin));
    assert!(orders::is_degenerate(&h, CutAxis::Freq, hop, bin));
    assert!(!orders::is_degenerate(&h, CutAxis::Time, hop, bin));
    assert!(!orders::is_degenerate(&sl, CutAxis::Time, hop, bin));
    assert!(!orders::is_degenerate(&sl, CutAxis::Freq, hop, bin));
    assert_eq!(
        orders::resolve_line_axis(&[v.clone()], LineAxis::Auto, hop, bin),
        CutAxis::Freq
    );
    assert_eq!(
        orders::resolve_line_axis(&[h.clone()], LineAxis::Auto, hop, bin),
        CutAxis::Time
    );
    // a slope fits both axes, so the axis that can also draw the other line wins
    assert_eq!(
        orders::resolve_line_axis(&[sl.clone(), v.clone()], LineAxis::Auto, hop, bin),
        CutAxis::Freq
    );
    assert_eq!(
        orders::resolve_line_axis(&[sl.clone(), h.clone()], LineAxis::Auto, hop, bin),
        CutAxis::Time
    );
    assert_eq!(
        orders::resolve_line_axis(&[], LineAxis::Auto, hop, bin),
        CutAxis::Time
    );
    assert_eq!(
        orders::resolve_line_axis(&[v.clone(), v.clone(), h.clone()], LineAxis::Auto, hop, bin),
        CutAxis::Freq
    );
    assert_eq!(
        orders::resolve_line_axis(&[v.clone()], LineAxis::Time, hop, bin),
        CutAxis::Time
    );
}
