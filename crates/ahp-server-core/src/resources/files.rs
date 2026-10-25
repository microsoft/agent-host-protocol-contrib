use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use ahp_types::commands::{
    ContentEncoding, ResourceReadResult, ResourceResolveResult, ResourceType, ResourceWriteMode,
    ResourceWriteParams,
};
use ahp_types::errors::ahp_error_codes;
use base64::Engine;

use super::access::{fault, io_fault, uri};
use crate::host::CoreHost;
use crate::rpc::RpcFault;

pub(super) fn read(
    path: &Path,
    encoding: Option<ContentEncoding>,
) -> Result<ResourceReadResult, RpcFault> {
    let mut file = open_regular(path, OpenOptions::new().read(true))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_fault)?;
    let (data, encoding) = if encoding == Some(ContentEncoding::Base64) {
        (
            base64::engine::general_purpose::STANDARD.encode(bytes),
            ContentEncoding::Base64,
        )
    } else {
        match String::from_utf8(bytes) {
            Ok(text) => (text, ContentEncoding::Utf8),
            Err(error) => (
                base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
                ContentEncoding::Base64,
            ),
        }
    };
    Ok(ResourceReadResult {
        data,
        encoding,
        content_type: None,
    })
}

pub(super) fn write(
    host: &CoreHost,
    path: &Path,
    params: ResourceWriteParams,
) -> Result<(), RpcFault> {
    let data = match params.encoding {
        ContentEncoding::Utf8 => params.data.into_bytes(),
        ContentEncoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(params.data)
            .map_err(|_| RpcFault::invalid_params("invalid base64 data"))?,
    };
    let position = u64::try_from(params.position.unwrap_or(0))
        .map_err(|_| RpcFault::invalid_params("position must not be negative"))?;
    let existing = match fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_fault(error)),
    };
    if params.create_only.unwrap_or(false) && existing.is_some() {
        return Err(fault(
            ahp_error_codes::ALREADY_EXISTS,
            "resource already exists",
        ));
    }
    if let Some(expected) = params.if_match {
        if existing
            .as_ref()
            .map(|metadata| etag(host, path, metadata))
            .as_ref()
            != Some(&expected)
        {
            return Err(fault(
                ahp_error_codes::CONFLICT,
                "resource etag no longer matches",
            ));
        }
    }
    let mode = params.mode.unwrap_or(ResourceWriteMode::Truncate);
    let length = existing.as_ref().map_or(0, Metadata::len);
    if position > length {
        return Err(RpcFault::invalid_params("position exceeds resource length"));
    }
    let mut options = OpenOptions::new();
    options.write(true).read(true);
    if params.create_only.unwrap_or(false) {
        options.create_new(true);
    } else {
        options.create(true);
    }
    if mode == ResourceWriteMode::Append && position == 0 {
        options.append(true);
    }
    let mut file = open_regular(path, &mut options)?;
    let result = (|| -> io::Result<()> {
        match mode {
            ResourceWriteMode::Append if position == 0 => file.write_all(&data),
            ResourceWriteMode::Truncate => {
                file.set_len(position)?;
                file.seek(SeekFrom::Start(position))?;
                file.write_all(&data)
            }
            ResourceWriteMode::Append | ResourceWriteMode::Insert => {
                let offset = if mode == ResourceWriteMode::Append {
                    file.metadata()?
                        .len()
                        .checked_sub(position)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "position exceeds resource length",
                            )
                        })?
                } else {
                    position
                };
                file.seek(SeekFrom::Start(offset))?;
                let mut suffix = Vec::new();
                file.read_to_end(&mut suffix)?;
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(&data)?;
                file.write_all(&suffix)
            }
        }
    })();
    // Even a partial write invalidates a previously issued concurrency token.
    if let Ok(mut versions) = host.inner.resources.versions.lock() {
        let version = versions.entry(path.to_owned()).or_default();
        *version = version.wrapping_add(1);
    }
    result.map_err(io_fault)
}

pub(super) fn regular(metadata: &Metadata) -> Result<(), RpcFault> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(RpcFault::invalid_params("resource must be a regular file"))
    }
}

fn open_regular(path: &Path, options: &mut OpenOptions) -> Result<File, RpcFault> {
    match fs::metadata(path) {
        Ok(metadata) => regular(&metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_fault(error)),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Avoid blocking on a FIFO or following a symlink swapped in after preflight.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(io_fault)?;
    regular(&file.metadata().map_err(io_fault)?)?;
    Ok(file)
}

pub(super) fn copy(source: &Path, destination: &Path) -> Result<(), RpcFault> {
    let mut input = open_regular(source, OpenOptions::new().read(true))?;
    let mut output = open_regular(
        destination,
        OpenOptions::new().write(true).create(true).truncate(false),
    )?;
    output.set_len(0).map_err(io_fault)?;
    io::copy(&mut input, &mut output).map_err(io_fault)?;
    output
        .set_permissions(input.metadata().map_err(io_fault)?.permissions())
        .map_err(io_fault)
}

pub(super) fn resolve(
    host: &CoreHost,
    path: &Path,
    follow: bool,
    requested: String,
) -> Result<ResourceResolveResult, RpcFault> {
    let metadata = if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .map_err(io_fault)?;
    let kind = metadata.file_type();
    let r#type = if kind.is_symlink() {
        ResourceType::Symlink
    } else if kind.is_dir() {
        ResourceType::Directory
    } else {
        ResourceType::File
    };
    Ok(ResourceResolveResult {
        uri: if follow { uri(path)? } else { requested },
        r#type,
        size: if kind.is_dir() {
            None
        } else {
            i64::try_from(metadata.len()).ok()
        },
        mtime: metadata.modified().ok().map(timestamp),
        ctime: metadata.created().ok().map(timestamp),
        content_type: None,
        etag: Some(etag(host, path, &metadata)),
    })
}

fn timestamp(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}

pub(super) fn fingerprint(metadata: &Metadata) -> String {
    format!("{:?}-{}", metadata.modified().ok(), metadata.len())
}

fn etag(host: &CoreHost, path: &Path, metadata: &Metadata) -> String {
    let generation = host
        .inner
        .resources
        .versions
        .lock()
        .ok()
        .and_then(|versions| versions.get(path).copied())
        .unwrap_or(0);
    format!("{}-{generation}", fingerprint(metadata))
}
