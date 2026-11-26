use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use fft_benchmarks::{analyze, samples, settings, FRAME_COUNT, FRAME_SIZES};
use fft_core::spectrogram::RenderOptions;
use realfft::RealFftPlanner;

fn rfft(c: &mut Criterion) {
    let mut group = c.benchmark_group("rfft_db");
    for &frame in FRAME_SIZES {
        let input = samples(frame);
        let frame_samples = &input[..frame];
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(frame);
        let mut fft_input = plan.make_input_vec();
        fft_input.copy_from_slice(frame_samples);
        let mut fft_output = plan.make_output_vec();
        group.throughput(Throughput::Bytes(
            (frame * std::mem::size_of::<f32>()) as u64,
        ));
        group.bench_function(BenchmarkId::new("rust_realfft", frame), |b| {
            b.iter(|| {
                plan.process(&mut fft_input, &mut fft_output).unwrap();
                let db: Vec<f32> = fft_output
                    .iter()
                    .map(|bin| 20.0 * bin.norm().max(1e-10).log10())
                    .collect();
                black_box(db);
            });
        });

        #[cfg(feature = "python")]
        python_compare::bench_rfft(&mut group, frame, frame_samples);
    }
    group.finish();
}

fn spectrogram(c: &mut Criterion) {
    let mut group = c.benchmark_group("spectrogram_db");
    group.throughput(Throughput::Elements(FRAME_COUNT as u64));
    for &frame in FRAME_SIZES {
        let input = samples(frame);
        let analysis_settings = settings(frame);
        group.bench_function(BenchmarkId::new("rust_core", frame), |b| {
            b.iter(|| {
                let analysis = analyze(&input, &analysis_settings).unwrap();
                black_box(analysis.spectrogram.frames());
            });
        });

        #[cfg(feature = "python")]
        python_compare::bench_spectrogram(&mut group, frame, &input);
    }
    group.finish();
}

fn render(c: &mut Criterion) {
    let frame = 4096;
    let input = samples(frame);
    let analysis_settings = settings(frame);
    let analysis = analyze(&input, &analysis_settings).unwrap();
    let spectrogram = analysis.spectrogram;
    let options = RenderOptions {
        width: 800,
        height: 600,
        ..RenderOptions::default()
    };

    let mut group = c.benchmark_group("spectrogram_render");
    group.throughput(Throughput::Elements(800 * 600));
    group.bench_function("rust_core_800x600", |b| {
        b.iter(|| {
            black_box(spectrogram.render(&options));
        });
    });
    group.finish();
}

#[cfg(feature = "python")]
mod python_compare {
    use std::ffi::CString;

    use super::*;
    use criterion::{measurement::WallTime, BenchmarkGroup};
    use numpy::PyArray1;
    use pyo3::prelude::*;
    use pyo3::types::PyModule;

    const PYTHON_CODE: &str = r#"
import numpy as np

def rfft_db(x):
    return 20.0 * np.log10(np.maximum(np.abs(np.fft.rfft(x)), 1e-10))

def spectrogram_db(x, frame, hop):
    count = 1 + (len(x) - frame) // hop
    out = np.empty((count, frame // 2 + 1), dtype=np.float32)
    for index in range(count):
        start = index * hop
        spec = np.fft.rfft(x[start:start + frame])
        out[index] = 20.0 * np.log10(np.maximum(np.abs(spec), 1e-10))
    return out
"#;

    fn setup(name: &str, values: Vec<f32>) -> (Py<PyAny>, Py<PyArray1<f32>>) {
        Python::initialize();
        Python::attach(|py| {
            let code = CString::new(PYTHON_CODE).unwrap();
            let module =
                PyModule::from_code(py, code.as_c_str(), c"fft_bench.py", c"fft_bench").unwrap();
            let function = module.getattr(name).unwrap().unbind();
            let array = PyArray1::from_vec(py, values).unbind();
            (function, array)
        })
    }

    pub fn bench_rfft(
        group: &mut BenchmarkGroup<'_, WallTime>,
        frame: usize,
        frame_samples: &[f32],
    ) {
        let (function, array) = setup("rfft_db", frame_samples.to_vec());
        group.bench_function(BenchmarkId::new("python_numpy", frame), |b| {
            b.iter(|| {
                Python::attach(|py| {
                    let output = function.bind(py).call1((array.bind(py),)).unwrap();
                    black_box(output);
                });
            });
        });
    }

    pub fn bench_spectrogram(
        group: &mut BenchmarkGroup<'_, WallTime>,
        frame: usize,
        values: &[f32],
    ) {
        let (function, array) = setup("spectrogram_db", values.to_vec());
        let hop = frame / 2;
        group.bench_function(BenchmarkId::new("python_numpy", frame), |b| {
            b.iter(|| {
                Python::attach(|py| {
                    let output = function
                        .bind(py)
                        .call1((array.bind(py), frame, hop))
                        .unwrap();
                    black_box(output);
                });
            });
        });
    }
}

fn all_benchmarks(c: &mut Criterion) {
    rfft(c);
    spectrogram(c);
    render(c);
}

criterion_group!(benches, all_benchmarks);
criterion_main!(benches);
