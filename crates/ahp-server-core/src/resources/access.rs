use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ahp_types::commands::ResourceRequestParams;
use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::errors::{ahp_error_codes, PermissionDeniedErrorData};
use workspace_agent_tools::{file_uri_path, ConfiguredWorkspaces};

use crate::host::CoreHost;
use crate::rpc::RpcFault;

pub(super) fn root(channel: &str) -> Result<(), RpcFault> {
    if channel == ROOT_RESOURCE_URI {
        Ok(())
    } else {
        Err(RpcFault::invalid_params(
            "resource requests must target ahp-root://",
        ))
    }
}

pub(super) fn path(
    host: &CoreHost,
    uri: &str,
    write: bool,
    follow: bool,
) -> Result<PathBuf, RpcFault> {
    let path = file_uri_path(uri).map_err(|error| RpcFault::invalid_params(error.to_string()))?;
    let workspaces = host
        .inner
        .profile
        .resource_workspaces()
        .ok_or_else(|| denied(uri, write))?;
    confined(workspaces, &path, follow).map_err(|error| {
        if error.kind() == io::ErrorKind::PermissionDenied {
            denied(uri, write)
        } else {
            io_fault(error)
        }
    })
}

pub(super) fn confined(
    workspaces: &ConfiguredWorkspaces,
    path: &Path,
    follow: bool,
) -> io::Result<PathBuf> {
    if !follow && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
        return confined(workspaces, path, true);
    }
    let (mut ancestor, mut suffix) = if follow || path.file_name().is_none() {
        (path.to_path_buf(), Vec::new())
    } else {
        (
            path.parent().unwrap_or(path).to_path_buf(),
            vec![path.file_name().unwrap_or_default().to_owned()],
        )
    };
    let canonical = loop {
        match ancestor.canonicalize() {
            Ok(path) => break path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A dangling symlink is not a missing directory we may create through.
                if fs::symlink_metadata(&ancestor).is_ok() {
                    return Err(error);
                }
                let name = ancestor.file_name().ok_or(error)?;
                suffix.push(name.to_owned());
                if !ancestor.pop() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "outside workspace",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    };
    let directory = if canonical.is_dir() {
        canonical.as_path()
    } else {
        canonical.parent().unwrap_or(&canonical)
    };
    let uri = url::Url::from_file_path(directory)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid file path"))?;
    workspaces
        .select(Some(vec![uri.into()]))
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "outside workspace"))?;
    let mut result = canonical;
    for component in suffix.into_iter().rev() {
        result.push(component);
    }
    Ok(result)
}

pub(super) fn denied(uri: &str, write: bool) -> RpcFault {
    let request = ResourceRequestParams {
        channel: ROOT_RESOURCE_URI.into(),
        meta: None,
        uri: uri.into(),
        read: Some(true),
        write: Some(write),
    };
    RpcFault {
        code: ahp_error_codes::PERMISSION_DENIED,
        message: "resource is not accessible within the configured workspace roots".into(),
        data: serde_json::to_value(PermissionDeniedErrorData {
            request: Some(request),
        })
        .ok(),
    }
}

pub(super) fn fault(code: i32, message: impl Into<String>) -> RpcFault {
    RpcFault {
        code,
        message: message.into(),
        data: None,
    }
}

pub(super) fn io_fault(error: io::Error) -> RpcFault {
    let code = match error.kind() {
        io::ErrorKind::NotFound => ahp_error_codes::NOT_FOUND,
        io::ErrorKind::PermissionDenied => ahp_error_codes::PERMISSION_DENIED,
        io::ErrorKind::AlreadyExists => ahp_error_codes::ALREADY_EXISTS,
        _ => ahp_types::errors::json_rpc_error_codes::INVALID_PARAMS,
    };
    fault(code, error.to_string())
}

pub(super) fn uri(path: &Path) -> Result<String, RpcFault> {
    url::Url::from_file_path(path)
        .map(Into::into)
        .map_err(|_| RpcFault::invalid_params("invalid local file path"))
}
