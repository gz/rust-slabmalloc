use env_logger;
use libc;
use rand;
use std::alloc;
use std::alloc::Layout;
use std::mem::{size_of, transmute};
use std::prelude::v1::*;

use super::*;
use test::Bencher;

/// Page allocator based on mmap/munmap system calls for backing slab memory.
struct MmapPageProvider {
    currently_allocated: usize,
}

impl MmapPageProvider {
    pub fn new() -> MmapPageProvider {
        MmapPageProvider {
            currently_allocated: 0,
        }
    }
}

impl MmapPageProvider {
    pub fn currently_allocated(&self) -> usize {
        self.currently_allocated
    }
}

trait PageProvider<'a>: Send {
    fn allocate_page(&mut self) -> Option<&'a mut ObjectPage<'a>>;
    fn release_page(&mut self, page: &'a mut ObjectPage<'a>);
}

impl<'a> PageProvider<'a> for MmapPageProvider {
    /// Allocates a new ObjectPage from the system.
    ///
    /// Uses `mmap` to map a page and casts it to a ObjectPage.
    fn allocate_page(&mut self) -> Option<&'a mut ObjectPage<'a>> {
        let mut addr: libc::c_void = libc::c_void::__variant1;
        let len: libc::size_t = BASE_PAGE_SIZE;
        let prot = libc::PROT_READ | libc::PROT_WRITE;
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
        let fd = -1;
        let offset = 0;
        let r = unsafe { libc::mmap(&mut addr, len as libc::size_t, prot, flags, fd, offset) };
        if r == libc::MAP_FAILED {
            return None;
        } else {
            let slab_page: &'a mut ObjectPage = unsafe { transmute(r as usize) };
            self.currently_allocated += 1;
            return Some(slab_page);
        }
    }

    /// Release a ObjectPage back to the system.slab_page
    ///
    /// Uses `munmap` to release the page back to the OS.
    fn release_page(&mut self, p: &'a mut ObjectPage<'a>) {
        let addr: *mut libc::c_void = unsafe { transmute(p) };
        let len: libc::size_t = BASE_PAGE_SIZE;
        let r = unsafe { libc::munmap(addr, len) };
        if r != 0 {
            panic!("munmap failed!");
        }
        self.currently_allocated -= 1;
    }
}

#[test]
fn check_size() {
    assert_eq!(
        BASE_PAGE_SIZE as usize,
        size_of::<ObjectPage>(),
        "ObjectPage should be exactly the size of a single page."
    );

    /*assert_eq!(
        LARGE_PAGE_SIZE as usize,
        size_of::<LargeObjectPage>(),
        "LargeObjectPage should be exactly the size of a large-page."
    );*/
}

#[test]
fn test_mmap_allocator() {
    let mut mmap = MmapPageProvider::new();

    match mmap.allocate_page() {
        Some(sp) => {
            sp.bitfield.initialize(8, BASE_PAGE_SIZE - 64);
            assert!(!sp.is_full(), "Got empty slab");
            assert!(sp.is_empty(6 * 64), "Got empty slab");
            mmap.release_page(sp)
        }
        None => panic!("failed to allocate ObjectPage"),
    }
}

#[test]
fn check_sizes() {
    assert_eq!(size_of::<ObjectPage>(), BASE_PAGE_SIZE);
}

