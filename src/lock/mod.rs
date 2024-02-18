//! Locks.

mod api;

pub mod seqlock;
mod spinlock;

pub use api::{Lock, LockGuard, RawLock, RawTryLock};
