//! Narrow Arrow C Data boundary. Only concrete PyArrow objects are imported.
//! Capsules own their release callbacks; imported buffers retain that ownership.
use arrow_array::{
    Array, RecordBatch, StructArray,
    ffi::{FFI_ArrowArray, from_ffi},
};
use arrow_schema::{Schema, SchemaRef, ffi::FFI_ArrowSchema};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::PyCapsule,
};
use std::sync::Arc;
fn bad(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn check(value: &Bound<'_, PyAny>, class: &str) -> PyResult<()> {
    let expected = value.py().import("pyarrow")?.getattr(class)?;
    if !value.is_exact_instance(&expected) {
        return Err(PyTypeError::new_err(format!("expected pyarrow.{class}")));
    }
    Ok(())
}

pub fn schema(value: &Bound<'_, PyAny>) -> PyResult<SchemaRef> {
    check(value, "Schema")?;
    let capsule = value.call_method0("__arrow_c_schema__")?;
    let capsule = capsule.cast::<PyCapsule>()?;
    let ptr = capsule.pointer_checked(Some(c"arrow_schema"))?;
    // SAFETY: a concrete PyArrow Schema produced this named capsule, kept alive
    // until Schema::try_from has copied all schema metadata.
    let ffi = unsafe { &*ptr.as_ptr().cast::<FFI_ArrowSchema>() };
    Ok(Arc::new(Schema::try_from(ffi).map_err(bad)?))
}

pub fn import(value: &Bound<'_, PyAny>) -> PyResult<RecordBatch> {
    check(value, "RecordBatch")?;
    let capsules = value.call_method0("__arrow_c_array__")?;
    let (schema, array): (Bound<'_, PyCapsule>, Bound<'_, PyCapsule>) = capsules.extract()?;
    let sp = schema.pointer_checked(Some(c"arrow_schema"))?;
    let ap = array.pointer_checked(Some(c"arrow_array"))?;
    // SAFETY: capsules originate from concrete PyArrow RecordBatch. from_raw
    // moves array ownership and empties the capsule, preventing double release.
    // The schema stays alive throughout from_ffi; Arrow retains buffer owners.
    let (schema, data) = unsafe {
        let schema = &*sp.as_ptr().cast::<FFI_ArrowSchema>();
        let array = FFI_ArrowArray::from_raw(ap.as_ptr().cast());
        (
            Schema::try_from(schema).map_err(bad)?,
            from_ffi(array, schema).map_err(bad)?,
        )
    };
    let array = StructArray::from(data);
    RecordBatch::try_new(Arc::new(schema), array.columns().to_vec()).map_err(bad)
}

pub fn export<'py>(py: Python<'py>, batch: RecordBatch) -> PyResult<Bound<'py, PyAny>> {
    let schema = FFI_ArrowSchema::try_from(batch.schema().as_ref()).map_err(bad)?;
    let data = StructArray::from(batch).to_data();
    let array = FFI_ArrowArray::new(&data);
    let schema = PyCapsule::new_with_value(py, schema, c"arrow_schema")?;
    let array = PyCapsule::new_with_value(py, array, c"arrow_array")?;
    py.import("pyarrow")?
        .getattr("RecordBatch")?
        .call_method1("_import_from_c_capsule", (schema, array))
}
