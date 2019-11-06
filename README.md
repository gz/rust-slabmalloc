# slabmalloc [![Build Status](https://travis-ci.org/gz/rust-slabmalloc.svg)](https://travis-ci.org/gz/rust-slabmalloc) [![Crates.io](https://img.shields.io/crates/v/slabmalloc.svg)](https://crates.io/crates/slabmalloc)

Simple slab based malloc implementation in rust, in order to provide the
necessary interface to rusts liballoc library. slabmalloc only relies on
libcore and is designed to be used in kernel level code as the only interface a
client needs to provide is the necessary mechanism to allocate and free 4KiB
frames (or any other default page-size on non-x86 hardware).


## Build

By default this library should compile with nightly and stable versions of the
Rust compiler.

Add the following line to the Cargo.toml dependencies:
```
slabmalloc = <version>
```

## Usage

The slabmalloc API is designed to satisfy the rust liballoc low-level memory
allocation interface:

```rust
#[global_allocator]
static MEM_PROVIDER: slabmalloc::SafeZoneAllocator = slabmalloc::SafeZoneAllocator::new();
```

* Use the ZoneAllocator to allocate arbitrary sized objects:
```rust
let object_size = 12;
let alignment = 4;
let layout = Layout::from_size_align(object_size, alignment).unwrap();

// We need something that can provide backing memory
// (4 KiB and 2 MiB pages) to our ZoneAllocator
// (see tests.rs for a dummy implementation).
let mut pager = Pager::new();
let page = pager.allocate_page().expect("Can't allocate a page");

let mut zone: ZoneAllocator = Default::default();
// Prematurely fill the ZoneAllocator with memory.
// Alternatively, the allocate call would return an
// error which we can capture to refill on-demand.
unsafe { zone.refill(layout, page)? };

let allocated = zone.allocate(layout)?;
zone.deallocate(allocated, layout)?;
```

* Use the SCAllocator to allocate fixed sized objects:
```rust
let object_size = 10;
let alignment = 8;
let layout = Layout::from_size_align(object_size, alignment).unwrap();

// We need something that can provide backing memory
// (4 KiB and 2 MiB pages) to our ZoneAllocator
// (see tests.rs for a dummy implementation).
let mut pager = Pager::new();
let page = pager.allocate_page().expect("Can't allocate a page");

let mut sa: SCAllocator<ObjectPage> = SCAllocator::new(object_size);
// Prematurely fill the SCAllocator with memory.
// Alternatively, the allocate call would return an
// error which we can capture to refill on-demand.
unsafe { sa.refill(page) };

sa.allocate(layout)?;
```

## Performance

slabmalloc is optimized for single-threaded, fixed-size object allocations. For
anything else it will probably perform poorly (for example if your workload
does lots of reallocations, or if the allocator needs to scale to many cores).

At least on my system, it outperforms jemalloc in (silly) benchmarks:
```
test tests::jemalloc_allocate_deallocate       ... bench:          76 ns/iter (+/- 5)
test tests::jemalloc_allocate_deallocate_big   ... bench:         119 ns/iter (+/- 24)
test tests::slabmalloc_allocate_deallocate     ... bench:          38 ns/iter (+/- 8)
test tests::slabmalloc_allocate_deallocate_big ... bench:          38 ns/iter (+/- 11)
```

## Documentation
* [API Documentation](https://docs.rs/slabmalloc)
