//! Python boundary for the factor-expression engine.

use numpy::{IntoPyArray, PyArray1, PyReadonlyArray2};
use phasor_expr::{FactorInputs, FactorSet};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyDictMethods};

use crate::array::{columns_of, to_py_err};

/// The crossings, as one column-major boolean array the caller reshapes with
/// `order="F"`. Returned flat rather than as nested lists: a list of lists is
/// one Python object per cell, and `np.asarray` over it was the slowest step
/// of a sweep that used it.
#[pyfunction]
pub fn crossed_above<'py>(
    py: Python<'py>,
    left: PyReadonlyArray2<'_, f64>,
    right: PyReadonlyArray2<'_, f64>,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let (a, rows, cols) = columns_of(&left)?;
    let (b, other_rows, other_cols) = columns_of(&right)?;
    if (rows, cols) != (other_rows, other_cols) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "crossed_above inputs must have the same shape",
        ));
    }
    let flat = py.detach(|| {
        let mut out = Vec::with_capacity(rows * cols);
        for col in 0..cols {
            let start = col * rows;
            out.extend(phasor_generic::crossed_above_1d(
                &a[start..start + rows],
                &b[start..start + rows],
                0,
            ));
        }
        out
    });
    Ok(flat.into_pyarray(py))
}

/// A compiled set of named factor expressions.
#[pyclass(name = "_FactorSet", module = "phasor._phasor")]
#[derive(Debug)]
pub struct PyFactorSet {
    inner: FactorSet,
}

#[pymethods]
impl PyFactorSet {
    /// Compiles `(name, expression)` pairs in their supplied order.
    #[staticmethod]
    fn compile(factors: Vec<(String, String)>) -> PyResult<Self> {
        Ok(Self {
            inner: FactorSet::compile(factors).map_err(to_py_err)?,
        })
    }

    #[getter]
    fn names(&self) -> Vec<String> {
        self.inner.names().map(str::to_owned).collect()
    }

    #[getter]
    fn required_fields(&self) -> Vec<String> {
        self.inner
            .required_fields()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[getter]
    fn uses_future(&self) -> bool {
        self.inner.uses_future()
    }

    /// Evaluates F-ordered float64 fields and returns column-major flat arrays.
    fn evaluate<'py>(
        &self,
        py: Python<'py>,
        fields: Vec<(String, PyReadonlyArray2<'py, f64>)>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let inputs = inputs_of(&fields)?;

        // The inputs borrow NumPy buffers that stay alive for this call, and
        // the evaluation touches no Python object, so other threads may run.
        let values = py
            .detach(|| self.inner.evaluate(&inputs))
            .map_err(to_py_err)?;
        values_dict(py, values)
    }

    /// Evaluates `batch` symbols at a time and calls `sink(start, stop,
    /// values)` for each batch, `values` as [`Self::evaluate`] returns them.
    fn evaluate_batches<'py>(
        &self,
        py: Python<'py>,
        fields: Vec<(String, PyReadonlyArray2<'py, f64>)>,
        batch: usize,
        sink: Bound<'py, PyAny>,
    ) -> PyResult<()> {
        let inputs = inputs_of(&fields)?;
        let mut failure = None;
        let run = self.inner.evaluate_batches(&inputs, batch, |cols, values| {
            let called = values_dict(py, values)
                .and_then(|d| sink.call1((cols.start, cols.end, d)).map(|_| ()));
            called.map_err(|e| {
                failure = Some(e);
                phasor_core::Error::Invariant("the sink raised".into())
            })
        });
        if let Some(error) = failure {
            return Err(error);
        }
        run.map_err(to_py_err)
    }
}

fn values_dict(py: Python<'_>, values: phasor_expr::FactorValues) -> PyResult<Bound<'_, PyDict>> {
    let out = PyDict::new(py);
    for (name, value) in values.into_matrices() {
        out.set_item(name, value.into_vec().into_pyarray(py))?;
    }
    Ok(out)
}

fn inputs_of<'a>(fields: &'a [(String, PyReadonlyArray2<'_, f64>)]) -> PyResult<FactorInputs<'a>> {
    let first = fields.first().ok_or_else(|| {
        to_py_err(phasor_core::Error::Invariant(
            "factor inputs cannot be empty".into(),
        ))
    })?;
    let (_, n_rows, n_cols) = columns_of(&first.1)?;
    let mut inputs = FactorInputs::new(n_rows, n_cols).map_err(to_py_err)?;
    for (name, array) in fields {
        let (values, rows, cols) = columns_of(array)?;
        if (rows, cols) != (n_rows, n_cols) {
            return Err(to_py_err(phasor_core::Error::Invariant(format!(
                "factor field {name:?} has shape {rows}x{cols}, expected {n_rows}x{n_cols}"
            ))));
        }
        inputs = inputs.with_field(name, values).map_err(to_py_err)?;
    }
    Ok(inputs)
}

/// Qlib's default Alpha158 factor set.
#[pyfunction]
pub fn alpha158() -> PyResult<PyFactorSet> {
    Ok(PyFactorSet {
        inner: phasor_expr::alpha158().map_err(to_py_err)?,
    })
}

/// Qlib's default Alpha360 factor set.
#[pyfunction]
pub fn alpha360() -> PyResult<PyFactorSet> {
    Ok(PyFactorSet {
        inner: phasor_expr::alpha360().map_err(to_py_err)?,
    })
}

/// WorldQuant's 101 alphas: the 82 that read only prices and volume, or all
/// 101 (`all`), which also read `cap`, `sector`, `industry`, `subindustry`.
#[pyfunction]
#[pyo3(signature = (all=false))]
pub fn wq101(all: bool) -> PyResult<PyFactorSet> {
    let inner = if all {
        phasor_expr::wq101_all()
    } else {
        phasor_expr::wq101()
    };
    Ok(PyFactorSet {
        inner: inner.map_err(to_py_err)?,
    })
}

/// The paper's formula for alpha `k`.
#[pyfunction]
pub fn wq101_formula(k: usize) -> Option<&'static str> {
    phasor_expr::wq101_formula(k)
}
