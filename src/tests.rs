use std::prelude::v1::*;
use std::mem::{transmute};
use libc;
use rand;
use std::mem::size_of;

// The types we want to test:
use super::{SlabPage, SlabAllocator, SlabPageAllocator};

#[cfg(target_arch="x86_64")]
use x86::paging::{BASE_PAGE_SIZE};

use test::Bencher;

/// Page allocator based on mmap/munmap system calls for backing slab memory.
struct MmapSlabAllocator;

impl MmapSlabAllocator {
    pub fn new() -> MmapSlabAllocator {
        MmapSlabAllocator
    }
}

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

    fn release_slabpage(&self, p: &'a mut SlabPage<'a>) {
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
    let mmap = MmapSlabAllocator::new();
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
            let mut mmap = MmapSlabAllocator::new();

            {
                let mut sa: SlabAllocator = SlabAllocator::new($size, &mut mmap);
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
                    for i in 0..obj.len() {
                        assert!( (obj[i]) == pattern, "No two allocations point to the same memory.");
                    }
                }

                // Make sure we can correctly deallocate:
                let pages_allocated = sa.slabs.elements;

                // Deallocate all the objects
                for item in objects.iter_mut() {
                    sa.deallocate(*item);
                }

                objects.clear();

                // then allocate everything again,
                for idx in 0..$allocations {
                    match sa.allocate(alignment) {
                        None => panic!("OOM is unlikely."),
                        Some(obj) => {
                            unsafe { vec.push( (rand::random::<usize>(), transmute(obj)) ) };
                            objects.push(obj)
                        }
                    }
                }

                // and make sure we do not request more pages than what we had previously
                // println!("{} {}", pages_allocated, sa.slabs.elements);
                assert!(pages_allocated == sa.slabs.elements,
                        "Did not use more memory for 2nd allocation run.");

                // Deallocate everything once more
                for item in objects.iter_mut() {
                    sa.deallocate(*item);
                }
            }

            // Check that we released everything to our page allocator:
            // assert!(mmap.currently_allocated() == 0, "Released all pages to underlying memory manager.");
        }

    };
}

macro_rules! test_slab_allocation_panic {
    ( $test:ident, $size:expr, $alignment:expr, $allocations:expr  ) => {
        test_slab_allocation!($test, $size, $alignment, $allocations);
    };
}

test_slab_allocation!(test_slab_allocation8192_size8_alignment1, 8, 1, 512);
test_slab_allocation!(test_slab_allocation4096_size8_alignment8, 8, 8, 4096);
test_slab_allocation!(test_slab_allocation500_size8_alignment64, 8, 64, 500);
test_slab_allocation!(test_slab_allocation4096_size12_alignment1, 12, 1, 4096);
test_slab_allocation!(test_slab_allocation4096_size13_alignment1, 13, 1, 4096);
test_slab_allocation!(test_slab_allocation2000_size14_alignment1, 14, 1, 2000);
test_slab_allocation!(test_slab_allocation4096_size15_alignment1, 15, 1, 4096);
test_slab_allocation!(test_slab_allocation8000_size16_alignment1, 16, 1, 8000);
test_slab_allocation!(test_slab_allocation1024_size24_alignment1, 24, 1, 1024);
test_slab_allocation!(test_slab_allocation3090_size32_alignment1, 32, 1, 3090);
test_slab_allocation!(test_slab_allocation4096_size64_alignment1, 64, 1, 4096);
test_slab_allocation!(test_slab_allocation1000_size512_alignment1, 512, 1, 1000);
test_slab_allocation!(test_slab_allocation4096_size1024_alignment1, 1024, 1, 4096);
test_slab_allocation!(test_slab_allocation10_size2048_alignment1, 2048, 1, 10);


#[test]
#[should_panic]
fn invalid_alignment() {
    let mmap = MmapSlabAllocator::new();
    let mut sa: SlabAllocator = SlabAllocator::new(10, &mmap);

    sa.allocate(3);
}

#[bench]
fn bench_allocate(b: &mut Bencher) {
    let mmap = MmapSlabAllocator::new();
    let mut sa: SlabAllocator = SlabAllocator::new(8, &mmap);

    b.iter(|| sa.allocate(4));
}


#[bench]
fn bench_allocate_big(b: &mut Bencher) {
    let mmap = MmapSlabAllocator::new();
    let mut sa: SlabAllocator = SlabAllocator::new(512, &mmap);

    b.iter(|| sa.allocate(1));
}