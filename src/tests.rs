use super::{ZoneAllocator, SlabPage, SlabPageMeta, SlabAllocator};

#[cfg(target_arch="x86_64")]
use x86::paging::{VAddr, CACHE_LINE_SIZE, BASE_PAGE_SIZE};

#[test]
fn type_size() {
    use std::mem::size_of;
    assert!(CACHE_LINE_SIZE as usize == size_of::<SlabPageMeta>(),
               "Meta-data within page should not be larger than a single cache-line.");
    assert!(BASE_PAGE_SIZE as usize == size_of::<SlabPage>(),
               "SlabPage should be exactly the size of a single page.");
}