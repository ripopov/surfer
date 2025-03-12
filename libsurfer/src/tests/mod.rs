#[cfg(not(target_arch = "wasm32"))]
mod remote;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod snapshot;
#[cfg(not(target_arch = "wasm32"))]
mod wcp;
#[cfg(not(target_arch = "wasm32"))]
mod wcp_tcp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test as _;
