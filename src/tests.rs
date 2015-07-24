use std::prelude::v1::*;
use std::mem::{transmute};
use libc;
use rand;
use std::mem::size_of;

// The types we want to test:
use super::{DList, ZoneAllocator, SlabPage, SlabAllocator, SlabPageAllocator};

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
            return None;
        }
        else {
            let mut slab_page: &'a mut SlabPage = unsafe { transmute(r as usize) };
            return Some(slab_page);
        }
    }

    fn release_slabpage(&self, p: &'a SlabPage<'a>) {
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

macro_rules! test_slab_allocation {
    ( $test:ident, $size:expr, $alignment:expr, $allocations:expr  ) => {
        #[test]
        fn $test() {
            let mmap = MmapSlabAllocator;
            let mut sa: SlabAllocator = SlabAllocator{
                size: $size,
                pager: &mmap,
                allocateable: DList { head: None, head_elements: 0 },
            };
            let alignment = $alignment;

            let mut objects: Vec<*mut u8> = Vec::new();
            let mut vec: Vec<(usize, &mut [usize; $size / 8])> = Vec::new();

            for _ in 0..$allocations {
                match sa.allocate(alignment) {
                    None => panic!("OOM is unlikely."),
                    Some(obj) => {
                        unsafe { vec.push( (rand::random::<usize>(), transmute(obj)) ) };
                        objects.push(obj)
                    }
                }
            }

            // Write the objects with a random pattern
            for item in vec.iter_mut() {
                let (pattern, ref mut obj) = *item;
                for i in 0..obj.len() {
                    obj[i] = pattern;
                }
            }

            for item in vec.iter() {
                let (pattern, ref obj) = *item;
                //assert!(**obj as usize % alignment == 0, "Object allocation respects alignment.");
                for i in 0..obj.len() {
                    assert!( (obj[i]) == pattern, "No two allocations point to the same memory.");
                }
            }

            // Make sure we can correctly deallocate:
            let pages_allocated = sa.allocateable.head_elements;

            println!("Deallocate all the objects");
            for item in objects.iter_mut() {
                sa.deallocate(*item);
            }

            // then allocate everything again,
            for idx in 0..$allocations {
                println!("{:?}", idx);
                match sa.allocate(alignment) {
                    None => panic!("OOM is unlikely."),
                    Some(obj) => {
                        unsafe { vec.push( (rand::random::<usize>(), transmute(obj)) ) };
                        objects.push(obj)
                    }
                }
            }

            println!("{} {}", pages_allocated, sa.allocateable.head_elements);
            // and make sure we do not request more pages than what we had previously
            assert!(pages_allocated == sa.allocateable.head_elements,
                    "Did not use more memory for 2nd allocation run.");
        }

    };
}

macro_rules! test_slab_allocation_panic {
    ( $test:ident, $size:expr, $alignment:expr, $allocations:expr  ) => {
        test_slab_allocation!($test, $size, $alignment, $allocations);
    };
}

test_slab_allocation!(test_slab_allocation8192_size8_alignment1, 8, 1, 512);
//test_slab_allocation!(test_slab_allocation4096_size8_alignment8, 8, 8, 4096);
//test_slab_allocation!(test_slab_allocation500_size8_alignment64, 8, 64, 500);
//test_slab_allocation!(test_slab_allocation4096_size12_alignment1, 12, 1, 4096);
//test_slab_allocation!(test_slab_allocation4096_size13_alignment1, 13, 1, 4096);
//test_slab_allocation!(test_slab_allocation2000_size14_alignment1, 14, 1, 2000);
//test_slab_allocation!(test_slab_allocation4096_size15_alignment1, 15, 1, 4096);
//test_slab_allocation!(test_slab_allocation8000_size16_alignment1, 16, 1, 8000);
//test_slab_allocation!(test_slab_allocation1024_size24_alignment1, 24, 1, 1024);
//test_slab_allocation!(test_slab_allocation3090_size32_alignment1, 32, 1, 3090);
//test_slab_allocation!(test_slab_allocation4096_size64_alignment1, 64, 1, 4096);
//test_slab_allocation!(test_slab_allocation1000_size512_alignment1, 512, 1, 1000);
//test_slab_allocation!(test_slab_allocation4096_size1024_alignment1, 1024, 1, 4096);
//test_slab_allocation!(test_slab_allocation10_size2048_alignment1, 2048, 1, 10);

#[test]
#[should_panic]
fn allocation_too_big() {
    let mmap = MmapSlabAllocator;
    let mut sa: SlabAllocator = SlabAllocator{
        size: 10,
        pager: &mmap,
        allocateable: DList { head: None, head_elements: 0 },
    };

    sa.allocate(4096-CACHE_LINE_SIZE+1);
}

#[test]
#[should_panic]
fn invalid_alignment() {
    let mmap = MmapSlabAllocator;
    let mut sa: SlabAllocator = SlabAllocator{
        size: 10,
        pager: &mmap,
        allocateable: DList { head: None, head_elements: 0 },
    };

    sa.allocate(3);
}
