//! Crash-atomic session metadata and active-client changes.

use ahp_server_core::{
    ChangeSession, CommittedTransition, SessionMutation, StoreConflict, StoreError,
};
use ahp_types::state::SessionStatus;

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_session, write_session};
use crate::RedbHostStore;

impl RedbHostStore {
    pub(crate) fn change_session_transition(
        &self,
        request: ChangeSession,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.commit_transition(move |write| {
            let mut session = load_session(write, &request.session)?
                .ok_or_else(|| StoreConflict::UnknownChannel(request.session.clone()))?;
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
            write_session(write, &session)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(
                    request.session,
                    request.mutation.action(),
                    request.origin,
                )],
            ))
        })
    }
}

fn set_flag(status: &mut u32, flag: u32, value: bool) {
    if value {
        *status |= flag;
    } else {
        *status &= !flag;
    }
}
