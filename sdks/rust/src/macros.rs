//! WASM Entrypoint Export Macro
//!
//! Exposes `export_fluxcell!(MyCell)`, generating all necessary
//! low-level C-ABI entrypoints and memory management functions.

#[macro_export]
macro_rules! export_fluxcell {
    ($cell:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn allocate(size: usize) -> *mut u8 {
            $crate::abi::allocate(size)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn deallocate(ptr: *mut u8, size: usize) {
            $crate::abi::deallocate(ptr, size)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn get_metadata() -> u64 {
            let meta = <$cell as $crate::Fluxcell>::metadata();
            let json = serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string());
            $crate::abi::pack_string(json)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn get_subscriptions() -> u64 {
            let subs = <$cell as $crate::Fluxcell>::subscriptions();
            let json = serde_json::to_string(&subs).unwrap_or_else(|_| "[]".to_string());
            $crate::abi::pack_string(json)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn get_routes() -> u64 {
            let r = <$cell as $crate::Fluxcell>::routes();
            let json = serde_json::to_string(&r).unwrap_or_else(|_| "[]".to_string());
            $crate::abi::pack_string(json)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn handle_http(ptr: u32, len: u32) -> u64 {
            let bytes = $crate::abi::unpack_bytes(ptr, len);
            let req = if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                $crate::http::HttpRequest::from_json_val(&val)
            } else {
                $crate::http::HttpRequest::default()
            };

            let resp = <$cell as $crate::Fluxcell>::handle_http(req);
            let json = serde_json::to_string(&resp).unwrap_or_default();
            $crate::abi::pack_string(json)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn handle_event(ptr: u32, len: u32) -> u64 {
            let bytes = $crate::abi::unpack_bytes(ptr, len);
            let event = $crate::event::EventContext::from_bytes(&bytes);
            let res = <$cell as $crate::Fluxcell>::handle_event(event);
            let json = serde_json::to_string(&res).unwrap_or_default();
            $crate::abi::pack_string(json)
        }
    };
}
