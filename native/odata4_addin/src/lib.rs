//! Внешняя компонента Native API `OData4Native` (крейт `addin1c`) — тонкая обёртка над `odata4_core`.
//! Весь `unsafe` проекта — здесь, в точках входа из 1С; каждая точка входа ловит панику (`catch_unwind`),
//! паника не выходит за границу FFI.

mod bodies;
mod component;

use std::ffi::{c_int, c_long, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

use addin1c::{create_component, destroy_component, name, AttachType};

pub use component::{csdl_request, parse_request, OData4Native};

/// # Safety
///
/// `component` — указатель, который передаёт платформа.
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn GetClassObject(_name: *const u16, component: *mut *mut c_void) -> c_long {
    catch_unwind(AssertUnwindSafe(|| unsafe { create_component(component, OData4Native::new()) })).unwrap_or(0)
}

/// # Safety
///
/// `component` получен из `GetClassObject`, вызывается один раз.
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn DestroyObject(component: *mut *mut c_void) -> c_long {
    catch_unwind(AssertUnwindSafe(|| unsafe { destroy_component(component) })).unwrap_or(0)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn GetClassNames() -> *const u16 {
    catch_unwind(|| name!("OData4Native").as_ptr()).unwrap_or(std::ptr::null())
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn SetPlatformCapabilities(_capabilities: c_int) -> c_int {
    catch_unwind(|| 3).unwrap_or(0)
}

#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn GetAttachType() -> AttachType {
    catch_unwind(|| AttachType::Any).unwrap_or(AttachType::Any)
}
