// configuration options for the x86 architecture

#[cfg(feature = "cacheline_32")]
compile_error!("enabling 32-byte cacheline size on x86 is not supported.");

#[cfg(feature = "cacheline_128")]
compile_error!("enabling 128-byte cacheline size on x86 is not supported.");

#[cfg(all(target_arch = "x86_64", feature = "largepage_4M"))]
compile_error!("enabling 4 MiB large page size on x86 is not supported.");

#[cfg(target_arch = "x86_64")]
pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

#[cfg(target_arch = "x86_32")]
pub const LARGE_PAGE_SIZE: usize = 4 * 1024 * 1024;

pub const CACHE_LINE_SIZE: usize = 64;

pub const BASE_PAGE_SIZE: usize = 4 * 1024;

pub type VAddr = usize;
