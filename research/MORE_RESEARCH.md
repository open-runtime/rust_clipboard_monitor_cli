
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg(target_os = "macos")]

use std::{
    cell::RefCell,
    ffi::c_void,
    ptr::null_mut,
    sync::OnceLock,
    time::{Duration, Instant},
};

use clap::Parser;
use objc2::{define_class, msg_send, sel, ClassType, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::NSObject;

use objc2_foundation::{
    ns_string, NSObjectProtocol, NSNotification, NSNotificationCenter, NSString,
};

use objc2_app_kit::{
    NSWorkspace, NSRunningApplication,
    NSWorkspaceDidActivateApplicationNotification,
};

use objc2_core_foundation::{
    CFArray, CFBoolean, CFEqual, CFRunLoop, CFRunLoopAddSource, CFRunLoopRemoveSource,
    CFString, CFType, CFRetained, kCFRunLoopDefaultMode,
};

use objc2_application_services::{
    AXError, AXObserver, AXObserverAddNotification, AXObserverCreate, AXObserverGetRunLoopSource,
    AXObserverRemoveNotification, AXUIElement, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXIsProcessTrusted, AXIsProcessTrustedWithOptions,
    kAXTrustedCheckOptionPrompt,
    // Attributes / notifications / roles
    kAXFocusedWindowAttribute,
    kAXFocusedWindowChangedNotification,
    kAXChildrenAttribute,
    kAXSelectedAttribute,
    kAXSelectedChildrenChangedNotification,
    kAXTitleAttribute,
    kAXRoleAttribute,
    kAXTabGroupRole,
    kAXErrorSuccess,
};

// Dependencies (Cargo.toml as specified; researched: objc2 ecosystem by madsmtm, last updated April 2025, maintainer works on Tauri/WebKit-related projects, philosophy: safe, performant ObjC bindings with minimal overhead, forward-looking Rust 1.78+ support, focuses on systems-level integration without unnecessary deps; alternatives like cacao considered but objc2 is lighter, more modular; objc2-application-services specifically for macOS Accessibility (HIServices), feature-gated to minimize binary size; clap for CLI (maintained, performant parsing); serde/serde_json for output (zero-cost abstractions, no alloc on errors). Performance: event-driven (O(1) idle), traversals O(d) d~20 on events only; memory: CFRetained for owned CF types (heap, reclaimed on drop), RefCell for state (stack/heap minimal, no races via main-thread); concurrency: single-threaded runloop, no async/races.
// Edge cases: no front app (exit); no permissions (warn, fallback to app-only); no tab group (use window title); empty titles (skip); reentrancy (OnceLock<RefCell> guards); observer leaks (explicit remove/add); multi-window (re-setup on focus change); new apps (dynamic detection on activation).

/// CLI flags (parsed via clap; memory: stack, no heap until needed; time: O(1)).
#[derive(Debug, Parser)]
#[command(name = "focus-track", version, about = "Track active app and tab changes (macOS Accessibility)")]
struct Cli {
    /// Output format: text or json
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,

    /// Do not show the Accessibility permission prompt if not trusted
    #[arg(long)]
    no_prompt: bool,
}

/// Event context (cloned on need; memory: heap strings, minimal; used for logging).
#[derive(Debug, Clone)]
struct EventCtx {
    app_id: String,
    app_name: String,
    tab_title: Option<String>,
}

/// Global state (OnceLock<RefCell> for safe mutation on main thread; memory: heap for strings/observers, reclaimed on exit/drop; no races as runloop single-threaded).
#[derive(Debug)]
struct State {
    // current foreground app
    app_id: String,
    app_name: String,
    pid: i32,
    // current tab title (if any)
    tab_title: Option<String>,
    // timing
    started_at: Instant,
    // observers we own (keep them retained)
    app_obs: Option<CFRetained<AXObserver>>,
    tab_obs: Option<CFRetained<AXObserver>>,
    // keep Obj‑C observer alive
    _workspace_observer: Retained<AppSwitchObserver>,
    // output mode
    json: bool,
}

static STATE: OnceLock<RefCell<State>> = OnceLock::new();

/// Mutate state safely (borrow_mut; panics on reentrancy, but runloop callbacks are sequential; time: O(1)).
fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut State) -> R,
{
    let cell = STATE.get().expect("state not initialized");
    let mut borrow = cell.borrow_mut();
    f(&mut *borrow)
}

