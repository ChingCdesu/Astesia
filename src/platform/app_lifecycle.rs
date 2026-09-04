#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod macos {
    use std::ffi::CString;

    use objc::declare::MethodImplementation as _;
    use objc::runtime::{class_addMethod, class_getInstanceMethod, Object, Sel, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};

    pub(crate) fn install_last_window_quit_policy() {
        unsafe {
            let application: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut Object = msg_send![application, delegate];
            assert!(
                !delegate.is_null(),
                "GPUI did not install an application delegate"
            );

            let delegate_class = (*delegate).class();
            let selector = sel!(applicationShouldTerminateAfterLastWindowClosed:);
            if !class_getInstanceMethod(delegate_class, selector).is_null() {
                return;
            }

            let implementation = should_terminate_after_last_window_closed
                as extern "C" fn(&Object, Sel, *mut Object) -> BOOL;
            let type_encoding =
                CString::new(format!("{}@:@", <BOOL as objc::Encode>::encode().as_str()))
                    .expect("valid Objective-C method encoding");

            // The pinned GPUI delegate omits this AppKit policy, so closing its last native
            // window can otherwise leave a windowless Astesia process running.
            let added = class_addMethod(
                delegate_class as *const _ as *mut _,
                selector,
                implementation.imp(),
                type_encoding.as_ptr(),
            );
            assert!(added != NO, "failed to install the macOS quit policy");
        }
    }

    extern "C" fn should_terminate_after_last_window_closed(
        _delegate: &Object,
        _selector: Sel,
        _application: *mut Object,
    ) -> BOOL {
        YES
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::install_last_window_quit_policy;

#[cfg(not(target_os = "macos"))]
pub(crate) fn install_last_window_quit_policy() {}
