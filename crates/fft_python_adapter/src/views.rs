//! NumPy view helpers for the Python bindings.
//!
//! Canonical runtime data is contiguous and `Arc`-backed. These helpers create
//! NumPy arrays whose base object is the owning Python handle, so the handle
//! keeps the buffer alive for as long as Python can reach the array. Read-only
//! views clear NumPy's `WRITEABLE` flag; mutable views are only used after the
//! handle has taken exclusive ownership of the result (`take_cut_data`).

use numpy::ndarray::{Array2, ArrayView, ArrayView1, ArrayView2, Dimension};
use numpy::npyffi::{self, PyArrayObject};
use numpy::{Element, PyArray, PyArray1, PyArray2};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Clear NumPy's writeable flag on an array created by `borrow_from_array`.
///
/// # Safety
/// `array` must point at a live `numpy::npyffi::PyArrayObject`.
pub(crate) fn mark_readonly<T, D>(array: &Bound<'_, PyArray<T, D>>)
where
    T: Element,
    D: Dimension,
{
    // SAFETY: every `PyArray<T, D>` is a NumPy ndarray object. The cast is the
    // same one the numpy crate uses internally, and clearing a flag does not
    // move or free the data buffer. The Python base object still owns the
    // backing allocation.
    unsafe {
        let ptr = array.as_ptr().cast::<PyArrayObject>();
        (*ptr).fields.flags &= !npyffi::NPY_ARRAY_WRITEABLE;
    }
}

/// Read-only 1-D view over a contiguous slice owned by `container`.
pub(crate) fn readonly_1d<'py, T: Element>(
    slice: &[T],
    container: Bound<'py, PyAny>,
) -> Bound<'py, PyArray1<T>> {
    // SAFETY: `slice` points at stable contiguous storage owned (directly or
    // through an Arc) by `container`, which is installed as NumPy's base
    // object. The handle never reallocates the buffer while a view exists.
    let view: ArrayView1<'_, T> = unsafe { ArrayView::from_shape_ptr(slice.len(), slice.as_ptr()) };
    let array = unsafe { PyArray::borrow_from_array(&view, container) };
    mark_readonly(&array);
    array
}

/// Read-only 2-D view over row-major contiguous storage owned by `container`.
pub(crate) fn readonly_2d<'py, T: Element>(
    slice: &[T],
    rows: usize,
    cols: usize,
    container: Bound<'py, PyAny>,
) -> Bound<'py, PyArray2<T>> {
    debug_assert_eq!(slice.len(), rows.saturating_mul(cols));
    // SAFETY: see `readonly_1d`. `rows * cols == slice.len()` and the layout
    // is C-contiguous, matching the runtime's frame-major buffers.
    let view: ArrayView2<'_, T> =
        unsafe { ArrayView::from_shape_ptr((rows, cols), slice.as_ptr()) };
    let array = unsafe { PyArray::borrow_from_array(&view, container) };
    mark_readonly(&array);
    array
}

/// Exclusive writeable 2-D view over storage owned by the leased handle.
///
/// The caller must hold the exclusive lease for its lifetime, keep every other
/// read view/info detached, and clear `WRITEABLE` when the lease ends (`close`
/// or handle drop). No alias is created while the lease is active.
pub(crate) fn mutable_2d<'py, T: Element>(
    slice: &mut [T],
    rows: usize,
    cols: usize,
    container: Bound<'py, PyAny>,
) -> Bound<'py, PyArray2<T>> {
    debug_assert_eq!(slice.len(), rows.saturating_mul(cols));
    // SAFETY: the handle lease guarantees this is the only live view of the
    // contiguous storage for the duration of the lease, and `container` keeps
    // the allocation alive.
    let view: ArrayView2<'_, T> =
        unsafe { ArrayView::from_shape_ptr((rows, cols), slice.as_ptr()) };
    unsafe { PyArray::borrow_from_array(&view, container) }
}

/// Owning 1-D array. The `Vec` is moved into NumPy, so this is a copy of the
/// caller's payload but zero additional copies afterward.
pub(crate) fn owned_1d<'py, T: Element>(
    py: Python<'py>,
    values: Vec<T>,
) -> Bound<'py, PyArray1<T>> {
    PyArray1::from_vec(py, values)
}

/// Owning 2-D array from a row-major `Vec`.
///
/// `Array2::from_shape_vec` and `from_owned_array` move the allocation into
/// NumPy, so the payload is not copied and the result owns its data.
pub(crate) fn owned_2d<'py, T: Element>(
    py: Python<'py>,
    values: Vec<T>,
    rows: usize,
    cols: usize,
) -> PyResult<Bound<'py, PyArray2<T>>> {
    let array = Array2::from_shape_vec((rows, cols), values)
        .map_err(|error| PyValueError::new_err(format!("array shape mismatch: {error}")))?;
    Ok(PyArray::from_owned_array(py, array))
}
