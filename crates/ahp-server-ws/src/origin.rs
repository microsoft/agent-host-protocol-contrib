//! Strict browser Origin parsing separate from the transport loop.

use std::collections::HashSet;

use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode, Uri};

use crate::WsError;

pub(crate) struct OriginPolicy {
    allowed: HashSet<BrowserOrigin>,
}

impl OriginPolicy {
    pub(crate) fn new(origins: Vec<String>) -> Result<Self, WsError> {
        let mut allowed = HashSet::new();
        for origin in origins {
            let parsed =
                BrowserOrigin::parse(&origin).map_err(|reason| WsError::InvalidOrigin {
                    origin: origin.clone(),
                    reason: reason.into(),
                })?;
            allowed.insert(parsed);
        }
        Ok(Self { allowed })
    }

    pub(crate) fn validate(&self, headers: &HeaderMap) -> Result<(), OriginRejection> {
        let Some(origin) = headers.get(ORIGIN) else {
            return Ok(());
        };
        let origin = origin
            .to_str()
            .map_err(|_| OriginRejection::malformed())
            .and_then(|origin| {
                BrowserOrigin::parse(origin).map_err(|_| OriginRejection::malformed())
            })?;
        let host = headers
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .ok_or_else(OriginRejection::missing_host)?;
        if origin.authority == host.to_ascii_lowercase() || self.allowed.contains(&origin) {
            Ok(())
        } else {
            Err(OriginRejection {
                status: StatusCode::FORBIDDEN,
                message: "WebSocket Origin is not allowed",
            })
        }
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct BrowserOrigin {
    authority: String,
    scheme: String,
}

impl BrowserOrigin {
    fn parse(value: &str) -> Result<Self, &'static str> {
        let uri: Uri = value.parse().map_err(|_| "must be an HTTP(S) origin")?;
        let scheme = uri
            .scheme_str()
            .filter(|scheme| matches!(*scheme, "http" | "https"))
            .ok_or("must use http or https")?;
        let authority = uri.authority().ok_or("must include an authority")?.as_str();
        if authority.contains('@') || uri.path() != "/" || uri.query().is_some() {
            return Err("must contain only scheme and authority");
        }
        Ok(Self {
            authority: authority.to_ascii_lowercase(),
            scheme: scheme.to_ascii_lowercase(),
        })
    }
}

pub(crate) struct OriginRejection {
    pub(crate) status: StatusCode,
    pub(crate) message: &'static str,
}

impl OriginRejection {
    fn malformed() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "WebSocket Origin must be a valid http:// or https:// origin",
        }
    }

    fn missing_host() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "WebSocket Origin validation requires a valid Host header",
        }
    }
}
