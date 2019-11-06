//! A slab allocator implementation for small objects
//! (< architecture page size).
//!
//! The organization is as follows (top-down):
//!
//!  * A `ZoneAllocator` manages many `SCAllocator` and can
//!    satisfy requests for different allocation sizes.
//!  * A `SCAllocator` allocates objects of exactly one size.
//!    It holds its data in a ObjectPageList.
//!  * A `ObjectPage` contains allocated objects and associated meta-data.
#![allow(unused_features)]
#![cfg_attr(
    test,
    feature(prelude_import, test, raw, libc, c_void_variant, core_intrinsics)
)]
#![no_std]
#![crate_name = "slabmalloc"]
#![crate_type = "lib"]

#[cfg(test)]
#[macro_use]
extern crate std;
#[cfg(test)]
extern crate test;

#[cfg(test)]
mod tests;

use core::alloc::{GlobalAlloc, Layout};
use core::fmt;
use core::mem;
use core::ptr::{self, NonNull};

use log::trace;
use spin::Mutex;

#[cfg(target_arch = "x86_64")]
const CACHE_LINE_SIZE: usize = 64;

#[cfg(target_arch = "x86_64")]
const BASE_PAGE_SIZE: usize = 4096;

#[cfg(target_arch = "x86_64")]
#[allow(unused)]
const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
type VAddr = usize;

const MAX_SIZE_CLASSES: usize = 10;

#[derive(Debug)]
pub enum AllocationError {
    /// Can't satisfy the request for Layout
    OutOfMemory(Layout),
    /// Allocator can't deal with the provided Layout
    InvalidLayout,
}

pub struct SafeZoneAllocator(Mutex<ZoneAllocator<'static>>);

impl Default for SafeZoneAllocator {
    fn default() -> SafeZoneAllocator {
        SafeZoneAllocator(Mutex::new(Default::default()))
    }
}

unsafe impl GlobalAlloc for SafeZoneAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match self.0.lock().allocate(layout) {
            Ok(nptr) => nptr.as_ptr(),
            Err(AllocationError::OutOfMemory(_l)) => panic!("No memory in slabs, needs refilling"),
            Err(AllocationError::InvalidLayout) => panic!("Can't allocate this size"),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(nptr) = NonNull::new(ptr) {
            self.0
                .lock()
                .deallocate(nptr, layout)
                .expect("Couldn't deallocate");
        } else {
            // Nothing to do (don't dealloc null pointers).
        }
    }
}

/// A zone allocator.
///
/// Has a bunch of size class allocators and can serve
/// allocation requests for many different (MAX_SIZE_CLASSES) object sizes
/// (by selecting the right slab allocator).
pub struct ZoneAllocator<'a> {
    slabs: [SCAllocator<'a>; MAX_SIZE_CLASSES],
}

impl<'a> Default for ZoneAllocator<'a> {
    fn default() -> ZoneAllocator<'a> {
        ZoneAllocator {
            slabs: [
                SCAllocator::new(8),
                SCAllocator::new(16),
                SCAllocator::new(32),
                SCAllocator::new(64),
                SCAllocator::new(128),
                SCAllocator::new(256),
                SCAllocator::new(512),
                SCAllocator::new(1024),
                SCAllocator::new(2048),
                SCAllocator::new(4032),
            ],
        }
    }
}

impl<'a> ZoneAllocator<'a> {
    pub const MAX_ALLOC_SIZE: usize = 4032;

    /// Return maximum size an object of size `current_size` can use.
    ///
    /// Used to optimize `realloc`.
    #[allow(dead_code)]
    fn get_max_size(current_size: usize) -> Option<usize> {
        match current_size {
            0..=8 => Some(8),
            9..=16 => Some(16),
            17..=32 => Some(32),
            33..=64 => Some(64),
            65..=128 => Some(128),
            129..=256 => Some(256),
            257..=512 => Some(512),
            513..=1024 => Some(1024),
            1025..=2048 => Some(2048),
            2049..=4032 => Some(4032),
            _ => None,
        }
    }

