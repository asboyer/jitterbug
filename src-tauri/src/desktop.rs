use crate::{
    types::{Snapshot, Workload},
    AppState,
};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, WINDOW_SUBMENU_ID},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

const WIDTH: f64 = 352.0;
const HEIGHT: f64 = 486.0;

#[derive(Clone, Serialize)]
struct Navigation {
    page: String,
    workload: Option<Workload>,
}

#[tauri::command]
pub fn open_main(
    app: tauri::AppHandle,
    page: String,
    workload_id: Option<String>,
) -> Result<(), String> {
    if !matches!(page.as_str(), "dashboard" | "settings") {
        return Err("Unknown destination.".into());
    }
    let window = app
        .get_webview_window("main")
        .ok_or("Main window unavailable.")?;
    // Resolve from the full inventory: the ranking may have changed since the click.
    let state = app.state::<AppState>();
    let workload = {
        let snapshot = state
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?;
        let workload = workload_id
            .map(|id| {
                snapshot
                    .workloads
                    .iter()
                    .find(|w| w.id == id)
                    .cloned()
                    .ok_or("This workload is no longer observed. Choose a current workload.")
            })
            .transpose()?;
        let _ = window.emit("telemetry", snapshot.dashboard_view());
        workload
    };
    window
        .emit("navigate", Navigation { page, workload })
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    hide_quick_view(app);
    Ok(())
}

#[tauri::command]
pub fn hide_quick_view(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("menubar") {
        let _ = window.hide();
    }
}

/// Monitoring continues when hidden; webviews only receive live data while visible.
pub fn publish(app: &tauri::AppHandle, snapshot: &Snapshot) {
    for label in ["main", "menubar"] {
        if let Some(window) = app.get_webview_window(label) {
            if window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false) {
                let view = if label == "menubar" {
                    snapshot.quick_view()
                } else {
                    snapshot.dashboard_view()
                };
                let _ = window.emit("telemetry", view);
            }
        }
    }
}

pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let application_menu = Menu::default(app.handle())?;
    let quick = MenuItem::with_id(
        app,
        "quick-view",
        "Quick View",
        true,
        Some("CmdOrCtrl+Shift+M"),
    )?;
    let preferences =
        MenuItem::with_id(app, "preferences", "Settings…", true, Some("CmdOrCtrl+,"))?;
    for item in application_menu.items()? {
        if let MenuItemKind::Submenu(submenu) = item {
            if submenu.id().as_ref() == WINDOW_SUBMENU_ID {
                submenu.append(&quick)?;
                submenu.append(&preferences)?;
            }
        }
    }
    app.set_menu(application_menu)?;
    app.on_menu_event(|app, event| match event.id.as_ref() {
        "quick-view" => {
            if let Err(error) = toggle(app) {
                eprintln!("Could not open menu bar panel: {error}");
            }
        }
        "preferences" => {
            let _ = open_main(app.clone(), "settings".into(), None);
        }
        _ => (),
    });
    let show = MenuItem::with_id(app, "show", "Open Jitterbug", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Jitterbug", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &quit])?;
    TrayIconBuilder::with_id("jitterbug")
        .icon(template_icon())
        .icon_as_template(true)
        .tooltip("Jitterbug — resource assistant")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = open_main(app.clone(), "dashboard".into(), None);
            }
            "settings" => {
                let _ = open_main(app.clone(), "settings".into(), None);
            }
            "quit" => app.exit(0),
            _ => (),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(error) = toggle(tray.app_handle()) {
                    // No private telemetry or process metadata is included in this error.
                    eprintln!("Could not open menu bar panel: {error}");
                    let _ = open_main(tray.app_handle().clone(), "dashboard".into(), None);
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn toggle(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("menubar") {
        if window.is_visible()? {
            return window.hide();
        }
    }
    // A second webview is only created when the user first opens the panel.
    let window = match app.get_webview_window("menubar") {
        Some(window) => window,
        None => {
            let builder = tauri::WebviewWindowBuilder::new(
                app,
                "menubar",
                tauri::WebviewUrl::App("index.html?view=menubar".into()),
            )
            .title("Jitterbug Quick View")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .shadow(true);
            #[cfg(target_os = "macos")]
            let builder = builder.transparent(true).effects(
                tauri::window::EffectsBuilder::new()
                    .effect(tauri::utils::WindowEffect::Popover)
                    .state(tauri::utils::WindowEffectState::Active)
                    .radius(10.0)
                    .build(),
            );
            builder.build()?
        }
    };
    position_quick_view(app, &window)?;
    window.show()?;
    window.set_focus()?;
    let state = app.state::<AppState>();
    if let Ok(snapshot) = state.snapshot.read() {
        let _ = window.emit("telemetry", snapshot.quick_view());
    }
    Ok(())
}

// macOS tray events use backing pixels, while monitor lookup uses Cocoa points.
// Read the status item's actual screen on the main thread, and keep all placement
// in points. This also avoids converting with the panel's *previous* screen scale.
#[cfg(target_os = "macos")]
fn position_quick_view(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let tray = app
        .tray_by_id("jitterbug")
        .ok_or_else(|| std::io::Error::other("Menu bar icon unavailable"))?;
    let position = tray
        .with_inner_tray_icon(|tray| {
            let mtm = MainThreadMarker::new()?;
            let button = tray.ns_status_item()?.button(mtm)?;
            let tray_window = button.window()?;
            let anchor = tray_window.frame();
            let area = tray_window.screen()?.visibleFrame();
            // Cocoa's global origin is the bottom-left of the primary screen;
            // Tauri's logical origin is its top-left, including on other displays.
            let primary = NSScreen::screens(mtm).firstObject()?.frame();
            Some(cocoa_panel_position(
                [
                    anchor.origin.x,
                    anchor.origin.y,
                    anchor.size.width,
                    anchor.size.height,
                ],
                [
                    area.origin.x,
                    area.origin.y,
                    area.size.width,
                    area.size.height,
                ],
                primary.origin.y + primary.size.height,
            ))
        })?
        .ok_or_else(|| std::io::Error::other("Menu bar position unavailable"))?;
    window.set_position(tauri::LogicalPosition::new(position.0, position.1))
}

#[cfg(any(target_os = "macos", test))]
fn cocoa_panel_position(anchor: [f64; 4], area: [f64; 4], primary_top: f64) -> (f64, f64) {
    let to_top_left = |rect: [f64; 4]| [rect[0], primary_top - rect[1] - rect[3], rect[2], rect[3]];
    panel_position(to_top_left(anchor), to_top_left(area), [WIDTH, HEIGHT], 6.0)
}

#[cfg(not(target_os = "macos"))]
fn position_quick_view(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let rect = app
        .tray_by_id("jitterbug")
        .ok_or_else(|| std::io::Error::other("Tray icon unavailable"))?
        .rect()?
        .ok_or_else(|| std::io::Error::other("Tray position unavailable"))?;
    let anchor = rect.position.to_physical::<f64>(1.0);
    let monitor = app
        .monitor_from_point(anchor.x, anchor.y)?
        .ok_or_else(|| std::io::Error::other("Tray display unavailable"))?;
    let scale = monitor.scale_factor();
    let origin = rect.position.to_physical::<f64>(scale);
    let size = rect.size.to_physical::<f64>(scale);
    let area = monitor.work_area();
    let (x, y) = panel_position(
        [origin.x, origin.y, size.width, size.height],
        [
            area.position.x as f64,
            area.position.y as f64,
            area.size.width as f64,
            area.size.height as f64,
        ],
        [WIDTH * scale, HEIGHT * scale],
        6.0 * scale,
    );
    window.set_position(tauri::PhysicalPosition::new(x as i32, y as i32))
}

fn panel_position(anchor: [f64; 4], area: [f64; 4], panel: [f64; 2], gap: f64) -> (f64, f64) {
    let left = area[0] + gap;
    let top = area[1] + gap;
    let right = (area[0] + area[2] - panel[0] - gap).max(left);
    let bottom = (area[1] + area[3] - panel[1] - gap).max(top);
    let x = (anchor[0] + anchor[2] / 2.0 - panel[0] / 2.0).clamp(left, right);
    let below = anchor[1] + anchor[3] + gap;
    let y = if below > bottom {
        anchor[1] - panel[1] - gap
    } else {
        below
    };
    (x, y.clamp(top, bottom))
}

fn template_icon() -> tauri::image::Image<'static> {
    // Original 18 pt companion glyph, rendered at 2x. macOS supplies the appearance tint.
    // Transparent background, antialiased alpha, no baked-in color or white square.
    let mut rgba = vec![0u8; 36 * 36 * 4];
    for y in 0..36 {
        for x in 0..36 {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f64 + (sx as f64 + 0.5) / 4.0) / 2.0;
                    let py = (y as f64 + (sy as f64 + 0.5) / 4.0) / 2.0;
                    let radius = ((px - 9.0).powi(2) + (py - 9.0).powi(2)).sqrt();
                    let outline = (6.1..=7.4).contains(&radius);
                    let eyes = [6.0, 12.0]
                        .iter()
                        .any(|cx| (px - cx).abs() <= 0.65 && (6.8..=8.7).contains(&py));
                    let smile_y = 12.6 - (px - 9.0).powi(2) * 0.22;
                    let smile = (6.0..=12.0).contains(&px) && (py - smile_y).abs() < 0.65;
                    if outline || eyes || smile {
                        hits += 1;
                    }
                }
            }
            let offset = (y * 36 + x) * 4;
            #[cfg(not(target_os = "macos"))]
            {
                rgba[offset..offset + 3].fill(128);
            }
            rgba[offset + 3] = (hits * 255 / 16) as u8;
        }
    }
    tauri::image::Image::new_owned(rgba, 36, 36)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_panel_uses_status_item_screen_points() {
        // A Retina tray's physical x would fall outside the logical screen.
        // Placement reads native points, so no backing scale enters this math.
        assert_eq!(
            cocoa_panel_position([1200., 958., 24., 24.], [0., 0., 1512., 958.], 982.),
            (1036., 30.)
        );
        // A secondary display left of and above the primary; its backing scale
        // can differ from the screen on which the panel was previously opened.
        assert_eq!(
            cocoa_panel_position([-400., 1176., 24., 24.], [-1920., 120., 1920., 1056.], 982.),
            (-564., -188.)
        );
        // Right edge clamps horizontally, but stays six points below the bar.
        assert_eq!(
            cocoa_panel_position([1480., 958., 24., 24.], [0., 0., 1512., 958.], 982.),
            (1154., 30.)
        );
    }

    #[test]
    fn panel_stays_on_monitor_and_flips_above_bottom_taskbar() {
        assert_eq!(
            panel_position(
                [990., 0., 20., 24.],
                [0., 24., 1024., 744.],
                [352., 486.],
                6.
            ),
            (666., 30.)
        );
        assert_eq!(
            panel_position(
                [-20., 760., 20., 24.],
                [-1024., 0., 1024., 760.],
                [352., 486.],
                6.
            ),
            (-358., 268.)
        );
        let (x, y) = panel_position(
            [-1000., 0., 20., 24.],
            [-1024., 24., 1024., 744.],
            [704., 972.],
            12.,
        );
        assert_eq!((x, y), (-1012., 36.));
    }
}
