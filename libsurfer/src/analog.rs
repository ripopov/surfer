use derive_more::Display;
use enum_iterator::Sequence;
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Copy, Clone, Debug, Deserialize, Display, Serialize, Sequence)]
pub enum AnalogDisplayMode {
    #[display("Step")]
    Step,

    #[display("Linear Interpolation")]
    LinearInterpolation,
}

#[derive(PartialEq, Copy, Clone, Debug, Deserialize, Display, Serialize, Sequence)]
pub enum AnalogRangeMode {
    #[display("Type Range")]
    TypeRange,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AnalogViewSettings {
    pub enabled: bool,
    pub mode: AnalogDisplayMode,
    pub range: AnalogRangeMode,
}

impl Default for AnalogViewSettings {
    fn default() -> Self {
        AnalogViewSettings {
            enabled: false,
            mode: AnalogDisplayMode::Step,
            range: AnalogRangeMode::TypeRange,
        }
    }
}
