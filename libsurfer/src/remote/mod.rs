mod client;

use serde::{Deserialize, Serialize};

pub(crate) use client::get_transaction_projection;
pub use client::{
    ReloadError, get_hierarchy_from_server, get_server_status, get_signals,
    get_time_table_from_server, get_transactions_from_server, server_reload,
};

#[derive(Serialize, Deserialize)]
pub struct HierarchyResponse {
    pub hierarchy: wellen::Hierarchy,
    pub file_format: wellen::FileFormat,
}
