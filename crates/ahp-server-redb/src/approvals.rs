//! Exact-command approval persistence.

use ahp_server_core::StoreError;
use redb::ReadableTable;

use crate::db::{backend, json_error};
use crate::tables::{EXACT_COMMAND_APPROVALS, META};
use crate::RedbHostStore;

impl RedbHostStore {
    pub(crate) fn command_is_allowed(
        &self,
        approval: &ahp_server_core::workspace_agent_tools::ExactCommandApproval,
    ) -> Result<bool, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let meta = read.open_table(META).map_err(backend)?;
        let approvals = read_approvals_read(&meta)?;
        Ok(approvals.iter().any(|stored| stored == approval.as_str()))
    }

    pub(crate) fn reset_exact_command_approvals(&self) -> Result<(), StoreError> {
        let database = self.database()?;
        let write = database.get()?.begin_write().map_err(backend)?;
        let mut meta = write.open_table(META).map_err(backend)?;
        meta.insert(EXACT_COMMAND_APPROVALS, "[]")
            .map_err(backend)?;
        drop(meta);
        write.commit().map_err(backend)
    }
}

pub(crate) fn add_approval(
    write: &redb::WriteTransaction,
    approval: &ahp_server_core::workspace_agent_tools::ExactCommandApproval,
) -> Result<(), StoreError> {
    let mut meta = write.open_table(META).map_err(backend)?;
    let mut approvals = read_approvals(&meta)?;
    if !approvals.iter().any(|stored| stored == approval.as_str()) {
        approvals.push(approval.as_str().to_owned());
        approvals.sort_unstable();
        let serialized = serde_json::to_string(&approvals).map_err(json_error)?;
        meta.insert(EXACT_COMMAND_APPROVALS, serialized.as_str())
            .map_err(backend)?;
    }
    drop(meta);
    Ok(())
}

fn read_approvals(meta: &redb::Table<&str, &str>) -> Result<Vec<String>, StoreError> {
    let value = meta
        .get(EXACT_COMMAND_APPROVALS)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend("exact command approvals metadata is missing".into()))?;
    serde_json::from_str(value.value()).map_err(json_error)
}

fn read_approvals_read(meta: &redb::ReadOnlyTable<&str, &str>) -> Result<Vec<String>, StoreError> {
    let value = meta
        .get(EXACT_COMMAND_APPROVALS)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend("exact command approvals metadata is missing".into()))?;
    serde_json::from_str(value.value()).map_err(json_error)
}