    /// Figure out index into zone array to get the correct slab allocator for that size.
    fn get_slab_idx(requested_size: usize) -> Option<usize> {
        match requested_size {
            0..=8 => Some(0),
            9..=16 => Some(1),
            17..=32 => Some(2),
            33..=64 => Some(3),
            65..=128 => Some(4),
            129..=256 => Some(5),
            257..=512 => Some(6),
            513..=1024 => Some(7),
            1025..=2048 => Some(8),
            2049..=4032 => Some(9),
            _ => None,
        }
    }

    /// Tries to locate a slab allocator.
    ///
    /// Returns either a index into the slab array or None in case
    /// the requested allocation size can not be satisfied by
    /// any of the available slabs.
    fn try_acquire_slab(&mut self, size: usize) -> Option<usize> {
        ZoneAllocator::get_slab_idx(size).map(|idx| {
            if self.slabs[idx].size == 0 {
                self.slabs[idx].size = size;
            }
            idx
        })
    }

    /// Refills the SCAllocator for a given Layouyt with an ObjectPage.
    ///
    /// # Safety
    /// ObjectPage needs to be emtpy etc.
    pub unsafe fn refill(
        &mut self,
        layout: Layout,
        new_page: &'a mut ObjectPage<'a>,
    ) -> Result<(), AllocationError> {
        match ZoneAllocator::get_slab_idx(layout.size()) {
            Some(idx) => {
                self.slabs[idx].insert_empty(new_page);
                Ok(())
            }
            None => Err(AllocationError::InvalidLayout),
        }
    }

    /// Allocate a pointer to a block of memory of size `size` with alignment `align`.
    ///
    /// Can return None in case the zone allocator can not satisfy the allocation
    /// of the requested size or if we do not have enough memory.
    /// In case we are out of memory we try to refill the slab using our local pager
    /// and re-try the allocation request once more before we give up.
    pub fn allocate(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocationError> {
        match self.try_acquire_slab(layout.size()) {
            Some(idx) => self.slabs[idx].allocate(layout),
            None => Err(AllocationError::InvalidLayout),
        }
    }

    /// Deallocates a pointer to a block of memory previously allocated by `allocate`.
    ///
    /// # Arguments
    ///  * `ptr` - Address of the memory location to free.
    ///  * `old_size` - Size of the block.
    ///  * `align` - Alignment of the block.
    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) -> Result<(), AllocationError> {
        match self.try_acquire_slab(layout.size()) {
            Some(idx) => self.slabs[idx].deallocate(ptr, layout),
            None => Err(AllocationError::InvalidLayout),
        }
    }
}

/// A list of ObjectPage.
struct ObjectPageList<'a> {
    /// Points to the head of the list.
    head: Option<&'a mut ObjectPage<'a>>,
    /// Number of elements in the list.
    pub elements: usize,
}

