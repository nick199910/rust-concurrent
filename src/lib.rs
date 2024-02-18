#![feature(allocator_api)]
//! Homeworks

#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(unreachable_pub)]
#![allow(clippy::result_unit_err)]
// Allow lints for homework.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_mut)]

pub mod adt;
pub mod list_set;

pub mod test;
pub mod lock;
pub mod lockfree;


pub use adt::{
    ConcurrentMap, ConcurrentSet, SequentialMap,
};
pub use list_set::{FineGrainedListSet, OptimisticFineGrainedListSet};
