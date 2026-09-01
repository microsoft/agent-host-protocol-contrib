//! Canonical workspace selection and confined file resolution.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use ahp_types::common::Uri;
use url::Url;

/// A configured workspace root and its canonical URI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedWorkspace {
    path: PathBuf,
    uri: Uri,
}

impl SelectedWorkspace {
    /// Returns the canonical directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the canonical `file:` URI for the directory.
    #[must_use]
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
}

/// Canonical workspace roots which a host allows callers to select.
#[derive(Clone, Debug)]
pub struct ConfiguredWorkspaces {
    roots: Vec<PathBuf>,
}

/// Failure while validating workspace roots, URIs, or workspace-relative paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceError {
    /// A configured root was not an existing directory.
    InvalidRoot { path: PathBuf },
    /// The requested URI was not a `file:` URI.
    NonFileUri,
    /// The URI could not be parsed into a local file-system path.
    InvalidFileUri,
    /// A selection URI did not identify its canonical location exactly.
    NonCanonicalFileUri { supplied: Uri, canonical: Uri },
    /// A requested workspace did not exist.
    Missing { path: PathBuf },
    /// A requested workspace was not a directory.
    NotDirectory { path: PathBuf },
    /// A workspace was outside every configured root.
    OutsideRoot { path: PathBuf },
    /// More than one `workingDirectory` was supplied.
    TooManyWorkingDirectories,
    /// A target did not name a non-empty relative path.
    InvalidRelativePath,
    /// A relative target did not exist.
    MissingPath { path: PathBuf },
    /// A relative target resolved outside the selected workspace.
    PathEscape { path: PathBuf },
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot { path } => {
                write!(
                    formatter,
                    "workspace root {path:?} is not an existing directory"
                )
            }
            Self::NonFileUri => formatter.write_str("working directory must be a file: URI"),
            Self::InvalidFileUri => {
                formatter.write_str("working directory URI is not a valid local file path")
            }
            Self::NonCanonicalFileUri { .. } => {
                formatter.write_str("working directory must be a canonical file: URI")
            }
            Self::Missing { path } => {
                write!(formatter, "working directory {path:?} does not exist")
            }
            Self::NotDirectory { path } => {
                write!(formatter, "working directory {path:?} is not a directory")
            }
            Self::OutsideRoot { path } => write!(
                formatter,
                "working directory {path:?} is outside the configured workspace roots"
            ),
            Self::TooManyWorkingDirectories => {
                formatter.write_str("this host profile accepts zero or one workingDirectory")
            }
            Self::InvalidRelativePath => formatter
                .write_str("path must be a non-empty relative path beneath the selected workspace"),
            Self::MissingPath { path } => write!(formatter, "path {path:?} does not exist"),
            Self::PathEscape { path } => {
                write!(
                    formatter,
                    "path {path:?} resolves outside the selected workspace"
                )
            }
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl ConfiguredWorkspaces {
    /// Canonicalizes non-empty, existing directory roots.
    pub fn new(roots: Vec<PathBuf>) -> Result<Self, WorkspaceError> {
        let roots = roots
            .into_iter()
            .map(|root| {
                let canonical = root
                    .canonicalize()
                    .map_err(|_| WorkspaceError::InvalidRoot { path: root.clone() })?;
                if canonical.is_dir() {
                    Ok(canonical)
                } else {
                    Err(WorkspaceError::InvalidRoot { path: root })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if roots.is_empty() {
            return Err(WorkspaceError::InvalidRoot {
                path: PathBuf::new(),
            });
        }
        Ok(Self { roots })
    }

    /// Configures the process current directory as the sole permitted root.
    pub fn from_current_dir() -> Result<Self, WorkspaceError> {
        let current = std::env::current_dir().map_err(|_| WorkspaceError::InvalidRoot {
            path: PathBuf::new(),
        })?;
        Self::new(vec![current])
    }

    /// Alias for [`Self::from_current_dir`] for host setup code.
    pub fn current_root() -> Result<Self, WorkspaceError> {
        Self::from_current_dir()
    }

    /// Returns the first configured canonical root, used when none is selected.
    #[must_use]
    pub fn default_root(&self) -> &Path {
        &self.roots[0]
    }

    /// Selects zero or one canonical `file:` URI within a configured root.
    pub fn select(&self, requested: Option<Vec<Uri>>) -> Result<SelectedWorkspace, WorkspaceError> {
        let requested = requested.unwrap_or_default();
        let selected = match requested.as_slice() {
            [] => self.default_root().to_path_buf(),
            [uri] => {
                let path = file_uri_path(uri)?;
                let canonical = canonical_path(&path)?;
                let canonical_uri = canonical_file_uri(&canonical)?;
                if *uri != canonical_uri {
                    return Err(WorkspaceError::NonCanonicalFileUri {
                        supplied: uri.clone(),
                        canonical: canonical_uri,
                    });
                }
                canonical
            }
            _ => return Err(WorkspaceError::TooManyWorkingDirectories),
        };
        self.selected_from_path(selected)
    }

    /// Revalidates a previously selected workspace before a tool uses it.
    pub(crate) fn revalidate(
        &self,
        workspace: &SelectedWorkspace,
    ) -> Result<SelectedWorkspace, WorkspaceError> {
        self.selected_from_path(workspace.path.clone())
    }

    fn selected_from_path(&self, selected: PathBuf) -> Result<SelectedWorkspace, WorkspaceError> {
        let canonical = canonical_path(&selected)?;
        if !canonical.is_dir() {
            return Err(WorkspaceError::NotDirectory { path: canonical });
        }
        if !self.roots.iter().any(|root| canonical.starts_with(root)) {
            return Err(WorkspaceError::OutsideRoot { path: canonical });
        }
        let uri = canonical_file_uri(&canonical)?;
        Ok(SelectedWorkspace {
            path: canonical,
            uri,
        })
    }
}

/// Returns an existing path's canonical path.
pub fn canonical_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    path.canonicalize()
        .map_err(|_| WorkspaceError::Missing { path: path.into() })
}

/// Converts an existing path to its canonical `file:` URI.
pub fn canonical_file_uri(path: &Path) -> Result<Uri, WorkspaceError> {
    let canonical = canonical_path(path)?;
    Url::from_file_path(canonical)
        .map(|uri| uri.into())
        .map_err(|_| WorkspaceError::InvalidFileUri)
}

/// Parses a local `file:` URI without resolving it.
pub fn file_uri_path(uri: &str) -> Result<PathBuf, WorkspaceError> {
    let parsed = Url::parse(uri).map_err(|_| WorkspaceError::InvalidFileUri)?;
    if parsed.scheme() != "file" {
        return Err(WorkspaceError::NonFileUri);
    }
    parsed
        .to_file_path()
        .map_err(|_| WorkspaceError::InvalidFileUri)
}

/// Parses a `file:` URI and returns the canonical existing path it identifies.
pub fn canonical_file_uri_path(uri: &str) -> Result<PathBuf, WorkspaceError> {
    canonical_path(&file_uri_path(uri)?)
}

/// Resolves an existing, workspace-confined relative target file.
pub fn resolve_existing_file(
    workspace: &Path,
    relative: &str,
) -> Result<(PathBuf, PathBuf), WorkspaceError> {
    let workspace = canonical_path(workspace)?;
    let relative_path = Path::new(relative);
    if relative.trim().is_empty()
        || relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(WorkspaceError::InvalidRelativePath);
    }
    let resolved = canonical_path(&workspace.join(relative_path)).map_err(|_| {
        WorkspaceError::MissingPath {
            path: relative_path.into(),
        }
    })?;
    if !resolved.starts_with(&workspace) {
        return Err(WorkspaceError::PathEscape {
            path: relative_path.into(),
        });
    }
    Ok((resolved, relative_path.into()))
}
