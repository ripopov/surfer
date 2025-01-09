use num::BigInt;
use serde::{Deserialize, Serialize};

/// A reference to a currently displayed item. From the protocol perspective,
/// This can be any integer or a string and what it is is decided by the server,
/// in this case surfer.
/// Since the representation is up to the server, clients cannot generate these on its
/// own, it can only use the ones it has received from the server.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(transparent)]
pub struct DisplayedItemRef(pub usize);

impl From<&DisplayedItemRef> for crate::DisplayedItemRef {
    fn from(value: &DisplayedItemRef) -> Self {
        crate::DisplayedItemRef(value.0)
    }
}
impl From<DisplayedItemRef> for crate::DisplayedItemRef {
    fn from(value: DisplayedItemRef) -> Self {
        crate::DisplayedItemRef(value.0)
    }
}
impl From<&crate::DisplayedItemRef> for DisplayedItemRef {
    fn from(value: &crate::DisplayedItemRef) -> Self {
        DisplayedItemRef(value.0)
    }
}
impl From<crate::DisplayedItemRef> for DisplayedItemRef {
    fn from(value: crate::DisplayedItemRef) -> Self {
        DisplayedItemRef(value.0)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct ItemInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub t: String,
    pub id: DisplayedItemRef,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "command")]
#[allow(non_camel_case_types)]
pub enum WcpResponse {
    get_item_list{ids: Vec<DisplayedItemRef>},
    add_variables{ids: Vec<DisplayedItemRef>},
    add_scope{ids: Vec<DisplayedItemRef>},
    get_item_info{info: Vec<ItemInfo>},
    ack,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "event")]
#[allow(non_camel_case_types)]
pub enum WcpEvent {
    waveforms_loaded,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
#[allow(non_camel_case_types)]
pub enum WcpSCMessage {
    greeting {
        version: String,
        commands: Vec<String>,
    },
    response(WcpResponse),
    error {
        error: String,
        arguments: Vec<String>,
        message: String,
    },
    event(WcpEvent),
}

impl WcpSCMessage {
    pub fn create_greeting(version: usize, commands: Vec<String>) -> Self {
        Self::greeting {
            version: version.to_string(),
            commands,
        }
    }

    pub fn create_error(error: String, arguments: Vec<String>, message: String) -> Self {
        Self::error {
            error,
            arguments,
            message,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "command")]
#[allow(non_camel_case_types)]
pub enum WcpCommand {
    get_item_list,
    get_item_info { ids: Vec<DisplayedItemRef> },
    set_item_color { id: DisplayedItemRef, color: String },
    add_variables { names: Vec<String> },
    add_scope { scope: String },
    reload,
    remove_items { ids: Vec<DisplayedItemRef> },
    focus_item { id: DisplayedItemRef },
    zoom_to_fit { viewport_idx: usize },
    set_viewport_to { timestamp: BigInt },
    clear,
    load { source: String },
    shutdowmn,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
#[allow(non_camel_case_types)]
pub enum WcpCSMessage {
    #[serde(rename = "greeting")]
    greeting {
        version: String,
        commands: Vec<String>,
    },
    command(WcpCommand),
}

impl WcpCSMessage {
    pub fn create_greeting(version: usize, commands: Vec<String>) -> Self {
        Self::greeting {
            version: version.to_string(),
            commands,
        }
    }
}
