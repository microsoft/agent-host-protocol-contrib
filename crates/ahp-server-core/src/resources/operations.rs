use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ahp_types::commands::*;
use ahp_types::errors::ahp_error_codes;
use ahp_types::messages::JsonRpcRequest;
use serde::Serialize;
use serde_json::Value;

use super::{access, files};
use crate::host::CoreHost;
use crate::rpc::{params, RpcFault};

pub(super) fn request(host: &CoreHost, request: JsonRpcRequest) -> Result<Value, RpcFault> {
    match request.method.as_str() {
        "resourceRead" => {
            let params: ResourceReadParams = params(&request)?;
            access::root(&params.channel)?;
            let path = access::path(host, &params.uri, false, true)?;
            value(files::read(&path, params.encoding)?)
        }
        "resourceWrite" => {
            let params: ResourceWriteParams = params(&request)?;
            access::root(&params.channel)?;
            let path = access::path(host, &params.uri, true, true)?;
            files::write(host, &path, params)?;
            value(ResourceWriteResult {})
        }
        "resourceList" => {
            let params: ResourceListParams = params(&request)?;
            access::root(&params.channel)?;
            let path = access::path(host, &params.uri, false, true)?;
            let mut entries = fs::read_dir(path)
                .map_err(access::io_fault)?
                .map(|entry| {
                    let entry = entry.map_err(access::io_fault)?;
                    let kind = entry.file_type().map_err(access::io_fault)?;
                    let is_directory = kind.is_dir()
                        || (kind.is_symlink()
                            && access::path(host, &access::uri(&entry.path())?, false, true)
                                .is_ok_and(|path| path.is_dir()));
                    Ok(DirectoryEntry {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        r#type: if is_directory { "directory" } else { "file" }.into(),
                    })
                })
                .collect::<Result<Vec<_>, RpcFault>>()?;
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            value(ResourceListResult { entries })
        }
        "resourceResolve" => {
            let params: ResourceResolveParams = params(&request)?;
            access::root(&params.channel)?;
            let follow = params.follow_symlinks.unwrap_or(true);
            let path = access::path(host, &params.uri, false, follow)?;
            value(files::resolve(host, &path, follow, params.uri)?)
        }
        "resourceMkdir" => {
            let params: ResourceMkdirParams = params(&request)?;
            access::root(&params.channel)?;
            let path = access::path(host, &params.uri, true, true)?;
            if path.exists() && !path.is_dir() {
                return Err(access::fault(
                    ahp_error_codes::ALREADY_EXISTS,
                    "resource is not a directory",
                ));
            }
            fs::create_dir_all(path).map_err(access::io_fault)?;
            value(ResourceMkdirResult {})
        }
        "resourceDelete" => {
            let params: ResourceDeleteParams = params(&request)?;
            access::root(&params.channel)?;
            let path = access::path(host, &params.uri, true, false)?;
            remove(&path, params.recursive.unwrap_or(false))?;
            value(ResourceDeleteResult {})
        }
        "resourceCopy" => {
            let params: ResourceCopyParams = params(&request)?;
            access::root(&params.channel)?;
            let source = access::path(host, &params.source, false, true)?;
            let destination = access::path(host, &params.destination, true, true)?;
            transfer(
                host,
                &source,
                &destination,
                params.fail_if_exists.unwrap_or(false),
                false,
            )?;
            value(ResourceCopyResult {})
        }
        "resourceMove" => {
            let params: ResourceMoveParams = params(&request)?;
            access::root(&params.channel)?;
            let source = access::path(host, &params.source, true, false)?;
            let destination = access::path(host, &params.destination, true, false)?;
            transfer(
                host,
                &source,
                &destination,
                params.fail_if_exists.unwrap_or(false),
                true,
            )?;
            value(ResourceMoveResult {})
        }
        "resourceRequest" => {
            let params: ResourceRequestParams = params(&request)?;
            access::root(&params.channel)?;
            access::path(host, &params.uri, params.write.unwrap_or(false), true)?;
            value(ResourceRequestResult {})
        }
        _ => Err(RpcFault::invalid_params("unknown resource request")),
    }
}

fn value(value: impl Serialize) -> Result<Value, RpcFault> {
    serde_json::to_value(value).map_err(RpcFault::internal)
}

fn remove(path: &Path, recursive: bool) -> Result<(), RpcFault> {
    let metadata = fs::symlink_metadata(path).map_err(access::io_fault)?;
    if metadata.is_dir() {
        if recursive {
            fs::remove_dir_all(path)
        } else {
            fs::remove_dir(path)
        }
    } else {
        fs::remove_file(path)
    }
    .map_err(access::io_fault)
}

fn transfer(
    host: &CoreHost,
    source: &Path,
    destination: &Path,
    fail_if_exists: bool,
    moving: bool,
) -> Result<(), RpcFault> {
    let metadata = fs::symlink_metadata(source).map_err(access::io_fault)?;
    let exists = match fs::symlink_metadata(destination) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(access::io_fault(error)),
    };
    if exists && fail_if_exists {
        return Err(access::fault(
            ahp_error_codes::ALREADY_EXISTS,
            "destination already exists",
        ));
    }
    if !moving {
        if !metadata.is_dir() {
            files::regular(&metadata)?;
        }
        if exists {
            let target = fs::metadata(destination).map_err(access::io_fault)?;
            if !target.is_dir() {
                files::regular(&target)?;
            }
        }
    }
    if source == destination {
        return Ok(());
    }
    if source.starts_with(destination) || (metadata.is_dir() && destination.starts_with(source)) {
        return Err(RpcFault::invalid_params(
            "source and destination must not contain one another",
        ));
    }
    if moving {
        if exists && (metadata.is_dir() || destination.is_dir()) {
            remove(destination, true)?;
        }
        fs::rename(source, destination).map_err(access::io_fault)
    } else {
        if exists && (metadata.is_dir() || destination.is_dir()) {
            remove(destination, true)?;
        }
        copy(host, source, destination, &mut HashSet::new())
    }
}

fn copy(
    host: &CoreHost,
    source: &Path,
    destination: &Path,
    ancestors: &mut HashSet<PathBuf>,
) -> Result<(), RpcFault> {
    let source = access::path(host, &access::uri(source)?, false, true)?;
    let destination = access::path(host, &access::uri(destination)?, true, true)?;
    if !ancestors.insert(source.clone()) {
        return Err(RpcFault::invalid_params("cyclic resource copy"));
    }
    let metadata = fs::metadata(&source).map_err(access::io_fault)?;
    if !metadata.is_dir() {
        files::regular(&metadata)?;
    }
    if let Ok(target) = fs::metadata(&destination) {
        if !target.is_dir() {
            files::regular(&target)?;
        }
    }
    if metadata.is_dir() {
        if destination.exists() && !destination.is_dir() {
            remove(&destination, false)?;
        }
        fs::create_dir_all(&destination).map_err(access::io_fault)?;
        for entry in fs::read_dir(&source).map_err(access::io_fault)? {
            let entry = entry.map_err(access::io_fault)?;
            copy(
                host,
                &entry.path(),
                &destination.join(entry.file_name()),
                ancestors,
            )?;
        }
    } else {
        if destination.is_dir() {
            remove(&destination, true)?;
        }
        files::copy(&source, &destination)?;
    }
    ancestors.remove(&source);
    Ok(())
}
