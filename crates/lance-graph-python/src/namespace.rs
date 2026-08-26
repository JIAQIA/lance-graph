use std::collections::HashMap;
use std::sync::Arc;

use tf_lance_graph::DirNamespace;
use pyo3::prelude::*;

#[pyclass(name = "DirNamespace", module = "lance.graph")]
pub struct PyDirNamespace {
    pub(crate) inner: Arc<DirNamespace>,
}

#[pymethods]
impl PyDirNamespace {
    /// Create a directory-backed namespace rooted at `base_uri`.
    ///
    /// `storage_options` (optional) carries explicit object-store options
    /// (e.g. `endpoint`/`access_key_id`/`region`/`allow_http`) through
    /// `describe_table` so downstream openers can configure the storage backend
    /// explicitly instead of relying on ambient AWS env (TFROB-806). Dict keys
    /// and values are strings; non-string values are rejected by the type.
    #[new]
    #[pyo3(signature = (base_uri, storage_options=None))]
    fn new(base_uri: String, storage_options: Option<HashMap<String, String>>) -> Self {
        let inner = match storage_options {
            Some(opts) => DirNamespace::new_with_storage_options(base_uri, Some(opts)),
            None => DirNamespace::new(base_uri),
        };
        Self { inner: Arc::new(inner) }
    }

    #[getter]
    fn base_uri(&self) -> String {
        self.inner.base_uri().to_string()
    }
}
