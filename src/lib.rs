#![allow(unused_features, dead_code, unused_variables)]

#![feature(no_std, core, raw, ptr_as_ref, core_prelude, core_slice_ext, libc)]

#![no_std]

#![crate_name = "slabmalloc"]
#![crate_type = "lib"]


#[cfg(test)]
#[macro_use]
extern crate std;

#[cfg(test)]
#[prelude_import]
use std::prelude::v1::*;

#[macro_use]
extern crate core;
#[cfg(not(test))]
use core::prelude::*;
use core::mem;
use core::ptr;

#[cfg(target_arch="x86_64")]
extern crate x86;
#[cfg(target_arch="x86_64")]
use x86::paging::{VAddr, CACHE_LINE_SIZE, BASE_PAGE_SIZE};

#[cfg(test)]
extern crate libc;
#[cfg(test)]
extern crate rand;
#[cfg(test)]
mod tests;

pub const EMPTY: *mut () = 0x1 as *mut ();
pub const MAX_SLABS: usize = 9;

/// The memory backing as used by the SlabAllocator.
/// A client that wants to use the Zone/Slab allocators
/// has to provide this interface.
trait SlabPageAllocator<'a> {
    fn allocate_slabpage(&self) -> Option<&'a mut SlabPage<'a>>;
    fn release_slabpage(&self, &'a SlabPage);
}

/// A zone allocator has a bunch of slab allocators and can serve
/// allocation requests for many different (MAX_SLABS) object sizes
/// (by selecting the right slab allocator).
pub struct ZoneAllocator {
    slabs: [SlabAllocator<'static>; MAX_SLABS]
}

impl ZoneAllocator{

    /// Round-up the requested size to fit one of the slab allocators.
    fn get_size_class(requested_size: usize) -> Option<usize> {
        if requested_size <= 8 {
            Some(8)
        }
        else if requested_size > (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE) {
            None
        }
        else {
            Some(requested_size.next_power_of_two())
        }
    }

    /// Figure out index into zone array to get the correct slab allocator for the size.
    fn get_slab_idx(size_class: usize) -> Option<usize> {
        if size_class > (BASE_PAGE_SIZE as usize) {
            return None;
        }
        Some(size_class.trailing_zeros() as usize - 3)
    }

    fn try_acquire_slab(&mut self, size: usize) -> Option<usize> {
        match ZoneAllocator::get_size_class(size) {
            None => None,
            Some(size_class) => match ZoneAllocator::get_slab_idx(size_class) {
                None => None,
                Some(idx) => {
                    if self.slabs[idx].size == 0 {
                        //log!("Initialize slab at idx {} / size_class {}", idx, size_class);
                        self.slabs[idx].size = size_class;
                    }
                    Some(idx)
                }
            }
        }
    }

    pub fn allocate(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        match self.try_acquire_slab(size) {
            Some(idx) => self.slabs[idx].allocate(align),
            None => panic!("Unable to find slab allocator for size ({})", size)
        }
    }

    pub fn deallocate(&mut self, ptr: *mut u8, old_size: usize, align: usize) {
        match self.try_acquire_slab(old_size) {
            Some(idx) => self.slabs[idx].deallocate(ptr),
            None => panic!("Unable to find slab allocator for size ({}) with ptr {:?}", old_size, ptr)
        }
    }
}

/// A slab allocator allocates elements of a fixed size and stores
/// them within a list of pages.
pub struct SlabAllocator<'a> {
    pub size: usize,
    pager: &'a SlabPageAllocator<'a>,
    pub allocateable_elements: usize,
    allocateable: Option<&'a mut SlabPage<'a>>,
}

impl<'a> SlabAllocator<'a> {

    fn refill_slab<'b>(&'b mut self, amount: usize) {
        match self.pager.allocate_slabpage() {
            Some(new_head) => {
                SlabAllocator::insert_front(&mut self.allocateable, new_head);
                self.allocateable_elements += 1;
            },
            None => panic!("OOM")
        }
    }

    fn try_allocate<'b>(&'b mut self, alignment: usize) -> Option<*mut u8> {

        for slab_page in self.allocateable.iter_mut() {
            match slab_page.allocate(self.size, alignment) {
                None => { () },
                Some(obj) => {
                    return Some(obj as *mut u8);
                }
            };
        }

        None
    }

    #[cfg(not(test))]
    fn has_slabpage(&mut self, s: &SlabPage<'a>) -> bool {
        for slab_page in self.allocateable.iter_mut() {
            if (*slab_page as *const SlabPage) == (s as *const SlabPage) {

                return true;
            }
        }
        return false;
    }


    #[cfg(test)]
    fn has_slabpage(&mut self, s: &SlabPage<'a>) -> bool {
        match self.allocateable {
            Some(ref mut list) => {
                for slab_page in list.iter_mut() {
                    println!("compare {:?} == {:?}",
                             (slab_page as *const SlabPage),
                             (s as *const SlabPage));
                }
                false
            },
            None => false
        }
    }

    pub fn allocate<'b>(&'b mut self, alignment: usize) -> Option<*mut u8> {
        assert!(self.size < (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE));

        match self.try_allocate(alignment) {
            None => {
                self.refill_slab(1);
                self.try_allocate(alignment)
            }
            Some(obj) => Some(obj)
        }
    }

    fn remove_from_list<'b>(head: &'b mut Option<&'a mut SlabPage<'a>>, p: &'a mut SlabPage<'a>) {
        unsafe {
            match p.meta.prev.resolve_mut() {
                None => {
                    match p.meta.next.resolve_mut() {
                        None => { *head = None; },
                        Some(next_page) => { *head = Some(next_page) }
                    }
                },
                Some(prev_page) => {
                    match p.meta.next.resolve_mut() {
                        None => { prev_page.meta.next = Rawlink::none(); },
                        Some(next_page) => { prev_page.meta.next = Rawlink::some(next_page) }
                    }
                }
            }
        }
    }

    fn insert_front<'b>(head: &'b mut Option<&'a mut SlabPage<'a>>, new_head: &'a mut SlabPage<'a>) {
        match *head {
            None => { *head = Some(new_head); }
            Some(ref mut current_head) => {
                current_head.meta.prev = Rawlink::some(new_head);
                new_head.meta.next = Rawlink::some(current_head);
                *current_head = new_head;
            }
        }
    }

    pub fn deallocate(&mut self, ptr: *mut u8) {
        let page = (ptr as usize) & !0xfff;
        let mut slab_page = unsafe {
            mem::transmute::<VAddr, &'a mut SlabPage>(page)
        };
        assert!(self.has_slabpage(slab_page));

        slab_page.deallocate(ptr, self.size);
    }

}

