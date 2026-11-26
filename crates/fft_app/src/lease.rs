//! Scoped, non-minting app leases.
//!
//! Adapters and engines see the erased [`AppLease`] trait object. Only the
//! concrete app host constructs its own private lease type; there is no public
//! generic lease constructor. A lease is intentionally not a factory: there is
//! no `lease()`/`try_lease()` method on [`AppLease`].
//!
//! The same lease may be shared through an `Arc` inside the adapter/run it was
//! granted to; sharing does not create a new lease or a minting capability.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::FftApp;

/// Stable identity of one lease.
///
/// Ids are allocated by the lease owner (for example an `AtomicU64` in the app
/// host) and are unique for that owner/run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct LeaseId(pub u64);

/// Scoped access to an app granted by the app's owner.
///
/// Implementations are host-specific and have private fields/constructors.
/// Adapters receive `Arc<dyn AppLease>`; they cannot construct a lease or mint
/// another one from an existing lease.
pub trait AppLease: Send + Sync {
    fn lease_id(&self) -> LeaseId;

    /// Borrow the leased app.
    fn app(&self) -> &dyn FftApp;

    /// Compatibility hook for callers that need an owned `'static` app handle.
    ///
    /// This clones the app `Arc`; it does **not** mint a lease and must not be
    /// used as a general-purpose app factory.
    fn app_arc(&self) -> Arc<dyn FftApp>;
}
