mod common;

use std::fs;

use common::TestDir;
use workspace_agent_tools::{
    canonical_file_uri, canonical_file_uri_path, canonical_path, file_uri_path,
    resolve_existing_file, ConfiguredWorkspaces, WorkspaceError,
};

#[test]
fn canonical_file_uri_round_trips_on_this_platform() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("uri")?;
    let canonical = root.path().canonicalize()?;
    let uri = canonical_file_uri(root.path())?;
    assert_eq!(canonical_path(&file_uri_path(&uri)?)?, canonical);
    assert_eq!(canonical_file_uri_path(&uri)?, canonical);
    #[cfg(windows)]
    assert!(uri.starts_with("file:///"));
    #[cfg(unix)]
    assert!(uri.starts_with("file:///"));
    Ok(())
}

#[test]
fn selection_requires_one_canonical_file_uri_below_a_root() -> Result<(), Box<dyn std::error::Error>>
{
    let root = TestDir::new("root")?;
    let outside = TestDir::new("outside")?;
    fs::create_dir(root.path().join("nested"))?;
    let workspaces = ConfiguredWorkspaces::new(vec![root.path().into()])?;
    let nested_uri = canonical_file_uri(&root.path().join("nested"))?;
    let selected = workspaces.select(Some(vec![nested_uri.clone()]))?;
    assert_eq!(selected.uri(), &nested_uri);
    assert!(matches!(
        workspaces.select(Some(vec!["https://example.test/workspace".into()])),
        Err(WorkspaceError::NonFileUri)
    ));
    assert!(matches!(
        workspaces.select(Some(vec![canonical_file_uri(outside.path())?])),
        Err(WorkspaceError::OutsideRoot { .. })
    ));
    assert!(matches!(
        workspaces.select(Some(vec![nested_uri.clone(), nested_uri])),
        Err(WorkspaceError::TooManyWorkingDirectories)
    ));
    Ok(())
}

#[test]
fn rejects_relative_traversal_and_symlink_escape() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("escape-root")?;
    let outside = TestDir::new("escape-outside")?;
    let outside_file = outside.path().join("outside.txt");
    fs::write(&outside_file, "outside")?;
    let root_path = root.path().canonicalize()?;
    assert!(matches!(
        resolve_existing_file(&root_path, "../outside.txt"),
        Err(WorkspaceError::InvalidRelativePath)
    ));
    #[cfg(windows)]
    {
        assert!(matches!(
            resolve_existing_file(&root_path, r"C:\outside.txt"),
            Err(WorkspaceError::InvalidRelativePath)
        ));
        let link = root.path().join("outside-link.txt");
        if std::os::windows::fs::symlink_file(&outside_file, &link).is_ok() {
            assert!(matches!(
                resolve_existing_file(&root_path, "outside-link.txt"),
                Err(WorkspaceError::PathEscape { .. })
            ));
        }
    }
    #[cfg(unix)]
    {
        let link = root.path().join("outside-link.txt");
        std::os::unix::fs::symlink(&outside_file, &link)?;
        assert!(matches!(
            resolve_existing_file(&root_path, "outside-link.txt"),
            Err(WorkspaceError::PathEscape { .. })
        ));
    }
    Ok(())
}
