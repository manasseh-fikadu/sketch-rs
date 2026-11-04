//! macOS integration: vibrancy behind the sidebar, traffic lights inset into the toolbar, and a
//! native menu bar (muda) wired to the editor's actions.

use crate::editor::shortcuts::Action;
use muda::accelerator::{Accelerator, Code, Modifiers as M};
use muda::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use objc2::MainThreadOnly;
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use objc2_foundation::NSPoint;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use winit::window::Window;

fn ns_window(window: &Window) -> Option<objc2::rc::Retained<NSWindow>> {
    let RawWindowHandle::AppKit(h) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit hands out a valid NSView pointer for the lifetime of the window, and we
    // are on the main thread (all winit window callbacks are).
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

/// Put an NSVisualEffectView (Sidebar material, behind-window blending) behind the app, so
/// wherever wgpu leaves pixels transparent the desktop shows through, blurred, as in a native
/// sidebar. It is a subview of winit's content view whose layer is pushed behind wgpu's Metal
/// sublayer (zPosition -1); as a plain subview it would cover the Metal layer, and AppKit
/// doesn't honour ordering for views added to the window's frame view.
/// Returns whether it was applied.
pub fn apply_vibrancy(window: &Window) -> bool {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return false;
    };
    let Some(win) = ns_window(window) else {
        return false;
    };
    let Some(content) = win.contentView() else {
        return false;
    };
    let fx = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content.bounds());
    fx.setMaterial(NSVisualEffectMaterial::Sidebar);
    fx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    fx.setState(NSVisualEffectState::FollowsWindowActiveState);
    fx.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    fx.setWantsLayer(true);
    content.addSubview_positioned_relativeTo(&fx, NSWindowOrderingMode::Below, None);
    // SAFETY: `layer` is a CALayer (the view is layer-backed); setZPosition: takes a CGFloat.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*fx, layer];
        if layer.is_null() {
            return false;
        }
        let _: () = msg_send![layer, setZPosition: -1.0f64];
    }
    true
}

