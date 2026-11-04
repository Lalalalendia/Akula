use std::sync::OnceLock;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{POINTER_INPUT_TYPE, PT_TOUCHPAD};
use windows::core::{w, PCSTR};

// Optional on Windows 7; never import USER32!GetPointerType at load time.
pub(super) fn is_touchpad(pointer: u32) -> bool {
    type GetPointerTypeFn = unsafe extern "system" fn(u32, *mut POINTER_INPUT_TYPE) -> i32;
    static FN: OnceLock<Option<GetPointerTypeFn>> = OnceLock::new();
    let get_pointer_type = FN.get_or_init(|| unsafe {
        let module = GetModuleHandleW(w!("user32.dll")).ok()?;
        let address = GetProcAddress(module, PCSTR(c"GetPointerType".as_ptr().cast()))?;
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, GetPointerTypeFn>(address))
    });
    let mut kind = POINTER_INPUT_TYPE::default();
    get_pointer_type.is_some_and(|f| unsafe { f(pointer, &mut kind) != 0 }) && kind == PT_TOUCHPAD
}
