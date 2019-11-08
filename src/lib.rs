//! A slab allocator implementation for objects less than a page-size (4 KiB or 2MiB).
//!
//! # Overview
//!
//! The organization is as follows:
//!
//!  * A `ZoneAllocator` manages many `SCAllocator` and can
//!    satisfy requests for different allocation sizes.
//!  * A `SCAllocator` allocates objects of exactly one size.
//!    It stores the objects and meta-data in one or multiple `AllocablePage` objects.
//!  * A trait `AllocablePage` that defines the page-type from which we allocate objects.
//!
//! Lastly, it provides two default `AllocablePage` implementations `ObjectPage` and `LargeObjectPage`:
//!  * A `ObjectPage` that is 4 KiB in size and contains allocated objects and associated meta-data.
//!  * A `LargeObjectPage` that is 2 MiB in size and contains allocated objects and associated meta-data.
//!
//!
//! # Example: Implementing GlobalAlloc
//!
//! You can use slabmalloc to implement a rust allocator. Note that slabmalloc requires
//! a lower-level allocator (not provided by this crate) that can supply the allocator
//! with backing memory (i.e., `LargeObjectPage` and `ObjectPage` structs).
//!
//! Here is how an eventual dummy implementation could look like:
//!
//! ```rust
//! use core::alloc::{GlobalAlloc, Layout};
//! use core::ptr::{self, NonNull};
//! use slabmalloc::*;
//! use spin::Mutex;
//!
//! const BASE_PAGE_SIZE: usize = 4096;
//! const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;
//!
//! /// slabmalloc requires a lower-level allocator (not provided by this crate)
//! /// that can supply the allocator with backing memory
//! /// for `LargeObjectPage` and `ObjectPage` structs.
//! ///
//! /// The implementation will just provide a dummy implementation here
//! /// that doesn't allocate anything.
//! struct Pager;
//!
//! impl Pager {
//!     /// Returns base-pages (must be 4 KiB in size and 4 KiB aligned).
//!     fn allocate_page(&mut self) -> Option<&'static mut ObjectPage<'static>> {
//!         None
//!     }
//!
//!     /// Returns large-pages (must be 2 MiB in size and 2 MiB aligned).
//!     fn allocate_large_page(&mut self) -> Option<&'static mut LargeObjectPage<'static>> {
//!         None
//!     }
//! }
//!
//! /// A global pager for GlobalAlloc.
//! static mut PAGER: Pager = Pager;
//!
//! /// A SafeZoneAllocator that wraps the ZoneAllocator in a Mutex.
//! /// Note: This is not very scalable since we use a single big lock
//! /// around the allocator. There are better ways make the ZoneAllocator
//! /// thread-safe directly but they are not implemented yet.
//! pub struct SafeZoneAllocator(Mutex<ZoneAllocator<'static>>);
//!
//! unsafe impl GlobalAlloc for SafeZoneAllocator {
//!     unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
//!         match layout.size() {
//!             BASE_PAGE_SIZE => {
//!                 // Best to use the underlying backend directly to allocate pages
//!                 // PAGER.allocate_page()
//!                 ptr::null_mut()
//!             }
//!             LARGE_PAGE_SIZE => {
//!                 // Best to use the underlying backend directly to allocate large
//!                 // PAGER.allocate_large_page()
//!                 ptr::null_mut()
//!             }
//!             0..=ZoneAllocator::MAX_ALLOC_SIZE => {
//!                 let mut zone_allocator = self.0.lock();
//!                 match zone_allocator.allocate(layout) {
//!                     Ok(nptr) => nptr.as_ptr(),
//!                     Err(AllocationError::OutOfMemory(l)) => {
//!                         if l.size() > BASE_PAGE_SIZE {
//!                             PAGER.allocate_page().map_or(ptr::null_mut(), |page| {
//!                                 zone_allocator.refill(l, page).expect("Could not refill?");
//!                                 zone_allocator
//!                                     .allocate(layout)
//!                                     .expect("Should succeed after refill")
//!                                     .as_ptr()
//!                             })
//!                         } else {
//!                             PAGER
//!                                 .allocate_large_page()
//!                                 .map_or(ptr::null_mut(), |large_page| {
//!                                     zone_allocator
//!                                         .refill_large(l, large_page)
//!                                         .expect("Could not refill?");
//!                                     zone_allocator
//!                                         .allocate(layout)
//!                                         .expect("Should succeed after refill")
//!                                         .as_ptr()
//!                                 })
//!                         }
//!                     }
//!                     Err(AllocationError::InvalidLayout) => panic!("Can't allocate this size"),
//!                 }
//!             }
//!             _ => unimplemented!("Can't handle it, probably needs another allocator."),
//!         }
//!     }
//!
//!     unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
//!         match layout.size() {
//!             BASE_PAGE_SIZE => {
//!                 // TODO: Release base-page to backend
//!             }
//!             LARGE_PAGE_SIZE => {
//!                 // TODO: Release large-page to backend
//!             }
//!             0..=ZoneAllocator::MAX_ALLOC_SIZE => {
//!                 if let Some(nptr) = NonNull::new(ptr) {
//!                     self.0
//!                         .lock()
//!                         .deallocate(nptr, layout)
//!                         .expect("Couldn't deallocate");
//!                 } else {
//!                     // Nothing to do (don't dealloc null pointers).
//!                 }
//!
//!                 // An eventual reclamation strategy could also be implemented here
//!                 // to release empty pages back from the ZoneAllocator to the PAGER
//!             }
//!             _ => unimplemented!("Can't handle it, probably needs another allocator."),
//!         }
//!     }
//! }
//! ```

#![allow(unused_features)]
#![cfg_attr(feature = "unstable", feature(const_fn))]
#![cfg_attr(
    test,
    feature(
        prelude_import,
        test,
        raw,
        c_void_variant,
        core_intrinsics,
        vec_remove_item
    )
)]
#![no_std]
#![crate_name = "slabmalloc"]
#![crate_type = "lib"]

mod pages;
mod sc;
mod zone;

pub use pages::*;
pub use sc::*;
pub use zone::*;

#[cfg(test)]
#[macro_use]
extern crate std;
#[cfg(test)]
extern crate test;

#[cfg(test)]
mod tests;

use core::alloc::Layout;
use core::fmt;
use core::mem;
use core::ptr::{self, NonNull};

use log::trace;

#[cfg(target_arch = "x86_64")]
const CACHE_LINE_SIZE: usize = 64;

#[cfg(target_arch = "x86_64")]
const BASE_PAGE_SIZE: usize = 4096;

#[cfg(target_arch = "x86_64")]
#[allow(unused)]
const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
type VAddr = usize;

/// Error that can be returned for `allocation` and `deallocation` requests.
#[derive(Debug)]
pub enum AllocationError {
    /// Can't satisfy the allocation request for Layout because the allocator
    /// does not have enough memory (you may be able to `refill` it).
    OutOfMemory(Layout),
    /// Allocator can't deal with the provided size of the Layout.
    InvalidLayout,
}