/// Move the close/minimise/zoom buttons so they start `x` points from the left and are
/// vertically centred in a titlebar area `bar_h` points tall (our toolbar). AppKit lays the
/// titlebar out again on show, resize and fullscreen changes, so this is re-applied from
/// redraw; it returns early when the buttons are already in place.
pub fn inset_traffic_lights(window: &Window, x: f64, bar_h: f64) {
    let Some(win) = ns_window(window) else { return };
    let (Some(close), Some(mini), Some(zoom)) = (
        win.standardWindowButton(NSWindowButton::CloseButton),
        win.standardWindowButton(NSWindowButton::MiniaturizeButton),
        win.standardWindowButton(NSWindowButton::ZoomButton),
    ) else {
        return;
    };
    // SAFETY: plain view-hierarchy getters on main-thread AppKit objects.
    let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else {
        return;
    };
    let close_rect = close.frame();
    let btn_y = ((bar_h - close_rect.size.height) / 2.0).round();
    if (close_rect.origin.x - x).abs() < 0.5
        && (close_rect.origin.y - btn_y).abs() < 0.5
        && (container.frame().size.height - bar_h).abs() < 0.5
    {
        return;
    }
    let mut bar = container.frame();
    bar.size.height = bar_h;
    bar.origin.y = win.frame().size.height - bar_h;
    container.setFrame(bar);
    // The buttons' own superview (NSTitlebarView) must grow with the container.
    // SAFETY: as above.
    if let Some(tv) = unsafe { close.superview() } {
        let mut f = tv.frame();
        f.origin.y = 0.0;
        f.size.height = bar_h;
        tv.setFrame(f);
    }
    let gap = mini.frame().origin.x - close_rect.origin.x;
    for (i, b) in [close, mini, zoom].iter().enumerate() {
        b.setFrameOrigin(NSPoint::new(x + gap * i as f64, btn_y));
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MenuCmd {
    Run(Action),
    Quit,
    ToggleGrid,
    ToggleHelp,
}

pub struct NativeMenu {
    _menu: Menu,
    cmds: HashMap<MenuId, MenuCmd>,
    pub snapping: CheckMenuItem,
    pub grid: CheckMenuItem,
    pub help: CheckMenuItem,
}

impl NativeMenu {
    pub fn command(&self, id: &MenuId) -> Option<MenuCmd> {
        self.cmds.get(id).copied()
    }
}

fn acc(m: M, c: Code) -> Option<Accelerator> {
    Some(Accelerator::new(m, c))
}

fn key_code(k: &crate::editor::Key) -> Option<Code> {
    use crate::editor::Key as K;
    Some(match k {
        K::Char(c) => match c.to_ascii_lowercase() {
            'a' => Code::KeyA,
            'b' => Code::KeyB,
            'c' => Code::KeyC,
            'd' => Code::KeyD,
            'e' => Code::KeyE,
            'f' => Code::KeyF,
            'g' => Code::KeyG,
            'h' => Code::KeyH,
            'i' => Code::KeyI,
            'j' => Code::KeyJ,
            'k' => Code::KeyK,
            'l' => Code::KeyL,
            'm' => Code::KeyM,
            'n' => Code::KeyN,
            'o' => Code::KeyO,
            'p' => Code::KeyP,
            'q' => Code::KeyQ,
            'r' => Code::KeyR,
            's' => Code::KeyS,
            't' => Code::KeyT,
            'u' => Code::KeyU,
            'v' => Code::KeyV,
            'w' => Code::KeyW,
            'x' => Code::KeyX,
            'y' => Code::KeyY,
            'z' => Code::KeyZ,
            '0' => Code::Digit0,
            '1' => Code::Digit1,
            '2' => Code::Digit2,
            '=' => Code::Equal,
            '-' => Code::Minus,
            '[' => Code::BracketLeft,
            ']' => Code::BracketRight,
            '\'' => Code::Quote,
            '/' => Code::Slash,
            '\\' => Code::Backslash,
            _ => return None,
        },
        _ => return None,
    })
}

/// Build the App / File / Edit / Object / Vector / Text / View / Tools / Window menus from the
/// shared menu definition and install them on NSApp. Chords with ⌘ become key equivalents;
/// single-key shortcuts are shown in the title only (as key equivalents they would swallow
/// typing).
pub fn install_menu() -> NativeMenu {
    use crate::editor::shortcuts::{self, MenuEntry};
    let mut cmds = HashMap::new();
    let cmd = M::META;
    use Action as A;
    use MenuCmd::Run;

    fn build(
        entries: &[MenuEntry],
        cmds: &mut HashMap<MenuId, MenuCmd>,
        keep: &mut Vec<Box<dyn muda::IsMenuItem>>,
    ) -> Vec<usize> {
        let mut idx = Vec::new();
        for e in entries {
            match e {
                MenuEntry::Item(label, a) => {
                    let accel = shortcuts::chord_of(*a).and_then(|(k, c, sh, alt)| {
                        if !c {
                            return None;
                        }
                        let mut m = M::META;
                        if sh {
                            m |= M::SHIFT;
                        }
                        if alt {
                            m |= M::ALT;
                        }
                        key_code(&k).map(|code| Accelerator::new(m, code))
                    });
                    let title = if accel.is_none() {
                        let t = shortcuts::chord_text(*a);
                        if t.is_empty() {
                            label.to_string()
                        } else {
                            format!("{label}\t{t}")
                        }
                    } else {
                        label.to_string()
                    };
                    let it = MenuItem::new(title, true, accel);
                    cmds.insert(it.id().clone(), MenuCmd::Run(*a));
                    keep.push(Box::new(it));
                }
                MenuEntry::Separator => keep.push(Box::new(PredefinedMenuItem::separator())),
                MenuEntry::Submenu(label, items) => {
                    let mut sub_keep = Vec::new();
                    build(items, cmds, &mut sub_keep);
                    let refs: Vec<&dyn muda::IsMenuItem> =
                        sub_keep.iter().map(|b| b.as_ref()).collect();
                    let sub = Submenu::with_items(*label, true, &refs).expect("submenu");
                    keep.push(Box::new(sub));
                }
            }
            idx.push(keep.len() - 1);
        }
        idx
    }

    let quit = MenuItem::new("Quit Sketch", true, acc(cmd, Code::KeyQ));
    cmds.insert(quit.id().clone(), MenuCmd::Quit);
    let _ = (A::Undo, Run(A::Undo));
    let snapping = CheckMenuItem::new("Snapping", true, true, acc(cmd, Code::Quote));
    cmds.insert(snapping.id().clone(), Run(A::ToggleSnapping));
    let grid = CheckMenuItem::new("Dot Grid", true, true, None);
    cmds.insert(grid.id().clone(), MenuCmd::ToggleGrid);
    let help = CheckMenuItem::new("Keyboard Shortcuts", true, false, acc(cmd, Code::Slash));
    cmds.insert(help.id().clone(), MenuCmd::ToggleHelp);

    let sep = PredefinedMenuItem::separator;
    let app_menu = Submenu::with_items(
        "Sketch",
        true,
        &[
            &PredefinedMenuItem::about(Some("About Sketch"), None),
            &sep(),
            &PredefinedMenuItem::services(None),
            &sep(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &sep(),
            &quit,
        ],
    )
    .expect("app menu");
    let mut menus: Vec<Submenu> = Vec::new();
    for (title, entries) in shortcuts::menus() {
        let mut keep = Vec::new();
        build(&entries, &mut cmds, &mut keep);
        let mut refs: Vec<&dyn muda::IsMenuItem> = keep.iter().map(|b| b.as_ref()).collect();
        let extra_sep = PredefinedMenuItem::separator();
        let fullscreen = PredefinedMenuItem::fullscreen(None);
        let close = PredefinedMenuItem::close_window(None);
        if title == "View" {
            refs.push(&extra_sep);
            refs.push(&snapping);
            refs.push(&grid);
            refs.push(&help);
            refs.push(&fullscreen);
        }
        if title == "File" {
            refs.push(&extra_sep);
            refs.push(&close);
        }
        menus.push(Submenu::with_items(title, true, &refs).expect("menu"));
    }
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(Some("Zoom")),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    )
    .expect("window menu");
    let mut top: Vec<&dyn muda::IsMenuItem> = vec![&app_menu];
    for m in &menus {
        top.push(m);
    }
    top.push(&window);
    let menu = Menu::with_items(&top).expect("menu");
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    NativeMenu {
        _menu: menu,
        cmds,
        snapping,
        grid,
        help,
    }
}
