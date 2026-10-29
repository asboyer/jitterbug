//! OS-owned application artwork. No network fetches, scripts, or arbitrary UI-supplied paths.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Serialize)]
pub struct Application {
    pub name: String,
    pub identifier: Option<String>,
    pub version: Option<String>,
    pub identity_source: String,
    #[serde(skip)]
    pub path: PathBuf,
}

pub fn identify(executable: &Path) -> Application {
    let bundle = crate::attribution::application_bundle(executable);
    let path = bundle.unwrap_or(executable).to_path_buf();
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(100)
        .collect();
    #[cfg(target_os = "macos")]
    let (identifier, version) = bundle.map(macos::metadata).unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    let (identifier, version): (Option<String>, Option<String>) = (None, None);
    Application {
        name,
        identifier: identifier.map(|value| value.chars().take(256).collect()),
        version: version.map(|value| value.chars().take(80).collect()),
        identity_source: if bundle.is_some() {
            "Installed application bundle"
        } else {
            "Executable filename; publisher not verified"
        }
        .into(),
        path,
    }
}

#[derive(Clone)]
struct CachedIcon {
    path: PathBuf,
    modified: Option<std::time::SystemTime>,
    expires: u64,
    value: Option<String>,
}

#[derive(Default)]
pub struct Icons(Mutex<VecDeque<CachedIcon>>);

impl Icons {
    pub fn get(&self, path: &Path) -> Option<String> {
        let mut entries = self.0.lock().ok()?;
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let now = crate::types::now();
        entries.retain(|entry| entry.expires > now);
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.path == path && entry.modified == modified)
        {
            let entry = entries.remove(index)?;
            let result = entry.value.clone();
            entries.push_back(entry);
            return result;
        }
        #[cfg(target_os = "macos")]
        let png = macos::icon(path);
        #[cfg(target_os = "windows")]
        let png = windows::icon(path);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let png: Option<Vec<u8>> = None;
        let value = png
            .filter(|bytes| bytes.len() <= 128 * 1024)
            .map(|bytes| format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
        if entries.len() >= 128 {
            entries.pop_front();
        }
        entries.push_back(CachedIcon {
            path: path.into(),
            modified,
            expires: now + 600,
            value: value.clone(),
        });
        value
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::{rc::autoreleasepool, AllocAnyThread};
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext,
        NSWorkspace,
    };
    use objc2_foundation::{NSBundle, NSDictionary, NSPoint, NSRect, NSSize, NSString};

    pub fn metadata(path: &Path) -> (Option<String>, Option<String>) {
        autoreleasepool(|_| {
            let Some(bundle) =
                NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
            else {
                return (None, None);
            };
            let identifier = bundle.bundleIdentifier().map(|id| id.to_string());
            let version = bundle
                .objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))
                .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()));
            (identifier, version)
        })
    }

    pub fn icon(path: &Path) -> Option<Vec<u8>> {
        autoreleasepool(|_| unsafe {
            let icon = NSWorkspace::sharedWorkspace()
                .iconForFile(&NSString::from_str(&path.to_string_lossy()));
            // Rasterize only 64 × 64 pixels, rather than serializing a multi-megabyte .icns file.
            let bitmap = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), 64, 64, 8, 4, true, false, NSDeviceRGBColorSpace, 0, 0,
            )?;
            let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
            NSGraphicsContext::saveGraphicsState_class();
            NSGraphicsContext::setCurrentContext(Some(&context));
            icon.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(64.0, 64.0)));
            NSGraphicsContext::restoreGraphicsState_class();
            bitmap
                .representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
                .map(|data| data.to_vec())
        })
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use ::windows::{
        core::PCWSTR,
        Win32::{
            Graphics::Gdi::*,
            Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
            UI::{Shell::*, WindowsAndMessaging::*},
        },
    };
    use std::{ffi::c_void, os::windows::ffi::OsStrExt};

    // One RAII owner covers every exit, including PNG failures and partially created GDI resources.
    #[derive(Default)]
    struct Drawing {
        icon: HICON,
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
    }
    impl Drop for Drawing {
        fn drop(&mut self) {
            unsafe {
                if !self.previous.0.is_null() {
                    let _ = SelectObject(self.dc, self.previous);
                }
                if !self.bitmap.0.is_null() {
                    let _ = DeleteObject(self.bitmap.into());
                }
                if !self.dc.0.is_null() {
                    let _ = DeleteDC(self.dc);
                }
                if !self.icon.0.is_null() {
                    let _ = DestroyIcon(self.icon);
                }
            }
        }
    }

    pub fn icon(path: &Path) -> Option<Vec<u8>> {
        unsafe {
            let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut info = SHFILEINFOW::default();
            if SHGetFileInfoW(
                PCWSTR(path.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            ) == 0
            {
                return None;
            }
            let mut drawing = Drawing {
                icon: info.hIcon,
                dc: CreateCompatibleDC(None),
                ..Default::default()
            };
            if drawing.dc.0.is_null() {
                return None;
            }
            let bitmap_info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 32,
                    biHeight: -32,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels: *mut c_void = std::ptr::null_mut();
            drawing.bitmap = CreateDIBSection(
                Some(drawing.dc),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut pixels,
                None,
                0,
            )
            .ok()?;
            drawing.previous = SelectObject(drawing.dc, drawing.bitmap.into());
            if drawing.previous.0 as isize == -1 {
                drawing.previous = HGDIOBJ::default();
                return None;
            }
            if pixels.is_null() || drawing.previous.0.is_null() {
                return None;
            }
            let buffer = std::slice::from_raw_parts_mut(pixels as *mut u8, 32 * 32 * 4);
            buffer.fill(0);
            DrawIconEx(drawing.dc, 0, 0, drawing.icon, 32, 32, 0, None, DI_NORMAL).ok()?;
            if !GdiFlush().as_bool() {
                return None;
            }
            let black = buffer.to_vec();
            buffer.fill(255);
            DrawIconEx(drawing.dc, 0, 0, drawing.icon, 32, 32, 0, None, DI_NORMAL).ok()?;
            if !GdiFlush().as_bool() {
                return None;
            }
            // Rendering on both backgrounds recovers alpha for modern and legacy mask icons.
            let mut rgba = Vec::with_capacity(buffer.len());
            for (dark, light) in black.chunks_exact(4).zip(buffer.chunks_exact(4)) {
                let alpha = 255 - light[0].saturating_sub(dark[0]);
                for channel in [2, 1, 0] {
                    rgba.push(if alpha == 0 {
                        0
                    } else {
                        (dark[channel] as u32 * 255 / alpha as u32).min(255) as u8
                    });
                }
                rgba.push(alpha);
            }
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 32, 32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
            }
            Some(bytes)
        }
    }
}