impl<'a> ObjectPageList<'a> {
    fn new() -> ObjectPageList<'a> {
        ObjectPageList {
            head: None,
            elements: 0,
        }
    }

    fn iter_mut<'b>(&'b mut self) -> ObjectPageIterMut<'a> {
        let m = match self.head {
            None => Rawlink::none(),
            Some(ref mut m) => Rawlink::some(*m),
        };
        ObjectPageIterMut { head: m }
    }

    /// Inserts `new_head` at the front of the list.
    fn insert_front<'b>(&'b mut self, mut new_head: &'a mut ObjectPage<'a>) {
        match self.head {
            None => {
                new_head.prev = Rawlink::none();
                self.head = Some(new_head);
            }
            Some(ref mut head) => {
                new_head.prev = Rawlink::none();
                head.prev = Rawlink::some(new_head);
                mem::swap(head, &mut new_head);
                head.next = Rawlink::some(new_head);
            }
        }

        self.elements += 1;
    }

    /// Removes `slab_page` from the list.
    fn remove_from_list<'b, 'c>(&'b mut self, slab_page: &'c mut ObjectPage<'a>) {
        unsafe {
            match slab_page.prev.resolve_mut() {
                None => {
                    self.head = slab_page.next.resolve_mut();
                }
                Some(prev) => {
                    prev.next = match slab_page.next.resolve_mut() {
                        None => Rawlink::none(),
                        Some(next) => Rawlink::some(next),
                    };
                }
            }

            match slab_page.next.resolve_mut() {
                None => (),
                Some(next) => {
                    next.prev = match slab_page.prev.resolve_mut() {
                        None => Rawlink::none(),
                        Some(prev) => Rawlink::some(prev),
                    };
                }
            }
        }

        slab_page.prev = Rawlink::none();
        slab_page.next = Rawlink::none();
        self.elements -= 1;
    }

    /// Removes `slab_page` from the list.
    fn pop<'b, 'c>(&'b mut self) -> Option<&'c mut ObjectPage<'a>> {
        match self.head {
            None => None,
            Some(ref mut head) => {
                let mut new_head = unsafe { head.next.resolve_mut() };
                mem::swap(&mut self.head, &mut new_head);
                let _ = self.head.as_mut().map(|n| {
                    n.prev = Rawlink::none();
                });

                self.elements -= 1;
                new_head.map(|node| {
                    node.prev = Rawlink::none();
                    node.next = Rawlink::none();
                    node
                })
            }
        }
    }

    /// Does the list contain `s`?
    fn contains(&mut self, s: *const ObjectPage<'a>) -> bool {
        for slab_page in self.iter_mut() {
            if slab_page as *const ObjectPage == s as *const ObjectPage {
                return true;
            }
        }

        false
    }
}

/// Iterate over all the pages inside a slab allocator
struct ObjectPageIterMut<'a> {
    head: Rawlink<ObjectPage<'a>>,
}

impl<'a> Iterator for ObjectPageIterMut<'a> {
    type Item = &'a mut ObjectPage<'a>;

    #[inline]
    fn next(&mut self) -> Option<&'a mut ObjectPage<'a>> {
        unsafe {
            self.head.resolve_mut().map(|next| {
                self.head = match next.next.resolve_mut() {
                    None => Rawlink::none(),
                    Some(ref mut sp) => Rawlink::some(*sp),
                };
                next
            })
        }
    }
}

/// A slab allocator allocates elements of a fixed size.
///
/// It has a 3 lists of ObjectPages from which it can allocate memory.
///
///  * `empty_slabs`: Is a list of pages that the SCAllocator maintains, but
///    has 0 allocations in them, these can be given back to a requestor in case
///    of reclamation.
///  * `slabs`: A list of pages partially allocated and still have room for more.
///  * `full_slabs`: A list of pages that are completely allocated.
///
/// On allocation we allocate memory from `slabs`, however if the list is empty
/// we try to reclaim a page from `empty_slabs` before we return with an out-of-memory
/// error. If a page becomes full after the allocation we move it from `slabs` to
/// `full_slabs`.
///
/// Similarly, on dealloaction we might move a page from `full_slabs` to `slabs`
/// or from `slabs` to `empty_slabs` after we deallocated an object.
pub struct SCAllocator<'a> {
    /// Maximum possible allocation size for this `SCAllocator`.
    size: usize,
    /// max objects per page
    obj_per_page: usize,
    /// List of empty ObjectPages (nothing allocated in these).
    empty_slabs: ObjectPageList<'a>,
    /// List of partially used ObjectPage (some objects allocated but pages are not full).
    slabs: ObjectPageList<'a>,
    /// List of full ObjectPages (everything allocated in these don't need to search them).
    full_slabs: ObjectPageList<'a>,
}

impl<'a> SCAllocator<'a> {
    /// Create a new SCAllocator.
    pub fn new(size: usize) -> SCAllocator<'a> {
        // const_assert!(size < (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE);
        let obj_per_page = core::cmp::min((BASE_PAGE_SIZE - 64) / size, 6 * 64);

