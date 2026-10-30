//! Small windows that appear over whatever the user is doing: the quick view and the popup.
//! On macOS a regular window from an inactive app cannot join another app's full-screen
//! Space, and focusing it switches Spaces. Converting it to a non-activating panel fixes both.

/// Prepare a freshly built window. Call once, on the main thread.
pub fn prepare(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos::prepare(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Show without activating Jitterbug or switching Spaces. With `key`, the window takes
/// keyboard input (the quick view); without it, the user's app keeps typing focus (the popup).
pub fn show(window: &tauri::WebviewWindow, key: bool) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    return macos::show(window, key);
    #[cfg(not(target_os = "macos"))]
    {
        window.show()?;
        if key {
            window.set_focus()?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{define_class, runtime::AnyObject, runtime::NSObject, ClassType, MainThreadOnly};
    use objc2_app_kit::{
        NSPanel, NSResponder, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior,
        NSWindowStyleMask,
    };

    define_class!(
        // SAFETY: NSPanel adds no requirements for subclasses; only two getters are overridden.
        #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "JitterbugFloatingPanel"]
        struct FloatingPanel;

        impl FloatingPanel {
            // Borderless panels refuse key status by default; the quick view needs Escape.
            #[unsafe(method(canBecomeKeyWindow))]
            fn can_become_key_window(&self) -> bool {
                true
            }
            #[unsafe(method(canBecomeMainWindow))]
            fn can_become_main_window(&self) -> bool {
                false
            }
        }
    );

    fn ns_window(window: &tauri::WebviewWindow) -> Option<&NSWindow> {
        let pointer = window.ns_window().ok()?;
        // SAFETY: Tauri returns this webview's live NSWindow; callers run on the main thread.
        unsafe { pointer.cast::<NSWindow>().as_ref() }
    }

    pub fn prepare(window: &tauri::WebviewWindow) {
        let Some(ns_window) = ns_window(window) else {
            return;
        };
        if ns_window.class() != FloatingPanel::class() {
            // SAFETY: FloatingPanel is an NSPanel (itself an NSWindow subclass) with no ivars;
            // swapping the class of a live window is the established way to make Tauri's
            // window a panel. The window delegate and webview are untouched.
            unsafe {
                AnyObject::set_class(
                    &*(ns_window as *const NSWindow).cast::<AnyObject>(),
                    FloatingPanel::class(),
                );
            }
        }
        // SAFETY: the class swap above makes this window an NSPanel.
        let panel = unsafe { &*(ns_window as *const NSWindow).cast::<NSPanel>() };
        panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        panel.setFloatingPanel(true);
        // Panels hide when their app deactivates by default; Jitterbug is usually inactive.
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setLevel(NSStatusWindowLevel);
    }

    pub fn show(window: &tauri::WebviewWindow, key: bool) -> tauri::Result<()> {
        let Some(ns_window) = ns_window(window) else {
            return window.show();
        };
        if key {
            // A non-activating panel becomes key without activating Jitterbug.
            ns_window.makeKeyAndOrderFront(None);
        } else {
            ns_window.orderFrontRegardless();
        }
        Ok(())
    }
}
