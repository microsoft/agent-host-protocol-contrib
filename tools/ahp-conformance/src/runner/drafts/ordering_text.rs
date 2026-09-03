use serde_json::Value;

use super::super::ConformanceError;
use super::ordering::{action_type, OrderedAction, FINAL_RESPONSE};

pub(super) fn assert_final_text(
    actions: &[OrderedAction],
    initial: usize,
    final_part: usize,
    complete: usize,
    initial_id: &str,
    final_id: &str,
) -> Result<(), ConformanceError> {
    let text = actions
        .iter()
        .enumerate()
        .filter(|(_, action)| action_type(action) == Some("chat/delta"))
        .map(|(index, action)| {
            if index <= final_part
                || index >= complete
                || action.action.get("partId").and_then(Value::as_str) != Some(final_id)
            {
                return Err(ConformanceError::Options(
                    "text was appended before completion's markdown part or to the wrong part"
                        .to_owned(),
                ));
            }
            action
                .action
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| ConformanceError::Options("text delta omitted content".to_owned()))
        })
        .collect::<Result<String, _>>()?;
    if text != FINAL_RESPONSE
        || actions[..=initial].iter().any(|action| {
            action_type(action) == Some("chat/delta")
                && action.action.get("partId").and_then(Value::as_str) == Some(initial_id)
        })
    {
        Err(ConformanceError::Options(
            "final text was not reconstructed solely in the post-tool markdown part".to_owned(),
        ))
    } else {
        Ok(())
    }
}