        SCAllocator {
            size,
            obj_per_page,
            empty_slabs: ObjectPageList::new(),
            slabs: ObjectPageList::new(),
            full_slabs: ObjectPageList::new(),
        }
    }

    /// Return object size of this allocator.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Refill the SCAllocator
    ///
    /// # Safety
    /// ObjectPage needs to be empty etc.
    pub unsafe fn refill(&mut self, page: &'a mut ObjectPage<'a>) {
        page.bitfield.initialize(self.size, BASE_PAGE_SIZE - 64);
        trace!("adding page to SCAllocator {:p}", page);
        self.insert_empty(page);
    }

    /// Add a new ObjectPage.
    fn insert_partial_slab(&mut self, new_head: &'a mut ObjectPage<'a>) {
        self.slabs.insert_front(new_head);
    }

    /// Add page to empty list.
    fn insert_empty(&mut self, new_head: &'a mut ObjectPage<'a>) {
        self.empty_slabs.insert_front(new_head);
    }

    /// Move a page from `slabs` to `empty_slabs`.
    fn move_to_empty(&mut self, page: &'a mut ObjectPage<'a>) {
        let page_ptr = page as *const ObjectPage;

        debug_assert!(self.slabs.contains(page_ptr));
        debug_assert!(
            !self.empty_slabs.contains(page_ptr),
            "Page {:p} already in emtpy_slabs",
            page_ptr
        );

        self.slabs.remove_from_list(page);
        self.empty_slabs.insert_front(page);

        debug_assert!(!self.slabs.contains(page_ptr));
        debug_assert!(self.empty_slabs.contains(page_ptr));
    }

    /// Move a page from `full_slabs` to `slab`.
    fn move_partial_to_full(&mut self, page: &'a mut ObjectPage<'a>) {
        let page_ptr = page as *const ObjectPage;

        debug_assert!(self.slabs.contains(page_ptr));
        debug_assert!(!self.full_slabs.contains(page_ptr));

        self.slabs.remove_from_list(page);
        self.full_slabs.insert_front(page);

        debug_assert!(!self.slabs.contains(page_ptr));
        debug_assert!(self.full_slabs.contains(page_ptr));
    }

    /// Move a page from `full_slabs` to `slab`.
    fn move_full_to_partial(&mut self, page: &'a mut ObjectPage<'a>) {
        let page_ptr = page as *const ObjectPage;

        debug_assert!(!self.slabs.contains(page_ptr));
        debug_assert!(self.full_slabs.contains(page_ptr));

        self.full_slabs.remove_from_list(page);
        self.slabs.insert_front(page);

        debug_assert!(self.slabs.contains(page_ptr));
        debug_assert!(!self.full_slabs.contains(page_ptr));
    }

    /// Tries to allocate a block of memory with respect to the `alignment`.
    /// Searches within already allocated slab pages, if no suitable spot is found
    /// will try to use a page from the empty page list.
    ///
    /// # Arguments
    ///  * `sc_layout`: This is not the original layout but adjusted for the
    ///     SCAllocator size (>= original).
    fn try_allocate_from_pagelist(&mut self, sc_layout: Layout) -> *mut u8 {
        // TODO: Do we really need to check multiple slab pages (due to alignment)
        // If not we can get away with a singly-linked list and have 8 more bytes
        // for the bitfield in an ObjectPage.
        for slab_page in self.slabs.iter_mut() {
            let ptr = slab_page.allocate(sc_layout);
            if !ptr.is_null() {
                if slab_page.is_full() {
                    trace!("move {:p} partial -> full", slab_page);
                    self.move_partial_to_full(slab_page);
                }
                return ptr;
            } else {
                continue;
            }
        }

        ptr::null_mut()
    }

    /// Allocates a block of memory with respect to `alignment`.
    ///
    /// In case of failure will try to grow the slab allocator by requesting
    /// additional pages and re-try the allocation once more before we give up.
    pub fn allocate(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocationError> {
        trace!(
            "SCAllocator({}) is trying to allocate {:?}",
            self.size,
            layout
        );
        assert!(layout.size() <= self.size);
        assert!(self.size <= (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE));
        let new_layout = unsafe { Layout::from_size_align_unchecked(self.size, layout.align()) };
        assert!(new_layout.size() >= layout.size());

        let ptr = {
            // Try to allocate from partial slabs,
            // if we fail check if we have empty pages and allocate from there
            let ptr = self.try_allocate_from_pagelist(new_layout);
            if ptr.is_null() && self.empty_slabs.head.is_some() {
                // Re-try allocation in empty page
                let empty_page = self.empty_slabs.pop().expect("We checked head.is_some()");
                debug_assert!(!self.empty_slabs.contains(empty_page));

                let ptr = empty_page.allocate(layout);
                debug_assert!(!ptr.is_null(), "Allocation must have succeeded here.");

                trace!(
                    "move {:p} empty -> partial empty count {}",
                    empty_page,
                    self.empty_slabs.elements
                );
                // Move empty page to partial pages
                self.insert_partial_slab(empty_page);
                ptr
            } else {
                ptr
            }
        };

        let res = NonNull::new(ptr).ok_or(AllocationError::OutOfMemory(layout));

        if !ptr.is_null() {
            trace!(
                "SCAllocator({}) allocated ptr=0x{:x}",
                self.size,
                ptr as usize
            );
        }

        res
    }

    /// Deallocates a previously allocated block.
    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) -> Result<(), AllocationError> {
        assert!(layout.size() <= self.size);
        assert!(self.size <= (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE));
        trace!(
            "SCAllocator({}) is trying to deallocate ptr = {:p} layout={:?}",
            self.size,
            ptr,
            layout
        );

        let page = (ptr.as_ptr() as usize) & !(BASE_PAGE_SIZE - 1) as usize;

        // Figure out which page we are on and construct a reference to it
        // TODO: The linked list will have another &mut reference
        let slab_page = unsafe { mem::transmute::<VAddr, &'a mut ObjectPage>(page) };
        let new_layout = unsafe { Layout::from_size_align_unchecked(self.size, layout.align()) };

        let slab_page_was_full = slab_page.is_full();
        let ret = slab_page.deallocate(ptr, new_layout);
        debug_assert!(ret.is_ok(), "Slab page deallocate won't fail at the moment");

        if slab_page.is_empty(self.obj_per_page) {
            // We need to move it from self.slabs -> self.empty_slabs
            trace!("move {:p} partial -> empty", slab_page);
            self.move_to_empty(slab_page);
        } else if slab_page_was_full {
            // We need to move it from self.full_slabs -> self.slabs
            trace!("move {:p} full -> partial", slab_page);
            self.move_full_to_partial(slab_page);
        }

        ret
    }
}

