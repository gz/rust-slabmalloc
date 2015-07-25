# slabmalloc [![Build Status](https://travis-ci.org/gz/rust-slabmalloc.svg)](https://travis-ci.org/gz/rust-slabmalloc) [![Crates.io](https://img.shields.io/crates/v/slabmalloc.svg)](https://crates.io/crates/slabmalloc)

Simple slab based malloc implementation in rust, in order to provide the
necessary interface to rusts liballoc library. slabmalloc only relies on libcore
and is designed to be used in kernel level code as the only interface a client
needs to provide is the  necessary mechanism to allocate and free 4KiB frames
(or any other default page-size on non-x86 hardware).

## Usage

* Use the ZoneAllocator to allocate arbitrary sized objects:
```rust
let mut mmap = MmapSlabAllocator::new();
let mut zone: ZoneAllocator = ZoneAllocator::new(&mmap);
let object_size = 12;
let alignment = 4;

match zone.allocate(object_size, alignment) {
    None => println!("Out of memory..."),
    Some(ptr) => zone.deallocate(ptr, object_size, alignment),
}
```

* Use the SlabAllocator to allocate fixed sized objects:
```rust
let object_size = 10;
let alignment = 8;
let mmap = MmapSlabAllocator::new();
let mut sa: SlabAllocator = SlabAllocator::new(object_size, &mmap);
sa.allocate(alignment);
```

The slabmalloc API is designed satisfy the rust liballoc low-level memory allocation interface. 

## Documentation
* [API Documentation](http://gz.github.io/rust-slabmalloc/slabmalloc/)