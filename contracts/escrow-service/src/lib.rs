// Escrow Service Contract
// Main contract entry point that re-exports all modules

mod escrow;
mod dispute;
mod timelock;
pub mod shared;

pub use escrow::EscrowService;
pub use dispute::DisputeResolution;
pub use timelock::TimeLockRelease;