//! Host-core configuration validation.

use thiserror::Error;

/// Configuration shared by all transports of one logical AHP host.
#[derive(Clone, Debug)]
pub struct CoreConfig {
    /// Server implementation name sent in `initialize`.
    pub server_name: String,
    /// Largest number of Unicode scalar values in one persisted `chat/delta`.
    pub max_delta_scalars: usize,
    /// Per-peer bounded outbound frame queue capacity.
    pub outbound_queue_capacity: usize,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            server_name: "ahp-server".into(),
            max_delta_scalars: 16 * 1024,
            outbound_queue_capacity: 256,
        }
    }
}

impl CoreConfig {
    /// Validates configuration before a host accepts peers.
    pub fn validate(&self) -> Result<(), CoreConfigError> {
        if self.server_name.trim().is_empty() {
            return Err(CoreConfigError::EmptyServerName);
        }
        if self.max_delta_scalars == 0 {
            return Err(CoreConfigError::ZeroDeltaScalars);
        }
        if self.outbound_queue_capacity == 0 {
            return Err(CoreConfigError::ZeroOutboundQueueCapacity);
        }
        Ok(())
    }
}

/// Invalid [`CoreConfig`] input.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreConfigError {
    /// A server must provide a non-empty implementation name.
    #[error("server_name must not be empty")]
    EmptyServerName,
    /// Splitting a delta into zero scalar chunks is undefined.
    #[error("max_delta_scalars must be greater than zero")]
    ZeroDeltaScalars,
    /// A bounded queue must hold at least one frame.
    #[error("outbound_queue_capacity must be greater than zero")]
    ZeroOutboundQueueCapacity,
}

#[cfg(test)]
mod tests {
    use super::{CoreConfig, CoreConfigError};

    #[test]
    fn rejects_zero_scalar_limit() {
        let config = CoreConfig {
            max_delta_scalars: 0,
            ..CoreConfig::default()
        };
        assert_eq!(config.validate(), Err(CoreConfigError::ZeroDeltaScalars));
    }
}
