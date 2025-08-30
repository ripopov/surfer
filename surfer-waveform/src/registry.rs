use crate::python::PythonBasicTranslator;
use pyo3::prelude::*;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

// Global singleton instance
static PYTHON_TRANSLATOR_REGISTRY: OnceLock<Mutex<PythonTranslatorRegistry>> = OnceLock::new();

// Internal registry structure
#[derive(Debug)]
struct PythonTranslatorRegistry {
    items: HashSet<PythonBasicTranslator>,
}

impl PythonTranslatorRegistry {
    fn new() -> Self {
        Self {
            items: HashSet::new(),
        }
    }

    fn register(&mut self, item: PythonBasicTranslator) -> bool {
        self.items.insert(item)
    }

    fn is_registered(&self, item: &PythonBasicTranslator) -> bool {
        self.items.contains(item)
    }

    fn unregister(&mut self, item: &PythonBasicTranslator) -> bool {
        self.items.remove(item)
    }

    fn clear(&mut self) {
        self.items.clear();
    }

    fn count(&self) -> usize {
        self.items.len()
    }

    fn get_all(&self) -> Vec<PythonBasicTranslator> {
        self.items.iter().cloned().collect()
    }
}

// Python wrapper for the singleton
#[pyclass]
pub struct TranslatorRegistrySingleton;

#[pymethods]
impl TranslatorRegistrySingleton {
    #[new]
    fn new() -> Self {
        // Initialize the singleton if it doesn't exist
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
        Self
    }

    /// Register an item in the singleton set
    /// Returns True if the item was newly added, False if it already existed
    fn register(&self, item: PythonBasicTranslator) -> PyResult<bool> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let mut registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(registry.register(item))
    }

    /// Check if an item is registered
    fn is_registered(&self, item: &PythonBasicTranslator) -> PyResult<bool> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(registry.is_registered(item))
    }

    /// Unregister an item from the set
    /// Returns True if the item was removed, False if it wasn't present
    fn unregister(&self, item: &PythonBasicTranslator) -> PyResult<bool> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let mut registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(registry.unregister(item))
    }

    /// Clear all items from the registry
    fn clear(&self) -> PyResult<()> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let mut registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        registry.clear();
        Ok(())
    }

    /// Get the number of registered items
    fn count(&self) -> PyResult<usize> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(registry.count())
    }

    /// Get all registered items as a list
    fn get_all(&self) -> PyResult<Vec<PythonBasicTranslator>> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(registry.get_all())
    }

    /// String representation for debugging
    fn __repr__(&self) -> PyResult<String> {
        let registry = PYTHON_TRANSLATOR_REGISTRY.get().unwrap();
        let registry = registry.lock().map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
        })?;
        Ok(format!("RegistrySingleton(items={})", registry.count()))
    }

    /// Length support for Python len() function
    fn __len__(&self) -> PyResult<usize> {
        self.count()
    }

    /// Contains support for Python 'in' operator
    fn __contains__(&self, item: &PythonBasicTranslator) -> PyResult<bool> {
        self.is_registered(item)
    }
}

// Convenience functions for direct access without creating instances
#[pyfunction]
pub fn register_translator(item: PythonBasicTranslator) -> PyResult<bool> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let mut registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    Ok(registry.register(item))
}

#[pyfunction]
pub fn is_translator_registered(item: &PythonBasicTranslator) -> PyResult<bool> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    Ok(registry.is_registered(item))
}

#[pyfunction]
pub fn unregister_translator(item: &PythonBasicTranslator) -> PyResult<bool> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let mut registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    Ok(registry.unregister(item))
}

#[pyfunction]
pub fn clear_translator_registry() -> PyResult<()> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let mut registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    registry.clear();
    Ok(())
}

#[pyfunction]
pub fn get_translator_registry_count() -> PyResult<usize> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    Ok(registry.count())
}

#[pyfunction]
pub fn get_all_registered_translators() -> PyResult<Vec<PythonBasicTranslator>> {
    let registry =
        PYTHON_TRANSLATOR_REGISTRY.get_or_init(|| Mutex::new(PythonTranslatorRegistry::new()));
    let registry = registry.lock().map_err(|_| {
        pyo3::exceptions::PyRuntimeError::new_err("Failed to acquire registry lock")
    })?;
    Ok(registry.get_all())
}