/// A trait defining bitfield operations we need for tracking allocated objects within a page.
trait Bitfield {
    fn initialize(&mut self, for_size: usize, capacity: usize);
    fn first_fit(&self, base_addr: usize, layout: Layout) -> Option<(usize, usize)>;
    fn is_allocated(&self, idx: usize) -> bool;
    fn set_bit(&mut self, idx: usize);
    fn clear_bit(&mut self, idx: usize);
    fn is_full(&self) -> bool;
    fn all_free(&self, relevant_bits: usize) -> bool;
}

/// Implementation of bit operations on [u64] arrays.
impl Bitfield for [u64] {
    /// Initialize the bitfield
    ///
    /// # Arguments
    ///  * `for_size`: Object size we want to allocate
    ///  * `capacity`: Maximum size of the buffer the bitmap maintains.
    ///
    /// Ensures that we only have free slots for what we can allocate
    /// within the page (by marking everything else allocated).
    fn initialize(&mut self, for_size: usize, capacity: usize) {
        // Set everything to allocated
        for bitmap in self.iter_mut() {
            *bitmap = u64::max_value();
        }

        // Mark actual slots as free
        let relevant_bits = core::cmp::min(capacity / for_size, self.len() * 64);
        for idx in 0..relevant_bits {
            self.clear_bit(idx);
        }
    }

