//! Gitignored named WebSocket targets, including query-based credentials.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Default, Deserialize, Serialize)]
struct TargetStore {
    targets: BTreeMap<String, String>,
}

pub(crate) fn load(path: &Path, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    validate_name(name)?;
    let store: TargetStore = serde_json::from_slice(&fs::read(path)?)?;
    store.targets.get(name).cloned().ok_or_else(|| {
        format!(
            "target profile {name:?} does not exist in {}",
            path.display()
        )
        .into()
    })
}

pub(crate) fn save(
    path: &Path,
    name: &str,
    target: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_name(name)?;
    validate_target(target)?;
    let mut store = if path.exists() {
        serde_json::from_slice::<TargetStore>(&fs::read(path)?)?
    } else {
        TargetStore::default()
    };
    store.targets.insert(name.to_owned(), target.to_owned());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_private(path, &serde_json::to_vec_pretty(&store)?)?;
    Ok(())
}

pub(crate) fn validate_target(target: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = Url::parse(target)?;
    if !matches!(url.scheme(), "ws" | "wss") || url.host_str().is_none() {
        return Err("target must be an absolute ws:// or wss:// URL".into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(
            "target profile names may contain only letters, numbers, '.', '_', and '-'".into(),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &[u8]) -> Result<(), std::io::Error> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents)
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &[u8]) -> Result<(), std::io::Error> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(contents)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{load, save};

    #[test]
    fn saves_and_loads_named_targets() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!(
            "ahp-conformance-targets-{}.json",
            uuid::Uuid::new_v4()
        ));
        save(&path, "vscode-local", "ws://localhost:1234/?tkn=secret")?;
        assert_eq!(
            load(&path, "vscode-local")?,
            "ws://localhost:1234/?tkn=secret"
        );
        fs::remove_file(path)?;
        Ok(())
    }
}
