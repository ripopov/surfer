use crate::displayed_item::{DisplayedVariable, FieldFormat};
use crate::translation::{AnyTranslator, TranslationResultExt, TranslatorList};
use crate::wave_container::VariableMeta;
use surfer_translation_types::{Translator, VariableValue};

const MISSING_VALUE_TEXT: &str = "\u{2014}";

pub(crate) struct SignalValueFormatter {
    translator: AnyTranslator,
    translators: TranslatorList,
    root_format: Option<String>,
    field_formats: Vec<FieldFormat>,
}

pub(crate) fn resolve_signal_value_formatter(
    displayed_variable: Option<&DisplayedVariable>,
    field: &[String],
    translators: &TranslatorList,
    preferred_root_translator: impl FnOnce() -> String,
) -> SignalValueFormatter {
    let root_format = displayed_variable.and_then(|var| var.format.clone());
    let field_formats = displayed_variable
        .map(|var| var.field_formats.clone())
        .unwrap_or_default();

    let format_override = displayed_variable
        .and_then(|var| var.get_format(field))
        .cloned();
    let translator_name = format_override.unwrap_or_else(|| {
        if field.is_empty() {
            preferred_root_translator()
        } else {
            translators.default.clone()
        }
    });

    SignalValueFormatter {
        translator: translators.clone_translator(&translator_name),
        translators: translators.clone(),
        root_format,
        field_formats,
    }
}

pub(crate) fn format_signal_value(
    formatter: &SignalValueFormatter,
    meta: &VariableMeta,
    field: &[String],
    value: &VariableValue,
) -> (String, Option<f64>) {
    let numeric = if field.is_empty() {
        formatter.translator.translate_numeric(meta, value)
    } else {
        None
    };

    match formatter.translator.translate(meta, value) {
        Ok(translated) => {
            let fields = translated.format_flat(
                &formatter.root_format,
                &formatter.field_formats,
                &formatter.translators,
            );
            let match_field = fields.iter().find(|res| res.names == field);
            match match_field.and_then(|res| res.value.as_ref()) {
                Some(value) => (value.value.clone(), numeric),
                None => (MISSING_VALUE_TEXT.to_string(), numeric),
            }
        }
        Err(_) => (MISSING_VALUE_TEXT.to_string(), numeric),
    }
}