macro_rules! test_sc_allocation {
    ($test:ident, $size:expr, $alignment:expr, $allocations:expr) => {
        #[test]
        fn $test() {
            let _ = env_logger::try_init();
            let mut mmap = MmapPageProvider::new();
            {
                let mut sa: SCAllocator = SCAllocator::new($size);
                let alignment = $alignment;

                let mut objects: Vec<NonNull<u8>> = Vec::new();
                let mut vec: Vec<(usize, &mut [usize; $size / 8])> = Vec::new();
                let layout = Layout::from_size_align($size, alignment).unwrap();

                for _ in 0..$allocations {
                    loop {
                        match sa.allocate(layout) {
                            // Allocation was successful
                            Ok(nptr) => {
                                unsafe {
                                    vec.push((rand::random::<usize>(), transmute(nptr.as_ptr())))
                                };
                                objects.push(nptr);
                                break;
                            }
                            // Couldn't allocate need to refill first
                            Err(AllocationError::OutOfMemory(_)) => {
                                let page = mmap.allocate_page().unwrap();
                                unsafe {
                                    sa.refill(page);
                                }
                            }
                            // Unexpected errors
                            Err(AllocationError::InvalidLayout) => unreachable!("Unexpected error"),
                        }
                    }
                }

                // Write the objects with a random pattern
                for item in vec.iter_mut() {
                    let (pattern, ref mut obj) = *item;
                    assert!(obj.len() == $size / 8);
                    for i in 0..obj.len() {
                        obj[i] = pattern;
                    }
                }

                for item in vec.iter() {
                    let (pattern, ref obj) = *item;
                    for i in 0..obj.len() {
                        assert_eq!(
                            obj[i], pattern,
                            "No two allocations point to the same memory."
                        );
                    }
                }

                // Make sure we can correctly deallocate:
                let pages_allocated = sa.slabs.elements;

                // Deallocate all the objects
                for item in objects.iter_mut() {
                    sa.deallocate(*item, layout).expect("Can't deallocate");
                }

                objects.clear();

                // then allocate everything again,
                for _ in 0..$allocations {
                    loop {
                        match sa.allocate(layout) {
                            // Allocation was successful
                            Ok(nptr) => {
                                unsafe {
                                    vec.push((rand::random::<usize>(), transmute(nptr.as_ptr())))
                                };
                                objects.push(nptr);
                                break;
                            }
                            // Couldn't allocate need to refill first
                            Err(AllocationError::OutOfMemory(_)) => {
                                let page = mmap.allocate_page().unwrap();
                                unsafe {
                                    sa.refill(page);
                                }
                            }
                            // Unexpected errors
                            Err(AllocationError::InvalidLayout) => unreachable!("Unexpected error"),
                        }
                    }
                }

                // and make sure we do not request more pages than what we had previously
                // println!("{} {}", pages_allocated, sa.slabs.elements);
                assert_eq!(
                    pages_allocated, sa.slabs.elements,
                    "Did not use more memory for 2nd allocation run."
                );

                // Deallocate everything once more
                for item in objects.iter_mut() {
                    sa.deallocate(*item, layout).expect("Can't deallocate");
                }

                // Drain the slab-allocator and give unused pages back to the OS
                while let Some(page) = sa.empty_slabs.pop() {
                    mmap.release_page(page);
                }
            }

            // Check that we released everything to our page allocator:
            assert_eq!(
                mmap.currently_allocated(),
                0,
                "Released all pages to the underlying memory manager."
            );
        }
    };
}

test_sc_allocation!(test_sc_allocation512_size8_alignment1, 8, 1, 512);
test_sc_allocation!(test_sc_allocation4096_size8_alignment8, 8, 8, 4096);
test_sc_allocation!(test_sc_allocation500_size8_alignment64, 8, 64, 500);
test_sc_allocation!(test_sc_allocation4096_size12_alignment1, 12, 1, 4096);
test_sc_allocation!(test_sc_allocation4096_size13_alignment1, 13, 1, 4096);
test_sc_allocation!(test_sc_allocation2000_size14_alignment1, 14, 1, 2000);
test_sc_allocation!(test_sc_allocation4096_size15_alignment1, 15, 1, 4096);
test_sc_allocation!(test_sc_allocation8000_size16_alignment1, 16, 1, 8000);
test_sc_allocation!(test_sc_allocation1024_size24_alignment1, 24, 1, 1024);
test_sc_allocation!(test_sc_allocation3090_size32_alignment1, 32, 1, 3090);
test_sc_allocation!(test_sc_allocation4096_size64_alignment1, 64, 1, 4096);
test_sc_allocation!(test_sc_allocation1000_size512_alignment1, 512, 1, 1000);
test_sc_allocation!(test_sc_allocation4096_size1024_alignment1, 1024, 1, 4096);
test_sc_allocation!(test_sc_allocation10_size2048_alignment1, 2048, 1, 10);
test_sc_allocation!(test_sc_allocation10000_size512_alignment1, 512, 1, 10000);

