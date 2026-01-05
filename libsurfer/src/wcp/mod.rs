use crate::displayed_item;
use crate::displayed_item::DisplayedItemRef;

pub mod wcp_handler;
#[cfg(not(target_arch = "wasm32"))]
pub mod wcp_server;

impl From<&displayed_item::DisplayedItemRef> for surfer_wcp::DisplayedItemRef {
    fn from(value: &displayed_item::DisplayedItemRef) -> Self {
        surfer_wcp::DisplayedItemRef(value.0)
    }
}

impl From<DisplayedItemRef> for surfer_wcp::DisplayedItemRef {
    fn from(value: DisplayedItemRef) -> Self {
        surfer_wcp::DisplayedItemRef(value.0)
    }
}

impl From<&surfer_wcp::DisplayedItemRef> for DisplayedItemRef {
    fn from(value: &surfer_wcp::DisplayedItemRef) -> Self {
        DisplayedItemRef(value.0)
    }
}

impl From<surfer_wcp::DisplayedItemRef> for DisplayedItemRef {
    fn from(value: surfer_wcp::DisplayedItemRef) -> Self {
        DisplayedItemRef(value.0)
    }
}