    /// Tries to find a free block of memory that satisfies `alignment` requirement.
    ///
    /// # Notes
    /// * We pass size here to be able to calculate the resulting address within `data`.
    #[inline(always)]
    fn first_fit(&self, base_addr: usize, layout: Layout) -> Option<(usize, usize)> {
        for (base_idx, b) in self.iter().enumerate() {
            let bitval = *b;
            if bitval == u64::max_value() {
                continue;
            } else {
                let negated = !bitval;
                let first_free = negated.trailing_zeros() as usize;
                let idx: usize = base_idx * 64 + first_free;
                let offset = idx * layout.size();

                let offset_inside_data_area =
                    offset <= (BASE_PAGE_SIZE - CACHE_LINE_SIZE - layout.size());
                if !offset_inside_data_area {
                    return None;
                }

                let addr: usize = base_addr + offset;
                let alignment_ok = addr % layout.align() == 0;
                let block_is_free = bitval & (1 << first_free) == 0;
                if alignment_ok && block_is_free {
                    return Some((idx, addr));
                }
            }
        }
        None
    }

    /// Check if the bit `idx` is set.
    #[inline(always)]
    fn is_allocated(&self, idx: usize) -> bool {
        let base_idx = idx / 64;
        let bit_idx = idx % 64;
        (self[base_idx] & (1 << bit_idx)) > 0
    }

    /// Sets the bit number `idx` in the bit-field.
    #[inline(always)]
    fn set_bit(&mut self, idx: usize) {
        let base_idx = idx / 64;
        let bit_idx = idx % 64;
        self[base_idx] |= 1 << bit_idx;
    }

    /// Clears bit number `idx` in the bit-field.
    #[inline(always)]
    fn clear_bit(&mut self, idx: usize) {
        let base_idx = idx / 64;
        let bit_idx = idx % 64;
        self[base_idx] &= !(1 << bit_idx);
    }

    /// Checks if we could allocate more objects of a given `alloc_size` within the
    /// `capacity` of the memory allocator.
    ///
    /// # Note
    /// The ObjectPage will make sure to mark the top-most bits as allocated
    /// for large sizes (i.e., a size 512 SCAllocator will only really need 3 bits)
    /// to track allocated objects). That's why this function can be simpler
    /// than it would need to be in practice.
    #[inline(always)]
    fn is_full(&self) -> bool {
        self.iter().filter(|&x| *x != u64::max_value()).count() == 0
    }

    /// Checks if the page has currently no allocations.
    ///
    /// This is called `all_free` rather than `is_emtpy` because
    /// we already have an is_empty fn as part of the slice.
    #[inline(always)]
    fn all_free(&self, relevant_bits: usize) -> bool {
        for (idx, bitmap) in self.iter().enumerate() {
            let checking_bit_range = (idx * 64, (idx + 1) * 64);
            if relevant_bits >= checking_bit_range.0 && relevant_bits < checking_bit_range.1 {
                // Last relevant bitmap, here we only have to check that a subset of bitmap is marked free
                // the rest will be marked full
                let bits_that_should_be_free = relevant_bits - checking_bit_range.0;
                let free_mask = (1 << bits_that_should_be_free) - 1;
                return (free_mask & *bitmap) == 0;
            }

            if *bitmap == 0 {
                continue;
            } else {
                return false;
            }
        }

        true
    }
}

