use eyre::Result;
use eyre::anyhow;
use pyo3::types::{PyAnyMethods, PyDict, PyModule, PyStringMethods};
use pyo3::{Bound, Py, Python};
use std::ffi::{CStr, CString};
use surfer_translation_types::python::{PythonValueKind, surfer_pyo3_module};
use surfer_translation_types::{BasicTranslator, ValueKind, VariableValue};
use tracing::{error, info};

use crate::wave_container::{ScopeId, VarId};

pub struct PythonTranslator {
    module: Py<PyModule>,
    class_name: String,
}

impl PythonTranslator {
    pub fn new(code: &CStr) -> Result<Vec<Self>> {
        info!("Loading Python translator");
        Python::attach(|py| -> pyo3::PyResult<_> {
            let surfer_module = PyModule::new(py, "surfer")?;
            surfer_pyo3_module(&surfer_module)?;
            let sys = PyModule::import(py, "sys")?;
            let py_modules: Bound<'_, PyDict> = sys.getattr("modules")?.cast_into()?;
            py_modules.set_item("surfer", surfer_module)?;
            let filename = CString::new("plugin.py")?;
            let modulename = CString::new("plugin")?;
            let module = PyModule::from_code(py, code, filename.as_c_str(), modulename.as_c_str())?;

            let translators = module
                .getattr("translators")?
                .try_iter()?
                .map(|t| Ok(t?.str()?.to_string_lossy().to_string()))
                .collect::<pyo3::PyResult<Vec<_>>>()?;

            //            let module = module.unbind();
            Ok(translators
                .into_iter()
                .map(|class_name| Self {
                    module: module.clone().unbind(),
                    class_name,
                })
                .collect())
        })
        .map_err(|e| anyhow!("Error initializing Python translator: {e}"))
    }
}

impl BasicTranslator<VarId, ScopeId> for PythonTranslator {
    fn name(&self) -> String {
        let name = Python::attach(|py| {
            self.module
                .bind(py)
                .getattr(self.class_name.as_str())
                .unwrap()
                .getattr("name")
                .unwrap()
                .str()
                .unwrap()
                .to_string_lossy()
                .to_string()
        });
        name
    }

    fn basic_translate(&self, num_bits: u32, value: &VariableValue) -> (String, ValueKind) {
        let result = Python::attach(|py| -> pyo3::PyResult<_> {
            let ret = self
                .module
                .bind(py)
                .getattr(self.class_name.as_str())?
                .getattr("basic_translate")?
                .call((num_bits, value.to_string()), None)?;
            let ret = ret.cast()?;
            let v = ret.get_item(0).unwrap().extract().unwrap();
            let k = ValueKind::from(ret.get_item(1)?.cast::<PythonValueKind>()?.get().clone());
            Ok((v, k))
        });
        match result {
            Ok((v, k)) => (v, k),
            Err(e) => {
                error!(
                    "Could not translate '{}' with Python translator '{}': {}",
                    value,
                    self.name(),
                    e
                );
                (value.to_string(), ValueKind::Normal)
            }
        }
    }
}
