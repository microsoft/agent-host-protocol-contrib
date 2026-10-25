//! Exact single-replacement implementation for a confined workspace file.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use ahp_types::state::{ContentRef, FileEditSide, ToolResultContent, ToolResultFileEditContent};
use serde::Deserialize;

use crate::{resolve_existing_file, ToolExecution};

/// Largest regular UTF-8 file accepted by `replace_string`.
pub const MAX_REPLACE_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceStringArgs {
    path: String,
    old_string: String,
    new_string: String,
}

/// Parsed input for a `replace_string` invocation.
#[derive(Debug)]
pub struct PreparedReplaceString {
    args: ReplaceStringArgs,
}

impl PreparedReplaceString {
    pub(crate) fn parse(input: &str) -> Result<Self, String> {
        serde_json::from_str(input)
            .map(|args| Self { args })
            .map_err(|error| format!("invalid replace_string arguments: {error}"))
    }

    /// Returns the requested, still-untrusted relative target path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.args.path
    }
}

pub(crate) fn execute(workspace: &Path, prepared: PreparedReplaceString) -> ToolExecution {
    if prepared.args.old_string.is_empty() {
        return ToolExecution::failed("old_string must not be empty");
    }
    let (target, relative) = match resolve_existing_file(workspace, &prepared.args.path) {
        Ok(path) => path,
        Err(error) => return ToolExecution::failed(error.to_string()),
    };
    let mut file = match File::open(&target) {
        Ok(file) => file,
        Err(error) => {
            return ToolExecution::failed(format!("could not open {}: {error}", relative.display()))
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return ToolExecution::failed(format!(
                "could not inspect {}: {error}",
                relative.display()
            ))
        }
    };
    if !metadata.is_file() {
        return ToolExecution::failed(format!(
            "{} must be an existing regular file",
            relative.display()
        ));
    }
    let mut bytes = Vec::with_capacity((MAX_REPLACE_FILE_BYTES + 1) as usize);
    let read = Read::by_ref(&mut file)
        .take(MAX_REPLACE_FILE_BYTES + 1)
        .read_to_end(&mut bytes);
    if let Err(error) = read {
        return ToolExecution::failed(format!("could not read {}: {error}", relative.display()));
    }
    if bytes.len() as u64 > MAX_REPLACE_FILE_BYTES {
        return ToolExecution::failed(format!(
            "{} exceeds the {} byte replace_string limit",
            relative.display(),
            MAX_REPLACE_FILE_BYTES
        ));
    }
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(error) => {
            return ToolExecution::failed(format!(
                "{} must be a UTF-8 file: {error}",
                relative.display()
            ))
        }
    };
    drop(file);
    let Some(first) = contents.find(&prepared.args.old_string) else {
        return ToolExecution::failed(format!(
            "old_string was not found in {}",
            relative.display()
        ));
    };
    let first_character_bytes = contents[first..].chars().next().map_or(0, char::len_utf8);
    if contents[first + first_character_bytes..].contains(&prepared.args.old_string) {
        return ToolExecution::failed(format!(
            "old_string must occur exactly once in {}",
            relative.display()
        ));
    }
    let replacement = format!(
        "{}{}{}",
        &contents[..first],
        prepared.args.new_string,
        &contents[first + prepared.args.old_string.len()..]
    );
    let uri = match url::Url::from_file_path(&target) {
        Ok(uri) => uri.to_string(),
        Err(()) => {
            return ToolExecution::failed(format!(
                "could not convert {} to a file URI",
                relative.display()
            ))
        }
    };
    if let Err(error) = atomic_write(&target, &replacement, &metadata.permissions()) {
        return ToolExecution::failed(format!(
            "could not replace text in {}: {error}",
            relative.display()
        ));
    }
    let mut result = ToolExecution::succeeded(
        "Replaced text in a workspace file",
        format!("Replaced one exact occurrence in {}.", relative.display()),
    );
    result
        .content
        .push(ToolResultContent::FileEdit(ToolResultFileEditContent {
            before: None,
            after: Some(FileEditSide {
                uri: uri.clone(),
                content: ContentRef {
                    uri,
                    size_hint: i64::try_from(replacement.len()).ok(),
                    content_type: Some("text/plain".into()),
                    nonce: None,
                },
            }),
            diff: None,
        }));
    result
}

fn atomic_write(
    target: &Path,
    contents: &str,
    permissions: &fs::Permissions,
) -> Result<(), std::io::Error> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::other("file has no parent directory"))?;
    let name = target
        .file_name()
        .ok_or_else(|| std::io::Error::other("file has no name"))?
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{name}.workspace-agent-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(&temporary, permissions.clone())?;
        fs::rename(&temporary, target)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}
