//! Low-Level WebAssembly ABI & Memory Management Primitives
//!
//! Provides deterministic string and byte packing, pointer conversion,
//! and buffer lifetime protection across the WASM guest/host boundary.

#[cfg(not(target_arch = "wasm32"))]
mod native_mock {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NATIVE_ALLOCS: Mutex<Option<HashMap<u32, Vec<u8>>>> = Mutex::new(None);
    static NEXT_ID: AtomicU32 = AtomicU32::new(1);

    pub fn store(bytes: Vec<u8>) -> u32 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut guard = NATIVE_ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(HashMap::new).insert(id, bytes);
        id
    }

    pub fn fetch_and_remove(id: u32) -> Option<Vec<u8>> {
        let mut guard = NATIVE_ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_mut().and_then(|m| m.remove(&id))
    }
}

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
        unsafe { drop(Vec::from_raw_parts(ptr, size, size)) };
    }
}

/// Packs a String into a 64-bit integer encoding: `(ptr << 32) | (len & 0xFFFF_FFFF)`.
pub fn pack_string(s: String) -> u64 {
    pack_bytes(s.into_bytes())
}

/// Packs a byte buffer into a 64-bit integer encoding: `(ptr << 32) | (len & 0xFFFF_FFFF)`.
#[cfg(target_arch = "wasm32")]
pub fn pack_bytes(bytes: Vec<u8>) -> u64 {
    let mut bytes = bytes;
    bytes.shrink_to_fit();
    let len = bytes.len() as u64;
    let ptr = bytes.as_mut_ptr() as usize as u64;
    std::mem::forget(bytes);
    ((ptr & 0xFFFF_FFFF) << 32) | (len & 0xFFFF_FFFF)
}

/// Packs a byte buffer into a 64-bit integer encoding for native unit test mocking.
#[cfg(not(target_arch = "wasm32"))]
pub fn pack_bytes(bytes: Vec<u8>) -> u64 {
    let len = bytes.len() as u64;
    let id = native_mock::store(bytes) as u64;
    (id << 32) | (len & 0xFFFF_FFFF)
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
        let s = String::from_utf8_lossy(slice).to_string();
        deallocate(ptr as *mut u8, len);
        s
    }
}

/// On native 64-bit platforms (during unit tests), resolves safely from `NATIVE_ALLOCS`.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_guest_string(packed: u64) -> String {
    let id = (packed >> 32) as u32;
    if let Some(bytes) = native_mock::fetch_and_remove(id) {
        String::from_utf8_lossy(&bytes).to_string()
    } else {
        String::new()
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
