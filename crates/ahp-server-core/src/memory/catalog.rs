//! Derived catalogs are journaled in the same transaction as their chat state.

use ahp_types::actions::StateAction;
use ahp_types::state::SessionInputRequest;

use super::state::{Data, Session};
use super::{transitions::summary_draft, Draft};

impl Data {
    pub(super) fn input_requests(&self, session: &Session) -> Vec<SessionInputRequest> {
        session
            .chats
            .iter()
            .filter_map(|uri| self.chats.get(uri))
            .flat_map(|chat| {
                chat.active
                    .as_ref()
                    .map(|active| {
                        crate::catalog::chat_input_requests(
                            &chat.uri,
                            &active.id,
                            &active.response_parts,
                        )
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    pub(super) fn reconcile_catalogs(&mut self, before: &Self, drafts: &mut Vec<Draft>) {
        let mut updates = Vec::new();
        for (uri, session) in &self.sessions {
            let previous = before
                .sessions
                .get(uri)
                .map(|session| session.input_needed.clone())
                .unwrap_or_default();
            let requests = self.input_requests(session);
            updates.push((
                uri.clone(),
                crate::catalog::updated_input_requests(&previous, &requests),
            ));
            for action in crate::catalog::input_request_actions(&previous, &requests) {
                drafts.push(Draft::action(uri.clone(), action, None));
            }
            for chat_uri in &session.chats {
                let Some(chat) = self.chats.get(chat_uri) else {
                    continue;
                };
                let summary = self.chat_summary(chat);
                let changed = before
                    .chats
                    .get(chat_uri)
                    .is_some_and(|previous| before.chat_summary(previous) != summary);
                let already_updated = drafts.iter().any(|draft| {
                    matches!(&draft.action,
                    StateAction::SessionChatUpdated(action) if action.chat == *chat_uri)
                });
                if changed && !already_updated {
                    drafts.push(summary_draft(self, chat_uri, summary));
                }
            }
        }
        for (uri, session) in &before.sessions {
            if !self.sessions.contains_key(uri) {
                for action in crate::catalog::input_request_actions(&session.input_needed, &[]) {
                    drafts.push(Draft::action(uri.clone(), action, None));
                }
            }
        }
        for (uri, requests) in updates {
            if let Some(session) = self.sessions.get_mut(&uri) {
                session.input_needed = requests;
            }
        }
    }
}
