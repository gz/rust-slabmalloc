use std::prelude::v1::*;
use std::mem::{transmute};
use libc;

// The types we want to test:
use super::{ZoneAllocator, SlabPage, SlabPageMeta, SlabAllocator, SlabPageAllocator};

#[cfg(target_arch="x86_64")]
use x86::paging::{CACHE_LINE_SIZE, BASE_PAGE_SIZE};

/// Page allocator based on mmap/munmap system calls for backing slab memory.
struct MmapSlabAllocator;

/// mmap/munmap page allocator implementation.
impl<'a> SlabPageAllocator<'a> for MmapSlabAllocator {
    fn allocate_slabpage(&self) -> Option<&'a mut SlabPage<'a>> {
        let mut addr: libc::c_void = libc::c_void::__variant1;
        let len: libc::size_t = BASE_PAGE_SIZE;
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
        let fd = -1;
        let offset = 0;
        let r = unsafe { libc::mmap(&mut addr, len as libc::size_t, prot, flags, fd, offset) };
        if r == libc::MAP_FAILED {
            panic!("mmap failed!");
            return None;
        }
        else {
            let mut slab_page: &'a mut SlabPage = unsafe { transmute(r as usize) };
            return Some(slab_page);
        }
    }

    fn release_slabpage(&self, p: &'a SlabPage) {
        let addr: *mut libc::c_void = unsafe { transmute(p) };
        let len: libc::size_t = BASE_PAGE_SIZE;
        let r = unsafe { libc::munmap(addr, len) };
        if r != 0 {
            panic!("munmap failed!");
        }
    }

}

#[test]
fn type_size() {
    use std::mem::size_of;
    assert!(CACHE_LINE_SIZE as usize == size_of::<SlabPageMeta>(),
               "Meta-data within page should not be larger than a single cache-line.");
    assert!(BASE_PAGE_SIZE as usize == size_of::<SlabPage>(),
               "SlabPage should be exactly the size of a single page.");
}

#[test]
fn test_mmap_allocator() {
    let mmap = MmapSlabAllocator;
    match mmap.allocate_slabpage() {
        Some(sp) =>  {
            assert!(!sp.is_full(), "Got empty slab");
            mmap.release_slabpage(sp)
        },
        None => panic!("failed to allocate slabpage")
    }
}

#[test]
fn test_slab_allocator() {
    let mmap = MmapSlabAllocator;
    let mut sa: SlabAllocator = SlabAllocator{
        size: 8,
        pager: &mmap,
        allocateable_elements: 0,
        allocateable: None,
    };
    let alignment = 1;


    let mut objects: Vec<*mut u8> = Vec::new();
    let mut vec: Vec<&mut usize> = Vec::new();
    for i in 0..4096 {
        match sa.allocate(alignment) {
            None => panic!("OOM is unlikely."),
            Some(obj) => {
                unsafe { vec.push(transmute(obj)) };
                objects.push(obj)
            }
        }
    }
    let upper: usize = 0xffffffff00000000;

    for (idx, item) in vec.iter_mut().enumerate() {
        **item = idx + upper;
    }

    // No two allocation point to the same memory:
    for (idx, item) in vec.iter().enumerate() {
        assert!( (**item) == (idx + upper));
    }

    // Deallocate all the things:
    for item in objects.iter_mut() {
        sa.deallocate(*item);
    }


    println!("sa.allocateable_elements = {:?}", sa.allocateable_elements);
}