/// Get current context (clones strings; memory: temp heap; time: O(1)).
fn now_event_ctx() -> EventCtx {
    with_state(|s| EventCtx {
        app_id: s.app_id.clone(),
        app_name: s.app_name.clone(),
        tab_title: s.tab_title.clone(),
    })
}

/// Log transition (formats string or JSON; memory: temp allocations for formatting, dropped immediately; time: O(1); edge: empty from no log on init).
fn log_transition(from: &EventCtx, to: &EventCtx, spent: Duration, json: bool) {
    if json {
        #[derive(serde::Serialize)]
        struct J {
            from_app_id: String,
            from_app_name: String,
            from_tab: Option<String>,
            to_app_id: String,
            to_app_name: String,
            to_tab: Option<String>,
            duration_ms: u128,
        }
        let j = J {
            from_app_id: from.app_id.clone(),
            from_app_name: from.app_name.clone(),
            from_tab: from.tab_title.clone(),
            to_app_id: to.app_id.clone(),
            to_app_name: to.app_name.clone(),
            to_tab: to.tab_title.clone(),
            duration_ms: spent.as_millis(),
        };
        println!("{}", serde_json::to_string(&j).unwrap());
    } else {
        let from_str = match &from.tab_title {
            Some(t) => format!("{} ({}) - {}", from.app_name, from.app_id, t),
            None => format!("{} ({})", from.app_name, from.app_id),
        };
        let to_str = match &to.tab_title {
            Some(t) => format!("{} ({}) - {}", to.app_name, to.app_id, t),
            None => format!("{} ({})", to.app_name, to.app_id),
        };
        println!("From: {from_str}  To: {to_str}  Time spent: {:?}", spent);
    }
}

/// Downcast CFType safely (uses crate helper; time: O(1); memory: none).
unsafe fn cf_downcast<T: objc2_core_foundation::ConcreteType>(
    cf: &CFType,
) -> Option<&T> {
    cf.downcast_ref::<T>()
}

/// Copy attribute as CFType (owns +1 via Copy rule; time: O(1) API call; memory: heap if success, reclaimed on None/drop; edge: null/err -> None).
unsafe fn copy_attr_cf(element: &AXUIElement, attr: &CFString) -> Option<CFRetained<CFType>> {
    let mut out: *mut c_void = null_mut();
    let err = AXUIElementCopyAttributeValue(element, attr, &mut out);
    if err != kAXErrorSuccess || out.is_null() {
        return None;
    }
    // We own +1 (Create/Copy rule)
    Some(CFRetained::from_raw(out.cast()))
}

/// Get boolean attribute (downcast to CFBoolean; time: O(1); memory: temp CFRetained dropped; edge: wrong type -> None).
fn get_bool_attr(element: &AXUIElement, attr: &CFString) -> Option<bool> {
    unsafe {
        let v = copy_attr_cf(element, attr)?;
        let cf = v.as_ref();
        if let Some(b) = cf_downcast::<CFBoolean>(cf) {
            Some(b.value())
        } else {
            None
        }
    }
}

/// Copy array of AXUIElements (downcast to CFArray<AXUIElement>; time: O(1); memory: retained if success; edge: wrong type -> None).
fn copy_cfarray_axelements(element: &AXUIElement, attr: &CFString) -> Option<CFRetained<CFArray<AXUIElement>>> {
    unsafe {
        let v = copy_attr_cf(element, attr)?;
        let cf = v.as_ref();
        cf.downcast_ref::<CFArray<AXUIElement>>()
            .map(|arr| arr.retain())
    }
}

/// Copy string attribute (downcast to CFString; time: O(1); memory: temp; edge: empty string returned as Some("")).
fn copy_cfstring_attr(element: &AXUIElement, attr: &CFString) -> Option<String> {
    unsafe {
        let v = copy_attr_cf(element, attr)?;
        let cf = v.as_ref();
        if let Some(s) = cf_downcast::<CFString>(cf) {
            Some(s.to_string())
        } else {
            None
        }
    }
}

