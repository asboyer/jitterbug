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
    use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};

    fn ns_window(window: &tauri::WebviewWindow) -> Option<&NSWindow> {
        let pointer = window.ns_window().ok()?;
        // SAFETY: Tauri returns this webview's live NSWindow; callers run on the main thread.
        unsafe { pointer.cast::<NSWindow>().as_ref() }
    }

    pub fn prepare(window: &tauri::WebviewWindow) {
        let Some(ns_window) = ns_window(window) else {
            return;
        };
        // Tauri's current NSWindow subclass has its own ivars and cannot safely be changed into
        // an NSPanel at runtime. Keep its concrete class and use supported NSWindow properties.
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        ns_window.setLevel(NSStatusWindowLevel);
    }

    pub fn show(window: &tauri::WebviewWindow, key: bool) -> tauri::Result<()> {
        let Some(ns_window) = ns_window(window) else {
            return window.show();
        };
        if key {
            ns_window.makeKeyAndOrderFront(None);
        } else {
            ns_window.orderFrontRegardless();
        }
        Ok(())
    }
}
