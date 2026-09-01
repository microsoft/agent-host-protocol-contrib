//! Physical table names and fixed retention limits.

use redb::TableDefinition;

pub(crate) const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
pub(crate) const SESSIONS: TableDefinition<&str, &str> = TableDefinition::new("sessions");
pub(crate) const CHATS: TableDefinition<&str, &str> = TableDefinition::new("chats");
pub(crate) const ACTIONS: TableDefinition<i64, &str> = TableDefinition::new("actions");

pub(crate) const SERVER_SEQUENCE: &str = "server_seq";
pub(crate) const FIRST_ACTION_SEQUENCE: &str = "first_action_seq";
pub(crate) const SESSION_IDS: &str = "session_ids";
pub(crate) const EXACT_COMMAND_APPROVALS: &str = "exact_command_approvals";

pub(crate) const REPLAY_LIMIT: i64 = 2_048;
pub(crate) const TURN_HISTORY_LIMIT: usize = 256;
