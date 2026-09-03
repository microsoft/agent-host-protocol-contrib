use ahp_types::state::{ResponsePart, SnapshotState};
use serde_json::Value;

use super::super::ConformanceError;
use super::ordering::{markdown_part_id, response_part, OrderedAction, FINAL_RESPONSE, TURN_ID};

pub(super) fn assert_snapshot_order(
    snapshot: &Value,
    actions: &[OrderedAction],
) -> Result<(), ConformanceError> {
    let SnapshotState::Chat(chat) = serde_json::from_value(snapshot.clone())? else {
        return Err(ConformanceError::Options(
            "response ordering subscription did not return a chat snapshot".to_owned(),
        ));
    };
    let turn = chat
        .turns
        .iter()
        .find(|turn| turn.id == TURN_ID)
        .ok_or_else(|| {
            ConformanceError::Options("completed ordering turn was not persisted".into())
        })?;
    let initial_id = markdown_part_id(&actions[response_part(actions, 0)?].action)?;
    let final_id = markdown_part_id(&actions[response_part(actions, 1)?].action)?;
    let display = turn
        .response_parts
        .iter()
        .filter(
            |part| !matches!(part, ResponsePart::Markdown(markdown) if markdown.content.is_empty()),
        )
        .collect::<Vec<_>>();
    if matches!(
        display.as_slice(),
        [ResponsePart::ToolCall(_), ResponsePart::Markdown(markdown)]
            if markdown.id == final_id && markdown.content == FINAL_RESPONSE
    ) && matches!(
        turn.response_parts.first(),
        Some(ResponsePart::Markdown(markdown)) if markdown.id == initial_id && markdown.content.is_empty()
    ) {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "snapshot response parts did not display as tool call followed by final markdown"
                .to_owned(),
        ))
    }
}
