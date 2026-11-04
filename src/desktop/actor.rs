//! Desktop ownership is shared across accounts and across gateway processes.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::resource_lock::ResourceLock;

pub(crate) struct DesktopActor {
    gate: Arc<Mutex<()>>,
}

pub(crate) struct DesktopLease {
    _process: ResourceLock,
    _local: OwnedMutexGuard<()>,
}

impl DesktopActor {
    pub fn new(gate: Arc<Mutex<()>>) -> Self {
        Self { gate }
    }

    pub async fn acquire(&self) -> Result<DesktopLease> {
        let local = self.gate.clone().lock_owned().await;
        let process = ResourceLock::desktop().await?;
        Ok(DesktopLease {
            _process: process,
            _local: local,
        })
    }
}