#[test]
#[should_panic]
fn invalid_alignment() {
    let _layout = Layout::from_size_align(10, 3).unwrap();
}

#[test]
fn test_readme() -> Result<(), AllocationError> {
    let object_size = 12;
    let alignment = 4;
    let layout = Layout::from_size_align(object_size, alignment).unwrap();

    let mut mmap = MmapPageProvider::new();
    let page = mmap.allocate_page().expect("Can't allocate a page");

    let mut zone: ZoneAllocator = Default::default();
    unsafe { zone.refill(layout, page)? };

    let allocated = zone.allocate(layout)?;
    zone.deallocate(allocated, layout)?;

    Ok(())
}

#[test]
fn test_readme2() -> Result<(), AllocationError> {
    let object_size = 10;
    let alignment = 8;
    let layout = Layout::from_size_align(object_size, alignment).unwrap();

    let mut mmap = MmapPageProvider::new();
    let page = mmap.allocate_page().expect("Can't allocate a page");

    let mut sa: SCAllocator = SCAllocator::new(object_size);
    unsafe { sa.refill(page) };

    sa.allocate(layout)?;
    Ok(())
}

#[test]
fn test_bug1() -> Result<(), AllocationError> {
    let _ = env_logger::try_init();

    let mut mmap = MmapPageProvider::new();
    let page = mmap.allocate_page();

    let mut sa: SCAllocator = SCAllocator::new(8);
    unsafe {
        sa.refill(page.unwrap());
    }

    let ptr1 = sa.allocate(Layout::from_size_align(1, 1).unwrap())?;
    let ptr2 = sa.allocate(Layout::from_size_align(2, 1).unwrap())?;
    sa.deallocate(ptr1, Layout::from_size_align(1, 1).unwrap())?;
    let _ptr3 = sa.allocate(Layout::from_size_align(4, 1).unwrap())?;
    sa.deallocate(ptr2, Layout::from_size_align(2, 1).unwrap())
}

#[bench]
fn slabmalloc_allocate_deallocate(b: &mut Bencher) {
    let _ = env_logger::try_init();

    let mut mmap = MmapPageProvider::new();
    let mut sa: SCAllocator = SCAllocator::new(8);
    let layout = Layout::from_size_align(8, 1).unwrap();

    let page = mmap.allocate_page();
    unsafe {
        sa.refill(page.unwrap());
    }

    let ptr = sa.allocate(layout).expect("Can't allocate");
    test::black_box(ptr);
    b.iter(|| {
        let ptr = sa.allocate(layout).expect("Can't allocate");
        test::black_box(ptr);
        sa.deallocate(ptr, layout).expect("Can't deallocate");
    });
}

#[bench]
fn slabmalloc_allocate_deallocate_big(b: &mut Bencher) {
    let _ = env_logger::try_init();

    let mut mmap = MmapPageProvider::new();
    let mut sa: SCAllocator = SCAllocator::new(512);

    let page = mmap.allocate_page();
    unsafe {
        sa.refill(page.unwrap());
    }

    let layout = Layout::from_size_align(512, 1).unwrap();
    let ptr = sa.allocate(layout).expect("Can't allocate");
    test::black_box(ptr);

    b.iter(|| {
        let ptr = sa.allocate(layout).expect("Can't allocate");
        test::black_box(ptr);
        sa.deallocate(ptr, layout).expect("Can't deallocate");
    });
}

#[bench]
fn jemalloc_allocate_deallocate(b: &mut Bencher) {
    let layout = Layout::from_size_align(8, 1).unwrap();
    let ptr = unsafe { alloc::alloc(layout) };
    test::black_box(ptr);

    b.iter(|| unsafe {
        let ptr = alloc::alloc(layout);
        test::black_box(ptr);
        alloc::dealloc(ptr, layout);
    });
}

