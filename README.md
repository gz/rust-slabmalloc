# slabmalloc

Simple slab based malloc implementation in rust, in order to provide the
necessary interface to rusts liballoc library. slabmalloc only relies on libcore
and is designed to be used in kernel level code as the only interface a client
needs to provide is the  necessary mechanism to allocate and free 4KiB frames
(or any other default page-size on non-x86 hardware).

## Usage
TBD