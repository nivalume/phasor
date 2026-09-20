//! Arrow exchange and Python exports; all row processing stays in barq-data.
use crate::arrow;
use barq_data::{Aggregate, DataSchema, Dataset, DecodeContext, InstrumentBinding, SourceItem};
use barq_model::{BarType, ids::InstrumentId};
use numpy::IntoPyArray;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::path::PathBuf;

fn bad(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

#[pyclass(
    name = "InstrumentId",
    module = "barq",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PyInstrumentId(pub InstrumentId);
#[pymethods]
impl PyInstrumentId {
    #[new]
    fn new(symbol: &str, venue: &str) -> PyResult<Self> {
        Ok(Self(InstrumentId::from_parts(symbol, venue).map_err(bad)?))
    }
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        Ok(Self(value.parse().map_err(bad)?))
    }
    #[getter]
    fn symbol(&self) -> &str {
        self.0.symbol.as_str()
    }
    #[getter]
    fn venue(&self) -> &str {
        self.0.venue.as_str()
    }
    fn __str__(&self) -> String {
        self.0.to_string()
    }
    fn __repr__(&self) -> String {
        format!("InstrumentId('{}')", self.0)
    }
}

#[pyclass(
    name = "BarType",
    module = "barq",
    frozen,
    eq,
    hash,
    skip_from_py_object
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PyBarType(pub BarType);
#[pymethods]
impl PyBarType {
    #[staticmethod]
    fn parse(value: &str) -> PyResult<Self> {
        Ok(Self(value.parse().map_err(bad)?))
    }
    #[getter]
    fn instrument(&self) -> PyInstrumentId {
        PyInstrumentId(self.0.instrument.clone())
    }
    #[getter]
    fn step(&self) -> u64 {
        self.0.step
    }
    #[getter]
    fn aggregation(&self) -> &str {
        &self.0.aggregation
    }
    #[getter]
    fn price_type(&self) -> &str {
        &self.0.price_type
    }
    #[getter]
    fn source(&self) -> &str {
        &self.0.source
    }
    fn __str__(&self) -> String {
        self.0.to_string()
    }
    fn __repr__(&self) -> String {
        format!("BarType('{}')", self.0)
    }
}

#[pyclass(
    name = "DecodeContext",
    module = "barq.data",
    frozen,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
pub struct PyContext(DecodeContext);
#[pymethods]
impl PyContext {
    #[getter]
    fn key(&self) -> &str {
        &self.0.key
    }
    #[getter]
    fn path(&self) -> String {
        self.0.path.to_string_lossy().into_owned()
    }
    #[getter]
    fn instrument(&self) -> Option<PyInstrumentId> {
        self.0.instrument.clone().map(PyInstrumentId)
    }
    #[getter]
    fn bar_type(&self) -> Option<PyBarType> {
        self.0.bar_type.clone().map(PyBarType)
    }
}

#[pyclass(name = "_Dataset", frozen)]
#[derive(Debug)]
pub struct PyDataset(Dataset);
#[pymethods]
impl PyDataset {
    fn prepare<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let panel = py.detach(|| self.0.prepare()).map_err(bad)?;
        let result = PyDict::new(py);
        result.set_item("time", panel.time.into_pyarray(py))?;
        result.set_item(
            "bar_types",
            panel
                .bar_types
                .into_iter()
                .map(PyBarType)
                .collect::<Vec<_>>(),
        )?;
        let p = panel.values;
        for (name, values) in [
            ("open", p.open),
            ("high", p.high),
            ("low", p.low),
            ("close", p.close),
            ("volume", p.volume),
        ] {
            result.set_item(name, values.into_pyarray(py))?;
        }
        Ok(result)
    }
    #[getter]
    fn batches(&self, py: Python<'_>) -> PyResult<Vec<(PyContext, Py<PyAny>)>> {
        self.0
            .batches
            .iter()
            .map(|part| {
                Ok((
                    PyContext(part.context.clone()),
                    arrow::export(py, part.batch.clone())?.unbind(),
                ))
            })
            .collect()
    }
    #[pyo3(signature = (time, instruments, field, window=None, agg="mean", empty=f64::NAN))]
    fn align<'py>(
        &self,
        py: Python<'py>,
        time: Vec<u64>,
        instruments: Vec<String>,
        field: &str,
        window: Option<u64>,
        agg: &str,
        empty: f64,
    ) -> PyResult<Bound<'py, numpy::PyArray1<f64>>> {
        let instruments = instruments
            .iter()
            .map(|s| s.parse().map_err(bad))
            .collect::<PyResult<Vec<_>>>()?;
        let agg = match agg {
            "mean" => Aggregate::Mean,
            "sum" => Aggregate::Sum,
            "count" => Aggregate::Count,
            _ => return Err(bad("agg must be mean, sum, or count")),
        };
        let values = py
            .detach(|| {
                self.0
                    .align(&time, &instruments, field, window.map(|w| (w, agg)), empty)
            })
            .map_err(bad)?;
        Ok(values.into_pyarray(py))
    }
}

#[pyfunction]
pub fn catalog_files(
    root: PathBuf,
    name: &str,
    keys: Vec<String>,
) -> PyResult<Vec<(String, Vec<PathBuf>)>> {
    Ok(barq_data::catalog(&root, name, &keys)
        .map_err(bad)?
        .into_iter()
        .map(|i| (i.key, i.paths))
        .collect())
}

#[pyfunction]
#[pyo3(signature=(items, schema, start=None, end=None))]
pub fn load_data(
    py: Python<'_>,
    items: Vec<(String, Vec<PathBuf>)>,
    schema: &Bound<'_, PyAny>,
    start: Option<u64>,
    end: Option<u64>,
) -> PyResult<PyDataset> {
    let binding = schema.getattr("instrument")?;
    let instrument = if binding.is_none() {
        InstrumentBinding::Global
    } else if binding.extract::<String>().is_ok_and(|s| s == "source") {
        InstrumentBinding::Source
    } else {
        let (symbol, venue) = binding.extract::<(String, String)>()?;
        InstrumentBinding::Columns { symbol, venue }
    };
    let decoder = schema.getattr("decode")?.unbind();
    let schema = DataSchema {
        name: schema.getattr("name")?.extract()?,
        input: arrow::schema(&schema.getattr("input")?)?,
        output: arrow::schema(&schema.getattr("output")?)?,
        event_time: schema.getattr("event_time")?.extract()?,
        available_time: schema.getattr("available_time")?.extract()?,
        instrument,
    };
    let items = items
        .into_iter()
        .map(|(key, paths)| SourceItem { key, paths })
        .collect::<Vec<_>>();
    let dataset = py
        .detach(move || {
            barq_data::load(schema, &items, start, end, |batch, context| {
                Python::attach(|py| -> PyResult<_> {
                    let batch = arrow::export(py, batch)?;
                    let decoded = decoder.call1(py, (batch, PyContext(context.clone())))?;
                    arrow::import(decoded.bind(py))
                })
                .map_err(|e| {
                    arrow_schema::ArrowError::InvalidArgumentError(format!("decoder failed: {e}"))
                })
            })
        })
        .map_err(bad)?;
    Ok(PyDataset(dataset))
}
