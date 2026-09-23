//! Python boundary for the factor-expression engine.

use barq_expr::{FactorInputs, FactorSet};
use numpy::{IntoPyArray, PyReadonlyArray2};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyDictMethods};

use crate::array::{columns_of, to_py_err};

#[pyfunction]
pub fn crossed_above(
    left: PyReadonlyArray2<'_, f64>,
    right: PyReadonlyArray2<'_, f64>,
) -> PyResult<Vec<Vec<bool>>> {
    let (a, rows, cols) = columns_of(&left)?;
    let (b, other_rows, other_cols) = columns_of(&right)?;
    if (rows, cols) != (other_rows, other_cols) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "crossed_above inputs must have the same shape",
        ));
    }
    Ok((0..cols)
        .map(|col| {
            let start = col * rows;
            barq_generic::crossed_above_1d(&a[start..start + rows], &b[start..start + rows], 0)
        })
        .collect())
}

/// A compiled set of named factor expressions.
#[pyclass(name = "_FactorSet", module = "barq._barq")]
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
        let first = fields.first().ok_or_else(|| {
            to_py_err(barq_core::Error::Invariant(
                "factor inputs cannot be empty".into(),
            ))
        })?;
        let (_, n_rows, n_cols) = columns_of(&first.1)?;
        let mut inputs = FactorInputs::new(n_rows, n_cols).map_err(to_py_err)?;
        for (name, array) in &fields {
            let (values, rows, cols) = columns_of(array)?;
            if (rows, cols) != (n_rows, n_cols) {
                return Err(to_py_err(barq_core::Error::Invariant(format!(
                    "factor field {name:?} has shape {rows}x{cols}, expected {n_rows}x{n_cols}"
                ))));
            }
            inputs = inputs.with_field(name, values).map_err(to_py_err)?;
        }

        // The inputs borrow NumPy buffers that stay alive for this call, and
        // the evaluation touches no Python object, so other threads may run.
        let values = py
            .detach(|| self.inner.evaluate(&inputs))
            .map_err(to_py_err)?;
        let out = PyDict::new(py);
        for (name, value) in values.into_matrices() {
            out.set_item(name, value.into_vec().into_pyarray(py))?;
        }
        Ok(out)
    }
}

/// Qlib's default Alpha158 factor set.
#[pyfunction]
pub fn alpha158() -> PyResult<PyFactorSet> {
    Ok(PyFactorSet {
        inner: barq_expr::alpha158().map_err(to_py_err)?,
    })
}

/// Qlib's default Alpha360 factor set.
#[pyfunction]
pub fn alpha360() -> PyResult<PyFactorSet> {
    Ok(PyFactorSet {
        inner: barq_expr::alpha360().map_err(to_py_err)?,
    })
}
