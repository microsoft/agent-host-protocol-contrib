//! Database lifetime, initialization, and record primitives.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use ahp_server_core::StoreError;
use ahp_types::common::Uri;
use redb::{Database, ReadableTable, TableDefinition, WriteTransaction};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::tables::{
    EXACT_COMMAND_APPROVALS, FIRST_ACTION_SEQUENCE, META, SERVER_SEQUENCE, SESSION_IDS,
};
use crate::types::{PersistedChat, PersistedSession};
use crate::RedbHostStore;

pub(crate) struct DatabaseGuard<'a>(MutexGuard<'a, Option<Database>>);

impl DatabaseGuard<'_> {
    pub(crate) fn get(&self) -> Result<&Database, StoreError> {
        self.0
            .as_ref()
            .ok_or_else(|| StoreError::Backend("redb store is closed".into()))
    }
}

impl RedbHostStore {
    pub(crate) fn open_database(path: &Path) -> Result<Self, StoreError> {
        let store = Self {
            database: Mutex::new(Some(Database::create(path).map_err(backend)?)),
        };
        store.initialize()?;
        store.recover_incomplete_turns()?;
        Ok(store)
    }

    pub(crate) fn database(&self) -> Result<DatabaseGuard<'_>, StoreError> {
        self.database
            .lock()
            .map(DatabaseGuard)
            .map_err(|_| StoreError::Backend("redb store mutex is poisoned".into()))
    }

    pub(crate) fn initialize(&self) -> Result<(), StoreError> {
        let database = self.database()?;
        let write = database.get()?.begin_write().map_err(backend)?;
        let mut meta = write.open_table(META).map_err(backend)?;
        initialize_meta(&mut meta, SERVER_SEQUENCE, "0")?;
        initialize_meta(&mut meta, FIRST_ACTION_SEQUENCE, "1")?;
        initialize_meta(&mut meta, SESSION_IDS, "[]")?;
        initialize_meta(&mut meta, EXACT_COMMAND_APPROVALS, "[]")?;
        drop(meta);
        write.commit().map_err(backend)
    }

    pub(crate) fn close_database(&self) -> Result<(), StoreError> {
        self.database
            .lock()
            .map_err(|_| StoreError::Backend("redb store mutex is poisoned".into()))?
            .take();
        Ok(())
    }

    pub(crate) fn get_session(&self, uri: &str) -> Result<Option<PersistedSession>, StoreError> {
        self.get_record(crate::tables::SESSIONS, uri)
    }

    pub(crate) fn get_chat(&self, uri: &str) -> Result<Option<PersistedChat>, StoreError> {
        self.get_record(crate::tables::CHATS, uri)
    }

    pub(crate) fn session_ids(&self) -> Result<Vec<Uri>, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        read_session_ids(&read)
    }

    fn get_record<T>(
        &self,
        table_definition: TableDefinition<&str, &str>,
        key: &str,
    ) -> Result<Option<T>, StoreError>
    where
        T: DeserializeOwned,
    {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        read_record(&read, table_definition, key)
    }

    #[cfg(test)]
    pub(crate) fn test_write<T>(
        &self,
        operation: impl FnOnce(&WriteTransaction) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let database = self.database()?;
        let write = database.get()?.begin_write().map_err(backend)?;
        let value = operation(&write)?;
        write.commit().map_err(backend)?;
        Ok(value)
    }
}

fn initialize_meta(
    meta: &mut redb::Table<&str, &str>,
    key: &str,
    value: &str,
) -> Result<(), StoreError> {
    if meta.get(key).map_err(backend)?.is_none() {
        meta.insert(key, value).map_err(backend)?;
    }
    Ok(())
}

pub(crate) fn read_record<T>(
    read: &redb::ReadTransaction,
    table_definition: TableDefinition<&str, &str>,
    key: &str,
) -> Result<Option<T>, StoreError>
where
    T: DeserializeOwned,
{
    let table = read.open_table(table_definition).map_err(backend)?;
    let value = table.get(key).map_err(backend)?;
    let result = value
        .map(|value| serde_json::from_str(value.value()).map_err(json_error))
        .transpose();
    drop(table);
    result
}

pub(crate) fn read_record_from_write<T>(
    write: &WriteTransaction,
    table_definition: TableDefinition<&str, &str>,
    key: &str,
) -> Result<Option<T>, StoreError>
where
    T: DeserializeOwned,
{
    let table = write.open_table(table_definition).map_err(backend)?;
    let value = table.get(key).map_err(backend)?;
    let result = value
        .map(|value| serde_json::from_str(value.value()).map_err(json_error))
        .transpose();
    drop(table);
    result
}

pub(crate) fn write_record<T: Serialize>(
    write: &WriteTransaction,
    table_definition: TableDefinition<&str, &str>,
    key: &str,
    record: &T,
) -> Result<(), StoreError> {
    let serialized = serde_json::to_string(record).map_err(json_error)?;
    let mut table = write.open_table(table_definition).map_err(backend)?;
    table.insert(key, serialized.as_str()).map_err(backend)?;
    drop(table);
    Ok(())
}

pub(crate) fn read_session_ids(read: &redb::ReadTransaction) -> Result<Vec<Uri>, StoreError> {
    let meta = read.open_table(META).map_err(backend)?;
    let value = meta
        .get(SESSION_IDS)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend("session ids metadata is missing".into()))?;
    let result = serde_json::from_str(value.value()).map_err(json_error);
    drop(value);
    drop(meta);
    result
}

pub(crate) fn read_session_ids_from_write(
    write: &WriteTransaction,
) -> Result<Vec<Uri>, StoreError> {
    let meta = write.open_table(META).map_err(backend)?;
    let value = meta
        .get(SESSION_IDS)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend("session ids metadata is missing".into()))?;
    let result = serde_json::from_str(value.value()).map_err(json_error);
    drop(value);
    drop(meta);
    result
}

pub(crate) fn write_session_ids(
    write: &WriteTransaction,
    session_ids: &[Uri],
) -> Result<(), StoreError> {
    let serialized = serde_json::to_string(session_ids).map_err(json_error)?;
    let mut meta = write.open_table(META).map_err(backend)?;
    meta.insert(SESSION_IDS, serialized.as_str())
        .map_err(backend)?;
    drop(meta);
    Ok(())
}

pub(crate) fn backend(error: impl ToString) -> StoreError {
    StoreError::Backend(error.to_string())
}

pub(crate) fn json_error(error: serde_json::Error) -> StoreError {
    StoreError::Backend(format!("invalid persisted JSON: {error}"))
}
