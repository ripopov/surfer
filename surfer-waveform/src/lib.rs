use pyo3::prelude::*;

pub mod python;
pub mod registry;

#[pymodule]
#[pyo3(name = "surfer_waveform")]
pub fn surfer_pyo3_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<python::PythonBasicTranslator>()?;
    m.add_class::<python::PythonValueKind>()?;

    // Add registry functions
    m.add_function(wrap_pyfunction!(registry::register_translator, m)?)?;
    m.add_function(wrap_pyfunction!(registry::is_translator_registered, m)?)?;
    m.add_function(wrap_pyfunction!(registry::unregister_translator, m)?)?;
    m.add_function(wrap_pyfunction!(registry::clear_translator_registry, m)?)?;
    m.add_function(wrap_pyfunction!(
        registry::get_translator_registry_count,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        registry::get_all_registered_translators,
        m
    )?)?;
    Ok(())
}
