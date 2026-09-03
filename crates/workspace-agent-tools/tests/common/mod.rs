use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use workspace_agent_tools::{
    ConfiguredWorkspaces, SelectedWorkspace, WorkspaceError, WorkspaceTools,
};

pub struct TestDir {
    path: PathBuf,
    base: PathBuf,
}

impl TestDir {
    pub fn new(name: &str) -> io::Result<Self> {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".test-artifacts");
        fs::create_dir_all(&base)?;
        let path = base.join(format!("{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path)?;
        Ok(Self { path, base })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
        let _ = fs::remove_dir(&self.base);
    }
}

#[allow(dead_code)]
pub fn workspace_tools(root: &Path) -> Result<(WorkspaceTools, SelectedWorkspace), WorkspaceError> {
    let tools = WorkspaceTools::new(ConfiguredWorkspaces::new(vec![root.into()])?);
    let workspace = tools.select(None)?;
    Ok((tools, workspace))
}