/// Holds allocated data.
///
/// Objects life within data and meta tracks the objects status.
/// Currently, `bitfield`, `next` and `prev` pointer should fit inside
/// a single cache-line.
///
/// # Notes
/// Marked `repr(C)` because we rely on a well defined order of struct
/// members (e.g., dealloc does a cast to find the bitfield).
#[repr(C)]
pub struct ObjectPage<'a> {
    /// Holds memory objects.
    #[allow(dead_code)]
    data: [u8; 4096 - 64],

    /// Next element in list (used by `ObjectPageList`).
    next: Rawlink<ObjectPage<'a>>,
    prev: Rawlink<ObjectPage<'a>>,

    /// A bit-field to track free/allocated memory within `data`.
    ///
    /// # Notes
    /// * With only 48 bits we do waste some space at the end of every page for 8 bytes allocations.
    ///   but 12 bytes on-wards is okay.
    bitfield: [u64; 6],
}

impl<'a> Default for ObjectPage<'a> {
    fn default() -> ObjectPage<'a> {
        unsafe { mem::MaybeUninit::zeroed().assume_init() }
    }
}

impl<'a> fmt::Debug for ObjectPage<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ObjectPage")
    }
}

impl<'a> ObjectPage<'a> {
    /// Tries to find a free block within `data` that satisfies `alignment` requirement.
    fn first_fit(&self, layout: Layout) -> Option<(usize, usize)> {
        let base_addr = (&*self as *const ObjectPage) as usize;
        self.bitfield.first_fit(base_addr, layout)
    }

    /// Deallocates a memory object within this page.
    fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) -> Result<(), AllocationError> {
        trace!("ObjectPage deallocating ptr = {:p} with {:?}", ptr, layout);
        let page_offset = (ptr.as_ptr() as usize) & 0xfff;
        assert!(page_offset % layout.size() == 0);
        let idx = page_offset / layout.size();
        assert!(
            self.bitfield.is_allocated(idx),
            "{:p} not marked allocated?",
            ptr
        );

        self.bitfield.clear_bit(idx);
        Ok(())
    }

    /// Tries to allocate an object within this page.
    ///
    /// In case the Slab is full, returns None.
    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        match self.first_fit(layout) {
            Some((idx, addr)) => {
                self.bitfield.set_bit(idx);
                addr as *mut u8
            }
            None => ptr::null_mut(),
        }
    }

    /// Checks if we can still allocate more objects of a given layout within the page.
    fn is_full(&self) -> bool {
        self.bitfield.is_full()
    }

    /// Checks if the page has currently no allocations.
    fn is_empty(&self, relevant_bits: usize) -> bool {
        self.bitfield.all_free(relevant_bits)
    }
}

/// Rawlink is a type like Option<T> but for holding a raw pointer
struct Rawlink<T> {
    p: *mut T,
}

impl<T> Default for Rawlink<T> {
    fn default() -> Self {
        Rawlink { p: ptr::null_mut() }
    }
}

impl<T> Rawlink<T> {
    /// Like Option::None for Rawlink
    fn none() -> Rawlink<T> {
        Rawlink { p: ptr::null_mut() }
    }

    /// Like Option::Some for Rawlink
    fn some(n: &mut T) -> Rawlink<T> {
        Rawlink { p: n }
    }

    /// Convert the `Rawlink` into an Option value
    ///
    /// **unsafe** because:
    ///
    /// - Dereference of raw pointer.
    /// - Returns reference of arbitrary lifetime.
    #[allow(dead_code)]
    unsafe fn resolve<'a>(&self) -> Option<&'a T> {
        self.p.as_ref()
    }

    /// Convert the `Rawlink` into an Option value
    ///
    /// **unsafe** because:
    ///
    /// - Dereference of raw pointer.
    /// - Returns reference of arbitrary lifetime.
    unsafe fn resolve_mut<'a>(&mut self) -> Option<&'a mut T> {
        self.p.as_mut()
    }

    /// Return the `Rawlink` and replace with `Rawlink::none()`
    #[allow(dead_code)]
    fn take(&mut self) -> Rawlink<T> {
        mem::replace(self, Rawlink::none())
    }
}
