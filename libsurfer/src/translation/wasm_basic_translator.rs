use std::sync::{Arc, Mutex};

use camino::Utf8PathBuf;
use color_eyre::{eyre::anyhow, Result};
use extism::Plugin;
use log::error;
use surfer_translation_types::{
    plugin_types::{BasicTranslateParams, BasicTranslateResult},
    translates_all_bit_types, BasicTranslator, TranslationPreference, ValueKind, VariableMeta,
    VariableValue,
};

use crate::wave_container::{ScopeId, VarId};

pub struct PluginBasicTranslator {
    plugin: Arc<Mutex<Plugin>>,
    file: Utf8PathBuf,
}

impl PluginBasicTranslator {
    pub fn new(file: Utf8PathBuf, plugin: Plugin) -> color_eyre::Result<Self> {
        Ok(Self {
            plugin: Arc::new(Mutex::new(plugin)),
            file,
        })
    }
}

impl BasicTranslator<VarId, ScopeId> for PluginBasicTranslator {
    fn name(&self) -> String {
        self.plugin
            .lock()
            .unwrap()
            .call::<_, &str>("name", ())
            .map_err(|e| {
                error!("Failed to get translator name from {}. {e}", self.file);
            })
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    fn basic_translate(
        &self,
        num_bits: u64,
        value: &VariableValue,
    ) -> (String, surfer_translation_types::ValueKind) {
        self.plugin
            .lock()
            .unwrap()
            .call(
                "basic_translate",
                BasicTranslateParams {
                    num_bits,
                    value: value.clone(),
                },
            )
            .map_err(|e| error!("Failed to translate with {}. {e}", self.file))
            .map(|BasicTranslateResult { value, kind }| (value, kind))
            .unwrap_or_else(|_| ("{Translation error}".to_string(), ValueKind::Undef))
    }

    fn translates(&self, variable: &VariableMeta<VarId, ScopeId>) -> Result<TranslationPreference> {
        let plugin = self.plugin.lock().unwrap();
        if plugin.function_exists("translates") {
            match self
                .plugin
                .lock()
                .unwrap()
                .call("translates", variable.clone().map_ids(|_| (), |_| ()))
            {
                Ok(r) => Ok(r),
                Err(e) => Err(anyhow!(e)),
            }
        } else {
            translates_all_bit_types(variable)
        }
    }
}
