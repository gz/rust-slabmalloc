// configuration options for the aarch64 architecture

#[cfg(all(feature = "cacheline_32", feature = "cacheline_64"))]
compile_error!(
    "feature \"cacheline_32\" and feature \"cacheline_64\" cannot be enabled at the same time"
);

#[cfg(all(feature = "cacheline_32", feature = "cacheline_128"))]
compile_error!(
    "feature \"cacheline_32\" and feature \"cacheline_128\" cannot be enabled at the same time"
);

#[cfg(all(feature = "cacheline_64", feature = "cacheline_128"))]
compile_error!(
    "feature \"cacheline_64\" and feature \"cacheline_128\" cannot be enabled at the same time"
);

#[cfg(not(any(
    feature = "cacheline_32",
    feature = "cacheline_64",
    feature = "cacheline_128"
)))]
compile_error!("need to select a cacheline size using features.");

#[cfg(feature = "cacheline_32")]
pub const CACHE_LINE_SIZE: usize = 32;

#[cfg(feature = "cacheline_64")]
pub const CACHE_LINE_SIZE: usize = 64;

#[cfg(feature = "cacheline_128")]
pub const CACHE_LINE_SIZE: usize = 128;

#[cfg(all(feature = "aarch64_granule_4k", feature = "aarch64_granule_16k"))]
compile_error!(
    "feature \"aarch64_granule_4k\" and feature \"aarch64_granule_16k\" cannot be enabled at the same time"
);

#[cfg(all(feature = "aarch64_granule_4k", feature = "aarch64_granule_64k"))]
compile_error!(
    "feature \"aarch64_granule_4k\" and feature \"aarch64_granule_64k\" cannot be enabled at the same time"
);

#[cfg(all(feature = "aarch64_granule_16k", feature = "aarch64_granule_64k"))]
compile_error!(
    "feature \"aarch64_granule_16k\" and feature \"aarch64_granule_64k\" cannot be enabled at the same time"
);

#[cfg(not(any(
    feature = "aarch64_granule_4k",
    feature = "aarch64_granule_16k",
    feature = "aarch64_granule_64k"
)))]
compile_error!("need to select a granule size using features.");

#[cfg(feature = "aarch64_granule_4k")]
pub const BASE_PAGE_SIZE: usize = 4 * 1024;

#[cfg(feature = "aarch64_granule_4k")]
#[allow(unused)]
pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

#[cfg(feature = "aarch64_granule_16k")]
pub const BASE_PAGE_SIZE: usize = 16 * 1024;

#[cfg(feature = "aarch64_granule_16k")]
#[allow(unused)]
pub const LARGE_PAGE_SIZE: usize = 32 * 1024 * 1024;

#[cfg(feature = "aarch64_granule_64k")]
pub const BASE_PAGE_SIZE: usize = 64 * 1024;

#[cfg(feature = "aarch64_granule_64k")]
#[allow(unused)]
pub const LARGE_PAGE_SIZE: usize = 512 * 1024 * 1024;

pub type VAddr = usize;
