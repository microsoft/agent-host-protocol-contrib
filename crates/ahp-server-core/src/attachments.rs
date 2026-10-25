//! Resolving bounded chat attachments without recursively expanding references.

use std::collections::HashSet;
use std::io::{self, Write};

use ahp_types::state::{Message, MessageAttachment, Turn};

use crate::{StoreConflict, StoreError};

const MAX_CHAT_ATTACHMENTS: usize = 16;
const MAX_MATERIALIZED_BYTES: usize = 1024 * 1024;

/// Materializes only the directly attached chats from their retained, finished turns.
/// The caller resolves every reference in the same transaction that accepts the message.
pub fn materialize_attachments(
    message: &Message,
    mut retained_turns: impl FnMut(&str) -> Result<Vec<Turn>, StoreError>,
) -> Result<Vec<Turn>, StoreError> {
    let mut attachments = Vec::new();
    let mut seen = HashSet::new();
    let mut count = 0;
    for attachment in message.attachments.iter().flatten() {
        let MessageAttachment::Chat(attachment) = attachment else {
            continue;
        };
        count += 1;
        if count > MAX_CHAT_ATTACHMENTS {
            return Err(StoreConflict::ChatAttachmentLimitExceeded(format!(
                "at most {MAX_CHAT_ATTACHMENTS} chat attachments are allowed per message"
            ))
            .into());
        }
        if seen.insert((attachment.resource.as_str(), attachment.end_turn.as_deref())) {
            attachments.push(attachment);
        }
    }
    let mut context = Vec::new();
    let mut budget = MaterializedBudget {
        bytes: 2,
        exceeded: false,
    };
    for attachment in attachments {
        let turns = retained_turns(&attachment.resource)?;
        let end = match &attachment.end_turn {
            Some(id) => turns
                .iter()
                .position(|turn| turn.id == *id)
                .map(|index| index + 1)
                .ok_or_else(|| crate::StoreConflict::InvalidChatAttachment {
                    chat: attachment.resource.clone(),
                    end_turn: id.clone(),
                })?,
            None => turns.len(),
        };
        for turn in turns.into_iter().take(end) {
            if !context.is_empty() {
                budget.write_all(b",").map_err(|_| materialized_limit())?;
            }
            if let Err(error) = serde_json::to_writer(&mut budget, &turn) {
                return Err(if budget.exceeded {
                    materialized_limit()
                } else {
                    StoreError::Backend(error.to_string())
                });
            }
            context.push(turn);
        }
    }
    Ok(context)
}

fn materialized_limit() -> StoreError {
    StoreConflict::ChatAttachmentLimitExceeded(format!(
        "materialized chat transcripts exceed {MAX_MATERIALIZED_BYTES} bytes per message"
    ))
    .into()
}

struct MaterializedBudget {
    bytes: usize,
    exceeded: bool,
}

impl Write for MaterializedBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_MATERIALIZED_BYTES.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "chat transcript byte limit exceeded",
            ));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
