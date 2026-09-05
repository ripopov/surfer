//! Opaque payload retained when a kind or its payload version is unavailable.

#[derive(Clone, Debug)]
pub struct UnknownTile {
    pub kind_name: String,
    pub kind_version: u32,
    pub payload: Box<ron::value::RawValue>,
}
