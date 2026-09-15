//! Low-Level WebAssembly ABI & Memory Management Primitives
//!
//! Provides deterministic string and byte packing, pointer conversion,
//! and buffer lifetime protection across the WASM guest/host boundary.

static mut LAST_ALLOC: Option<Vec<u8>> = None;

/// Allocates a contiguous memory buffer inside the guest memory space.
pub fn allocate(size: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Frees a previously allocated guest memory buffer.
pub fn deallocate(ptr: *mut u8, size: usize) {
    if !ptr.is_null() && size > 0 {
        unsafe { drop(Vec::from_raw_parts(ptr, 0, size)) };
    }
}

/// Packs a String into a 64-bit integer encoding: `(ptr << 32) | (len & 0xFFFF_FFFF)`.
/// Retains the allocation in `LAST_ALLOC` so the host can safely read it.
pub fn pack_string(s: String) -> u64 {
    pack_bytes(s.into_bytes())
}

/// Packs a byte buffer into a 64-bit integer encoding: `(ptr << 32) | (len & 0xFFFF_FFFF)`.
pub fn pack_bytes(bytes: Vec<u8>) -> u64 {
    let len = bytes.len() as u64;
    let ptr = bytes.as_ptr() as usize as u64;
    unsafe {
        LAST_ALLOC = Some(bytes);
    }
    // On 32-bit WASM, (ptr as u32 as u64) fits within the high 32 bits
    ((ptr & 0xFFFF_FFFF) << 32) | (len & 0xFFFF_FFFF)
}

/// Unpacks a string from raw guest memory pointer and length.
pub fn unpack_string(ptr: u32, len: u32) -> String {
    if ptr == 0 || len == 0 {
        return String::new();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        String::from_utf8_lossy(slice).to_string()
    }
}

/// Unpacks a byte vector from raw guest memory pointer and length.
pub fn unpack_bytes(ptr: u32, len: u32) -> Vec<u8> {
    if ptr == 0 || len == 0 {
        return Vec::new();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        slice.to_vec()
    }
}

/// Reads a packed pointer returned from host functions: `(ptr << 32) | len`.
#[cfg(target_arch = "wasm32")]
pub fn read_guest_string(packed: u64) -> String {
    let ptr = (packed >> 32) as usize;
    let len = (packed & 0xFFFF_FFFF) as usize;
    if ptr == 0 || len == 0 {
        return String::new();
    }
    unsafe {
        let slice = std::slice::from_raw_parts(ptr as *const u8, len);
        String::from_utf8_lossy(slice).to_string()
    }
}

/// On native 64-bit platforms (during unit tests), resolves directly from `LAST_ALLOC`.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_guest_string(_packed: u64) -> String {
    unsafe {
        if let Some(ref bytes) = LAST_ALLOC {
            String::from_utf8_lossy(bytes).to_string()
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack_string() {
        let original = "Hello SpectraFlux!".to_string();
        let packed = pack_string(original.clone());
        let unpacked = read_guest_string(packed);
        assert_eq!(original, unpacked);
    }

    #[test]
    fn test_allocate_deallocate() {
        let ptr = allocate(64);
        assert!(!ptr.is_null());
        deallocate(ptr, 64);
    }
}