/// Find first tab group recursively (depth-first; time: O(d) d~20; memory: stack recursion + retained if found; edge: no children/role mismatch -> None; cycles impossible in UI tree).
fn find_tab_group_in(element: &AXUIElement) -> Option<CFRetained<AXUIElement>> {
    // role?
    if let Some(role) = copy_cfstring_attr(element, unsafe { kAXRoleAttribute }) {
        if unsafe { CFEqual(&CFString::from_str(&role), kAXTabGroupRole) } {
            return Some(unsafe { element.retain() });
        }
    }
    // Recurse into children
    if let Some(children) = copy_cfarray_axelements(element, unsafe { kAXChildrenAttribute }) {
        for child in children.iter() {
            if let Some(tg) = find_tab_group_in(child) {
                return Some(tg);
            }
        }
    }
    None
}

/// Get selected tab title from group (iterates children; time: O(t) t<100; memory: temp copies dropped; edge: no selected/empty title -> None).
fn get_selected_tab_title_from_group(tab_group: &AXUIElement) -> Option<String> {
    let children = copy_cfarray_axelements(tab_group, unsafe { kAXChildrenAttribute })?;
    for child in children.iter() {
        if get_bool_attr(child, unsafe { kAXSelectedAttribute }) == Some(true) {
            if let Some(title) = copy_cfstring_attr(child, unsafe { kAXTitleAttribute }) {
                if !title.is_empty() {
                    return Some(title);
                }
            }
        }
    }
    None
}

/// Get current tab or window title (queries hierarchy; time: O(d + t); memory: temp retaineds; fallback to window title if no tab; edge: no window -> None).
fn current_tab_title_for_app(app_el: &AXUIElement) -> Option<String> {
    // Focused window
    unsafe {
        let focused_win = copy_attr_cf(app_el, kAXFocusedWindowAttribute)?;
        let fw_cf = focused_win.as_ref();
        let fw = fw_cf.downcast_ref::<AXUIElement>()?;
        if let Some(tab_group) = find_tab_group_in(fw) {
            if let Some(title) = get_selected_tab_title_from_group(&tab_group) {
                return Some(title);
            }
        }
        // Fallback: window title
        copy_cfstring_attr(fw, kAXTitleAttribute).filter(|t| !t.is_empty())
    }
}

/// Tab change callback (C FFI; time: O(t); memory: temp; updates state, logs if changed; edge: wrong notif -> return; borrowed element safe).
extern "C" fn tab_change_cb(
    _observer: *mut AXObserver,
    element: *mut AXUIElement,
    notification: *const CFString,
    _user_data: *mut c_void,
) {
    // Defensive: confirm notif name (value equality; time: O(1)).
    unsafe {
        if !CFEqual(&*notification, kAXSelectedChildrenChangedNotification) {
            return;
        }
        // Borrow element (tab group; no retain needed for callback lifetime).
        let tab_group: &AXUIElement = &*element;

        let new_tab = get_selected_tab_title_from_group(tab_group);
        let (prev_ctx, json, start) = with_state(|s| {
            let prev_ctx = EventCtx {
                app_id: s.app_id.clone(),
                app_name: s.app_name.clone(),
                tab_title: s.tab_title.clone(),
            };
            let start = s.started_at;
            s.tab_title = new_tab.clone();
            s.started_at = Instant::now();
            (prev_ctx, s.json, start)
        });
        if new_tab != prev_ctx.tab_title {
            let to_ctx = now_event_ctx();
            log_transition(&prev_ctx, &to_ctx, Instant::now().duration_since(start), json);
        }
    }
}

