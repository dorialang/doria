//! Coalesces only confirmed native notification posts.

use core::pin::Pin;

use super::sync::SyncCell;
use crate::platform_io::Failure;

pub(super) struct NotificationLatch {
    queued: SyncCell<bool>,
}

impl NotificationLatch {
    pub(super) const fn new() -> Self {
        Self {
            queued: SyncCell::new(false),
        }
    }

    fn queued(self: Pin<&Self>) -> Pin<&SyncCell<bool>> {
        // The private field is structurally pinned with this !Unpin owner;
        // no API moves it out or exposes mutable access to its storage.
        unsafe { self.map_unchecked(|latch| &latch.queued) }
    }

    /// `post` performs only the short native post, without reentering this
    /// latch or acquiring the broker's routing/election lock. Serialization
    /// spans posting and publication so neither a concurrent notifier nor
    /// consumer can observe an unconfirmed post as an existing notification.
    pub(super) fn notify(
        self: Pin<&Self>,
        post: impl FnOnce() -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let mut queued = self.queued().lock();
        if *queued {
            return Ok(());
        }
        post()?;
        *queued = true;
        Ok(())
    }

    /// Called after removing the corresponding pointer-free native packet.
    /// Clear before inspecting cancellation predicates so a subsequent change
    /// can post again instead of coalescing into an already-consumed packet.
    pub(super) fn consumed(self: Pin<&Self>) {
        *self.queued().lock() = false;
    }
}

#[cfg(test)]
mod tests;