#[bench]
fn jemalloc_allocate_deallocate_big(b: &mut Bencher) {
    let layout = Layout::from_size_align(512, 1).unwrap();
    let ptr = unsafe { alloc::alloc(layout) };
    test::black_box(ptr);

    b.iter(|| unsafe {
        let ptr = alloc::alloc(layout);
        test::black_box(ptr);
        alloc::dealloc(ptr, layout);
    });
}

#[test]
pub fn check_first_fit() {
    let op: ObjectPage = Default::default();
    let layout = Layout::from_size_align(8, 8).unwrap();
    println!("{:?}", op.first_fit(layout));
}

#[test]
fn list_pop() {
    let mut op1: ObjectPage = Default::default();
    let op1_ptr = &op1 as *const ObjectPage<'_>;
    let mut op2: ObjectPage = Default::default();
    let op2_ptr = &op2 as *const ObjectPage<'_>;
    let mut op3: ObjectPage = Default::default();
    let op3_ptr = &op3 as *const ObjectPage<'_>;
    let mut op4: ObjectPage = Default::default();
    let op4_ptr = &op4 as *const ObjectPage<'_>;

    let mut list: ObjectPageList = ObjectPageList::new();
    list.insert_front(&mut op1);
    list.insert_front(&mut op2);
    list.insert_front(&mut op3);

    assert!(list.contains(op1_ptr));
    assert!(list.contains(op2_ptr));
    assert!(list.contains(op3_ptr));
    assert!(!list.contains(op4_ptr));

    let popped = list.pop();
    assert_eq!(popped.unwrap() as *const ObjectPage, op3_ptr);
    assert!(!list.contains(op3_ptr));

    let popped = list.pop();
    assert_eq!(popped.unwrap() as *const ObjectPage, op2_ptr);
    assert!(!list.contains(op2_ptr));

    list.insert_front(&mut op4);
    assert!(list.contains(op4_ptr));
    let popped = list.pop();
    assert_eq!(popped.unwrap() as *const ObjectPage, op4_ptr);
    assert!(!list.contains(op4_ptr));

    let popped = list.pop();
    assert_eq!(popped.unwrap() as *const ObjectPage, op1_ptr);
    assert!(!list.contains(op1_ptr));

    let popped = list.pop();
    assert!(popped.is_none());

    assert!(!list.contains(op1_ptr));
    assert!(!list.contains(op2_ptr));
    assert!(!list.contains(op3_ptr));
    assert!(!list.contains(op4_ptr));
}

#[test]
pub fn iter_empty_list() {
    let mut new_head1: ObjectPage = Default::default();
    let mut l = ObjectPageList::new();
    l.insert_front(&mut new_head1);
    for _p in l.iter_mut() {}
}

#[test]
pub fn check_is_full_8() {
    let _r = env_logger::try_init();
    let layout = Layout::from_size_align(8, 1).unwrap();

    let mut page: ObjectPage = Default::default();
    page.bitfield.initialize(8, BASE_PAGE_SIZE - 64);
    let obj_per_page = core::cmp::min((BASE_PAGE_SIZE - 64) / 8, 6 * 64);

    let mut allocs = 0;
    loop {
        if page.allocate(layout).is_null() {
            break;
        }
        allocs += 1;

        if allocs < (6 * 64) {
            assert!(!page.is_full());
            assert!(!page.is_empty(obj_per_page));
        }
    }

    assert_eq!(allocs, 6 * 64, "Can use all bitmap space");
    assert!(page.is_full());
}

// Test for bug that reports pages not as full when
// the entire bitfield wasn't allocated.
#[test]
pub fn check_is_full_512() {
    let _r = env_logger::try_init();
    let mut page: ObjectPage = Default::default();
    page.bitfield.initialize(512, BASE_PAGE_SIZE - 64);
    let layout = Layout::from_size_align(512, 1).unwrap();
    let obj_per_page = core::cmp::min((BASE_PAGE_SIZE - 64) / 512, 6 * 64);

    let mut allocs = 0;
    loop {
        if page.allocate(layout).is_null() {
            break;
        }

        allocs += 1;

        if allocs < (BASE_PAGE_SIZE - 64) / 512 {
            assert!(!page.is_full());
            assert!(!page.is_empty(obj_per_page));
        }
    }
    assert!(page.is_full());
}
