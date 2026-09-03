//! Model-picker state that is durable even while the draft text is empty.

use std::time::Duration;

use super::super::ConformanceError;
use super::helpers::{
    assert_action_draft, assert_snapshot_draft, assert_snapshot_without_draft, chat_state,
    create_session_without_config, initialized_probe, reasoning_selection, send_draft, user_draft,
};

pub(in crate::runner) async fn draft_model_only(
    target: &str,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "draft-model-only").await?;
    let created = create_session_without_config(&mut probe, "draft-model-only").await?;
    let draft = user_draft("", Some(reasoning_selection("high")))?;
    let set_client_seq = send_draft(&mut probe, &created.chat, Some(draft.clone())).await?;
    let set = probe
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&set, Some(&draft), set_client_seq)?;
    assert_snapshot_draft(&chat_state(&mut probe, &created.chat).await?, &draft)?;

    let clear_client_seq = send_draft(&mut probe, &created.chat, None).await?;
    let clear = probe
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&clear, None, clear_client_seq)?;
    assert_snapshot_without_draft(&chat_state(&mut probe, &created.chat).await?)?;
    probe.close().await?;
    Ok("retains an empty-text draft model selection and clears draft state by omission".to_owned())
}
