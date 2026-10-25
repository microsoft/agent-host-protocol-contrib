//! Transaction-local projections of session chat catalogs and pending input.

use std::collections::{BTreeMap, BTreeSet};

use ahp_server_core::{
    catalog::{chat_input_requests, input_request_actions},
    StoreError,
};
use ahp_types::common::Uri;
use ahp_types::state::{ChatSummary, SessionInputRequest};

use crate::db::read_record;
use crate::replay::Draft;
use crate::sessions::{load_chat, load_session, write_session};
use crate::transition::summary_draft;
use crate::types::{chat_summary, PersistedChat, PersistedSession};

pub(crate) struct Catalog {
    chats: Vec<PersistedChat>,
    requests: Vec<SessionInputRequest>,
    stored_requests: Vec<SessionInputRequest>,
}

pub(crate) fn affected_sessions(
    read: &redb::ReadTransaction,
    write: &redb::WriteTransaction,
    channels: &[Uri],
) -> Result<BTreeSet<Uri>, StoreError> {
    let mut sessions = BTreeSet::new();
    for channel in channels {
        if channel.starts_with("ahp-session:/") {
            sessions.insert(channel.clone());
        } else if let Some(chat) = load_chat(write, channel)? {
            sessions.insert(chat.session_uri);
        } else if let Some(chat) =
            read_record::<PersistedChat>(read, crate::tables::CHATS, channel)?
        {
            sessions.insert(chat.session_uri);
        }
    }
    Ok(sessions)
}

pub(crate) fn catalogs_before(
    read: &redb::ReadTransaction,
    sessions: &BTreeSet<Uri>,
) -> Result<BTreeMap<Uri, Catalog>, StoreError> {
    collect_catalogs(
        sessions,
        |uri| read_record(read, crate::tables::SESSIONS, uri),
        |uri| read_record(read, crate::tables::CHATS, uri),
    )
}

pub(crate) fn catalogs(
    write: &redb::WriteTransaction,
    sessions: &BTreeSet<Uri>,
) -> Result<BTreeMap<Uri, Catalog>, StoreError> {
    collect_catalogs(
        sessions,
        |uri| load_session(write, uri),
        |uri| load_chat(write, uri),
    )
}

fn collect_catalogs(
    sessions: &BTreeSet<Uri>,
    mut session_record: impl FnMut(&str) -> Result<Option<PersistedSession>, StoreError>,
    mut chat_record: impl FnMut(&str) -> Result<Option<PersistedChat>, StoreError>,
) -> Result<BTreeMap<Uri, Catalog>, StoreError> {
    let mut catalogs = BTreeMap::new();
    for uri in sessions {
        let Some(session) = session_record(uri)? else {
            continue;
        };
        let mut chats = Vec::new();
        let mut requests = Vec::new();
        for chat_uri in session.chats {
            if let Some(chat) = chat_record(&chat_uri)? {
                requests.extend(input_requests(&chat));
                chats.push(chat);
            }
        }
        catalogs.insert(
            uri.clone(),
            Catalog {
                chats,
                requests,
                stored_requests: session.input_needed,
            },
        );
    }
    Ok(catalogs)
}

pub(crate) fn input_requests(chat: &PersistedChat) -> Vec<SessionInputRequest> {
    chat.active
        .as_ref()
        .map(|active| chat_input_requests(&chat.uri, &active.id, &active.response_parts))
        .unwrap_or_default()
}

pub(crate) fn reconcile(
    write: &redb::WriteTransaction,
    before: &BTreeMap<Uri, Catalog>,
    after: &BTreeMap<Uri, Catalog>,
    drafts: &mut Vec<Draft>,
) -> Result<(), StoreError> {
    for (uri, catalog) in after {
        let previous = before.get(uri);
        let requests = previous.map_or(&[][..], |catalog| catalog.stored_requests.as_slice());
        for action in input_request_actions(requests, &catalog.requests) {
            drafts.push(Draft::action(uri.clone(), action, None));
        }
        let updated = ahp_server_core::catalog::updated_input_requests(requests, &catalog.requests);
        if updated != catalog.stored_requests {
            if let Some(mut session) = load_session(write, uri)? {
                session.input_needed = updated;
                write_session(write, &session)?;
            }
        }
        let previous_chats: Vec<ChatSummary> = previous
            .map(|catalog| catalog.chats.iter().map(chat_summary).collect())
            .unwrap_or_default();
        for chat in &catalog.chats {
            let summary = chat_summary(chat);
            let changed = previous_chats
                .iter()
                .find(|previous| previous.resource == chat.uri)
                .is_some_and(|previous| *previous != summary);
            if changed && !drafts.iter().any(|draft| draft.updates_chat(&chat.uri)) {
                drafts.push(summary_draft(chat));
            }
        }
    }
    for (uri, catalog) in before {
        if !after.contains_key(uri) {
            for action in input_request_actions(&catalog.stored_requests, &[]) {
                drafts.push(Draft::action(uri.clone(), action, None));
            }
        }
    }
    Ok(())
}
