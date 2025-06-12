use extism_pdk::{plugin_fn, FnResult};
pub use surfer_translation_types::plugin_types::TranslateParams;
use surfer_translation_types::{
    PluginConfig, SubFieldTranslationResult, TranslationPreference, TranslationResult, ValueKind,
    VariableInfo, VariableMeta, VariableValue,
};

#[plugin_fn]
pub fn new(PluginConfig(_config): PluginConfig) -> FnResult<()> {
    Ok(())
}

#[plugin_fn]
pub fn name() -> FnResult<String> {
    Ok("Wasm Example Translator".to_string())
}

#[plugin_fn]
pub fn translate(
    TranslateParams { variable, value }: TranslateParams,
) -> FnResult<TranslationResult> {
    let binary_digits = match value {
        VariableValue::BigUint(big_uint) => {
            let raw = format!("{big_uint:b}");
            let padding = (0..((variable.num_bits.unwrap_or_default() as usize)
                .saturating_sub(raw.len())))
                .map(|_| "0")
                .collect::<Vec<_>>()
                .join("");

            format!("{padding}{raw}")
        }
        VariableValue::String(v) => v.clone(),
    };

    let digits = binary_digits.chars().collect::<Vec<_>>();

    Ok(TranslationResult {
        val: surfer_translation_types::ValueRepr::Tuple,
        subfields: {
            digits
                .chunks(4)
                .enumerate()
                .map(|(i, chunk)| SubFieldTranslationResult {
                    name: format!("[{i}]"),
                    result: TranslationResult {
                        val: surfer_translation_types::ValueRepr::Bits(4, chunk.iter().collect()),
                        subfields: vec![],
                        kind: ValueKind::Normal,
                    },
                })
                .collect()
        },
        kind: ValueKind::Normal,
    })
}

#[plugin_fn]
pub fn variable_info(variable: VariableMeta<(), ()>) -> FnResult<VariableInfo> {
    Ok(VariableInfo::Compound {
        subfields: (0..(variable.num_bits.unwrap_or_default() / 4 + 1))
            .map(|i| (format!("[{i}]"), VariableInfo::Bits))
            .collect(),
    })
}

#[plugin_fn]
pub fn translates(_variable: VariableMeta<(), ()>) -> FnResult<TranslationPreference> {
    Ok(TranslationPreference::Yes)
}
