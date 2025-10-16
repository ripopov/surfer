use crate::{
    wave_container::{ScopeRef, VariableMeta, VariableRef, VariableRefExt},
    wave_data::WaveData,
};

pub fn variable_tooltip_text(meta: &Option<VariableMeta>, variable: &VariableRef) -> String {
    if let Some(meta) = meta {
        format!(
            "{}\nNum bits: {}\nType: {}\nDirection: {}",
            variable.full_path_string(),
            meta.num_bits
                .map_or_else(|| "unknown".to_string(), |bits| bits.to_string()),
            meta.variable_type_name
                .clone()
                .or_else(|| meta.variable_type.map(|t| t.to_string()))
                .unwrap_or_else(|| "unknown".to_string()),
            meta.direction
                .map_or_else(|| "unknown".to_string(), |direction| format!("{direction}"))
        )
    } else {
        variable.full_path_string()
    }
}

pub fn scope_tooltip_text(wave: &WaveData, scope: &ScopeRef) -> String {
    let other = wave.inner.as_waves().unwrap().get_scope_tooltip_data(scope);
    if other.is_empty() {
        format!("{scope}")
    } else {
        format!("{scope}\n{other}")
    }
}