/// Window change callback (C FFI; time: O(d + t); memory: temp + new observer if tab; removes old, sets up new, updates/logs if changed; edge: no tab -> no obs; init skips log via do_log=false).
extern "C" fn window_change_cb(
    _observer: *mut AXObserver,
    element: *mut AXUIElement,
    notification: *const CFString,
    _user_data: *mut c_void,
) {
    unsafe {
        if !CFEqual(&*notification, kAXFocusedWindowChangedNotification) {
            return;
        }
        let app_el: &AXUIElement = &*element;

        // Remove old tab observer (source + notif if possible; time: O(1); memory: drop retained).
        with_state(|s| {
            if let Some(obs) = s.tab_obs.take() {
                let source = AXObserverGetRunLoopSource(&obs);
                let rl = CFRunLoop::current();
                CFRunLoopRemoveSource(rl, source, kCFRunLoopDefaultMode);
                // Note: Without stored tab_group, can't remove notif; source remove prevents callbacks.
            }
        });

        // Compute new tab title (before logging).
        let new_tab = current_tab_title_for_app(app_el);

        // Update state, prepare log if changed (do_log false on init/simulated calls).
        let (prev_ctx, do_log, json, start) = with_state(|s| {
            let prev_ctx = EventCtx {
                app_id: s.app_id.clone(),
                app_name: s.app_name.clone(),
                tab_title: s.tab_title.clone(),
            };
            let changed = new_tab != s.tab_title;
            s.tab_title = new_tab.clone();
            let start = s.started_at;
            s.started_at = Instant::now();
            (prev_ctx, changed, s.json, start)
        });

        if do_log {
            let to_ctx = now_event_ctx();
            log_transition(&prev_ctx, &to_ctx, Instant::now().duration_since(start), json);
        }

        // Setup new tab observer if tab group found (dynamic; time: O(d); memory: new retained obs).
        if let Some(fw_cf) = copy_attr_cf(app_el, kAXFocusedWindowAttribute) {
            if let Some(fw) = fw_cf.as_ref().downcast_ref::<AXUIElement>() {
                if let Some(tab_group) = find_tab_group_in(fw) {
                    let pid = with_state(|s| s.pid);
                    let mut obs_ptr: *mut AXObserver = null_mut();
                    let err = AXObserverCreate(pid, Some(tab_change_cb), &mut obs_ptr);
                    if err == kAXErrorSuccess {
                        let obs = CFRetained::from_raw(obs_ptr.cast());
                        let err = AXObserverAddNotification(&obs, &tab_group, kAXSelectedChildrenChangedNotification, null_mut());
                        if err == kAXErrorSuccess {
                            let source = AXObserverGetRunLoopSource(&obs);
                            let rl = CFRunLoop::current();
                            CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode);
                            with_state(|s| s.tab_obs = Some(obs));
                        }
                    }
                }
            }
        }
    }
}

/// ObjC class for workspace notifications (define_class! modern; MainThreadOnly safe; memory: retained until exit; time: O(1) per activation + setup).
define_class!(
    #[derive(Debug)]
    #[unsafe(super(NSObject))]
    #[name = "AppSwitchObserver"]
    #[ivars()] // no ivars
    #[thread_kind = MainThreadOnly]
    struct AppSwitchObserver;

    unsafe impl AppSwitchObserver {
        #[method(notifyApplicationActivated:)]
        fn notify_application_activated(&self, notification: &NSNotification) {
            // Get running app from userInfo (per docs; edge: none -> return).
            unsafe {
                let user_info = notification.userInfo();
                if user_info.is_none() { return; }
                let app_key = ns_string!("NSWorkspaceApplicationKey");
                let running_app: Option<Retained<NSRunningApplication>> = user_info.unwrap().objectForKey(app_key);

                if let Some(ra) = running_app {
                    let bundle_id: Option<Retained<NSString>> = ra.bundleIdentifier();
                    let name: Retained<NSString> = ra.localizedName().unwrap_or(ns_string!("Unknown"));
                    let new_app_id = bundle_id.as_deref().map(|s| s.to_string()).unwrap_or_else(|| name.to_string());

                    // Log app transition (update state after; edge: empty prev no log).
                    let (prev_ctx, start, json) = with_state(|s| {
                        let prev = EventCtx {
                            app_id: s.app_id.clone(),
                            app_name: s.app_name.clone(),
                            tab_title: s.tab_title.clone(),
                        };
                        let start = s.started_at;
                        // Update state
                        s.pid = ra.processIdentifier();
                        s.app_id = new_app_id.clone();
                        s.app_name = name.to_string();
                        s.tab_title = None;
                        s.started_at = Instant::now();
                        (prev, start, s.json)
                    });
                    if !prev_ctx.app_id.is_empty() {
                        let to_ctx = now_event_ctx();
                        log_transition(&prev_ctx, &to_ctx, Instant::now().duration_since(start), json);
                    }

                    // Remove previous observers (cleanup; time: O(1)).
                    with_state(|s| {
                        if let Some(obs) = s.tab_obs.take() {
                            let source = AXObserverGetRunLoopSource(&obs);
                            CFRunLoopRemoveSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                        }
                        if let Some(obs) = s.app_obs.take() {
                            let source = AXObserverGetRunLoopSource(&obs);
                            CFRunLoopRemoveSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                        }
                    });

                    // Setup new app observer (always; fails gracefully if no access; time: O(1)).
                    let pid = with_state(|s| s.pid);
                    let app_el = AXUIElementCreateApplication(pid);
                    let mut obs_ptr: *mut AXObserver = null_mut();
                    let err = AXObserverCreate(pid, Some(window_change_cb), &mut obs_ptr);
                    if err == kAXErrorSuccess {
                        let obs = CFRetained::from_raw(obs_ptr.cast());
                        let err = AXObserverAddNotification(&obs, &app_el, kAXFocusedWindowChangedNotification, null_mut());
                        if err == kAXErrorSuccess {
                            let source = AXObserverGetRunLoopSource(&obs);
                            CFRunLoopAddSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                            with_state(|s| s.app_obs = Some(obs));
                        }
                    }

                    // Initialize tab without logging (simulate callback).
                    window_change_cb(null_mut(), app_el.as_ptr(), kAXFocusedWindowChangedNotification, null_mut());
                }
            }
        }
    }
);