unsafe impl<'a> Send for SlabPage<'a> { }
unsafe impl<'a> Sync for SlabPage<'a> { }

/// Holds allocated data. Objects life within data and meta tracks the objects
/// status.
pub struct SlabPage<'a> {
    data: [u8; 4096 - 64],
    meta: SlabPageMeta<'a>
}

/// Meta-data stored at the end of a page to track allocations within the page.
/// This structure ideally fits inside a single cache-line.
pub struct SlabPageMeta<'a> {
    prev: Rawlink<SlabPage<'a>>,
    next: Rawlink<SlabPage<'a>>,
    // Note: with only 48 bits we do waste some space for the
    // 8 bytes slab allocator. But 12 bytes on-wards is ok.
    bitfield: [u8; CACHE_LINE_SIZE - 16]
}

/// Iterate over all the pages in the slab allocator
pub struct SlabPageIter<'a> {
    head: Rawlink<SlabPage<'a>>,
    nelem: usize,
}

impl<'a> SlabPage<'a> {

    pub fn iter_mut(&mut self) -> SlabPageIter<'a> {
        SlabPageIter {
            nelem: 2,
            head: Rawlink::some(self),
        }
    }

    fn first_fit(&self, size: usize, alignment: usize) -> Option<(usize, usize)> {
        assert!(alignment.is_power_of_two());
        for (base_idx, b) in self.meta.bitfield.iter().enumerate() {
            for bit_idx in 0..8 {
                let idx: usize = base_idx * 8 + bit_idx;
                let offset = idx * size;

                let offset_iniside_data_area = offset <=
                    (BASE_PAGE_SIZE as usize - CACHE_LINE_SIZE as usize - size);
                if !offset_iniside_data_area {
                    return None;
                }

                let addr: usize = ((self as *const SlabPage) as usize) + offset;
                let alignment_ok = addr % alignment == 0;
                let block_is_free = b & (1 << bit_idx) == 0;

                if alignment_ok && block_is_free {
                    return Some((idx, addr));
                }
            }
        }
        None
    }

    fn is_allocated(&mut self, idx: usize) -> bool {
        let base_idx = idx / 8;
        let bit_idx = idx % 8;

        (self.meta.bitfield[base_idx] & (1 << bit_idx)) > 0
    }

    fn set_bit(&mut self, idx: usize) {
        let base_idx = idx / 8;
        let bit_idx = idx % 8;
        self.meta.bitfield[base_idx] |= 1 << bit_idx;
    }

    fn clear_bit(&mut self, idx: usize) {
        let base_idx = idx / 8;
        let bit_idx = idx % 8;
        self.meta.bitfield[base_idx] &= !(1 << bit_idx);
    }

    pub fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        let page_offset = (ptr as usize) & 0xfff;
        assert!(page_offset % size == 0);
        let idx = page_offset / size;
        assert!(self.is_allocated(idx));
        self.clear_bit(idx);
    }

    pub fn allocate(&mut self, size: usize, alignment: usize) -> Option<*mut u8> {
        match self.first_fit(size, alignment) {
            Some((idx, addr)) => {
                self.set_bit(idx);
                Some(unsafe { mem::transmute::<usize, *mut u8>(addr) })
            }
            None => None
        }
    }

    pub fn is_full(&self) -> bool {
        self.meta.bitfield.iter().filter(|&x| *x != 0xff).count() == 0
    }

}

impl<'a> Iterator for SlabPageIter<'a> {
    type Item = &'a mut SlabPage<'a>;

    #[inline]
    fn next(&mut self) -> Option<&'a mut SlabPage<'a>> {
        if self.nelem == 0 {
            return None;
        }

        unsafe {
            return self.head.resolve_mut().map(|next| {
                self.nelem -= 1;
                self.head = next.meta.next;
                next
            });
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.nelem, Some(self.nelem))
    }
}

/// Rawlink is a type like Option<T> but for holding a raw pointer
struct Rawlink<T> {
    p: *mut T,
}

impl<T> Rawlink<T> {

    /// Like Option::None for Rawlink
    fn none() -> Rawlink<T> {
        Rawlink{ p: ptr::null_mut() }
    }

    /// Like Option::Some for Rawlink
    fn some(n: &mut T) -> Rawlink<T> {
        Rawlink{p: n}
    }

    /// Convert the `Rawlink` into an Option value
    ///
    /// **unsafe** because:
    ///
    /// - Dereference of raw pointer.
    /// - Returns reference of arbitrary lifetime.
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
    fn take(&mut self) -> Rawlink<T> {
        mem::replace(self, Rawlink::none())
    }
}