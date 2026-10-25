//! Atomic session metadata and active-client transitions.

use ahp_types::state::SessionStatus;

use super::{Draft, MemoryHostStore};
use crate::store::{ChangeSession, CommittedTransition, SessionMutation, StoreError};

pub(super) fn change_session(
    store: &MemoryHostStore,
    request: ChangeSession,
) -> Result<CommittedTransition<()>, StoreError> {
    store.transact(move |data| {
        let session = data.session_mut(&request.session)?;
        match &request.mutation {
            SessionMutation::Title(title) => session.title.clone_from(title),
            SessionMutation::IsRead(value) => {
                set_flag(
                    &mut session.status_flags,
                    SessionStatus::IsRead.bits(),
                    *value,
                );
            }
            SessionMutation::IsArchived(value) => {
                set_flag(
                    &mut session.status_flags,
                    SessionStatus::IsArchived.bits(),
                    *value,
                );
            }
            SessionMutation::ActiveClientSet(client) => {
                if let Some(existing) = session
                    .active_clients
                    .iter_mut()
                    .find(|entry| entry.client_id == client.client_id)
                {
                    existing.clone_from(client);
                } else {
                    session.active_clients.push(client.clone());
                }
            }
            SessionMutation::ActiveClientRemoved(client_id) => {
                session
                    .active_clients
                    .retain(|client| client.client_id != *client_id);
            }
        }
        Ok((
            (),
            vec![Draft::action(
                request.session,
                request.mutation.action(),
                request.origin,
            )],
        ))
    })
}

fn set_flag(status: &mut u32, flag: u32, value: bool) {
    if value {
        *status |= flag;
    } else {
        *status &= !flag;
    }
}