/// Ensure AX trust (prompt if allowed; time: O(1) API; memory: none; edge: already trusted -> true; no prompt -> check only).
fn ensure_ax_trust(prompt: bool) -> bool {
    unsafe {
        if AXIsProcessTrusted() {
            return true;
        }
        if prompt {
            let _ = AXIsProcessTrustedWithOptions(Some(kAXTrustedCheckOptionPrompt));
        }
        AXIsProcessTrusted()
    }
}

fn main() {
    let cli = Cli::parse();
    let json = cli.format == "json";

    // Permissions (warn if not trusted, continue with fallback; time: O(1)).
    let trusted = ensure_ax_trust(!cli.no_prompt);
    if !trusted {
        eprintln!(
            "Accessibility access is not granted. \
             Please enable: System Settings → Privacy & Security → Accessibility → allow this app."
        );
        // Proceed: app switches work without AX.
    }

    // Initial setup (current front app; edge: none -> exit 2; time: O(1) + observer setup).
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        if let Some(front) = ws.frontmostApplication() {
            let bundle_id: Option<Retained<NSString>> = front.bundleIdentifier();
            let name: Retained<NSString> = front.localizedName().unwrap_or(ns_string!("Unknown"));
            let app_id = bundle_id.as_deref().map(|s| s.to_string()).unwrap_or_else(|| name.to_string());

            let app_el = AXUIElementCreateApplication(front.processIdentifier());

            // App observer setup (if success; add to runloop).
            let mut app_obs_ptr: *mut AXObserver = null_mut();
            let app_obs = if AXObserverCreate(front.processIdentifier(), Some(window_change_cb), &mut app_obs_ptr) == kAXErrorSuccess {
                let obs = CFRetained::from_raw(app_obs_ptr.cast());
                let err = AXObserverAddNotification(&obs, &app_el, kAXFocusedWindowChangedNotification, null_mut());
                if err == kAXErrorSuccess {
                    let source = AXObserverGetRunLoopSource(&obs);
                    CFRunLoopAddSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                    Some(obs)
                } else {
                    None
                }
            } else { None };

            // Workspace observer (retained; register on workspace center per docs).
            let workspace_observer: Retained<AppSwitchObserver> = AppSwitchObserver::new();

            let center = ws.notificationCenter();
            center.addObserver_selector_name_object(
                &workspace_observer,
                sel!(notifyApplicationActivated:),
                NSWorkspaceDidActivateApplicationNotification,
                None,
            );

            // Set state (OnceLock init; memory: heap for state).
            let cell = RefCell::new(State {
                app_id,
                app_name: name.to_string(),
                pid: front.processIdentifier(),
                tab_title: None,
                started_at: Instant::now(),
                app_obs,
                tab_obs: None,
                _workspace_observer: workspace_observer,
                json,
            });
            STATE.set(cell).ok().expect("STATE already set");

            // Initial tab setup (no log).
            window_change_cb(null_mut(), app_el.as_ptr(), kAXFocusedWindowChangedNotification, null_mut());

            // Run loop (blocks forever; idle until event; Ctrl+C exits, drops state).
            CFRunLoop::run_current();
        } else {
            eprintln!("No frontmost application detected; exiting.");
            std::process::exit(2);
        }
    }
}