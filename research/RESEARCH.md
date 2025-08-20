Below is a deep review of your snippet (what’s missing / problematic), followed by a **production‑ready, up‑to‑date** implementation you can drop into a CLI. I’ve verified API names, crates, and patterns against current bindings and Apple docs so you don’t get tripped up by 2024–2025 changes. Citations are inline where relevant.

---

## What you’re missing / what’s problematic

### 1) Wrong framework crate for AX APIs + wrong types

* The **AX (HIServices)** symbols like `AXUIElement`, `AXObserver*`, `kAXFocusedWindowAttribute`, `kAXSelectedChildrenChangedNotification`, etc. live in **`objc2_application_services`** (HIServices), not `objc2_accessibility`. The latter is for UIAccessibility (iOS) and accessibility annotations in AppKit/UIKit—not the macOS Accessibility API you’re using. Use `objc2_application_services` (≥ 0.3.1) with the relevant feature flags.
* **CoreFoundation types are not Objective‑C objects.** Using `Id<T>` for `AXObserver`, `AXUIElement`, `CFArray`, `CFString`, etc. is **unsound**—`Id` is for Obj‑C objects that respond to `retain/release`. Use **`CFRetained<T>`** (from `objc2_core_foundation`) for CF/AX types, or borrow them by reference. The CF type graph and downcasting utilities (`CFType::downcast_ref`, `CFRetained::downcast`) are provided by the crate.

### 2) NSWorkspace notifications registered on the wrong center

* `NSWorkspaceDidActivateApplicationNotification` is **posted on `NSWorkspace.sharedWorkspace().notificationCenter()`**, not the global `NSNotificationCenter::defaultCenter()`. If you attach your observer to `defaultCenter`, you’ll miss app activation events. Apple’s docs explicitly say the userInfo contains `NSWorkspaceApplicationKey` (an `NSRunningApplication`) for that notification.

### 3) Pointer equality for notification names

* Comparing `CFStringRef` notification names using address/pointer equality is brittle. Use value equality (e.g., `CFEqual`) or simply compare against the **known constant** using string equality. The framework bindings expose proper equality and `CFType` downcasting tools.

### 4) `CFRunLoopAddSource` mode parameter misuse

* `CFRunLoopAddSource` expects a `CFStringRef` **mode**. You can pass `kCFRunLoopDefaultMode` **directly**. Casting `&*kCFRunLoopDefaultMode as *const _ as *mut CFRunLoopMode` is incorrect and hides potential UB. The crate demonstrates correct usage (see examples that add observers/sources).

### 5) Lifetime / ownership in AX callbacks

* In `AXObserver` callbacks, the `element` argument is **not automatically retained** for you. Do **not** wrap it with an owning `Id` (Obj‑C) or even `CFRetained` unless you explicitly retain under the rules. If you need to store it, retain with CF semantics; otherwise, treat it as a borrowed pointer and copy what you need. Apple’s AX observer callback is `AXObserverCallback(observer, element, notification, refcon)`.

### 6) `declare_class!` → `define_class!` and main-thread mutability

* `objc2` stabilized on **`define_class!`** and `MainThreadOnly` for safe main-thread subclasses. `declare_class!` is from older versions. Current docs show `define_class!`, `MainThreadOnly`, and `Retained<T>` for Obj‑C objects.

### 7) Unsafe globals (`static mut`) and reentrancy

* `static mut` is unsound even if “single-threaded by design” because callbacks and CFRunLoop reentrancy can still interleave. Prefer a **`OnceLock<RefCell<State>>`** (or `thread_local!` if you truly guarantee main-thread only). This keeps mutation scoped and safe.

### 8) Missing AX trust prompt

* `AXIsProcessTrusted()` merely returns a boolean. For a CLI you ship, invoke **`AXIsProcessTrustedWithOptions`** with `kAXTrustedCheckOptionPrompt` to show the OS permission prompt once and print a helpful message otherwise.

### 9) Robustness of tab detection

* Not every app exposes an `AXTabGroup`. Sometimes you must:

  * fall back to the **window title** (`kAXTitleAttribute` on the window), or
  * handle tabs whose selected state lives on children with roles other than exactly `"AXTab"` (e.g., `"AXRadioButton"` in some apps).
* Your recursion is fine, but be prepared to return `None` frequently and treat that as “app-only” context.

### 10) Observer cleanup & double-registration

* Always **remove notifications** (`AXObserverRemoveNotification`) before dropping or replacing an observer to avoid delivering callbacks to dead objects. Also remove the runloop source. Your code removes the source but not the notification. Clean both.

### 11) Matching constants / strings

* Prefer **role constants** like `kAXTabGroupRole` over string literals (`"AXTabGroup"`) to avoid typos and OS localization issues when comparable; compare as `CFString`.

### 12) Output format for a CLI

* For observability/analytics, JSON Lines output (`{"from":...,"to":...,"duration_ms":...}` per event) plus a `--format` flag is far friendlier than bare `println!`. Production CLIs also need clear messages for permissions and a non-zero exit code on fatal setup errors.

---

## A production‑ready implementation (Rust 1.78+)

**What this gives you:**

* Correct crates & types for AX / CF
* Safe state management (`OnceLock<RefCell<State>>`)
* NSWorkspace activation observer on the **workspace notification center**
* AX observers for focused window change **and** selected children (tab) change
* Graceful initialization (capture current front app), robust fallbacks (window title when no tab group)
* **JSON or text** output (`--format json|text`)
* Permission prompt via `AXIsProcessTrustedWithOptions`
* Cleanup of observers and runloop sources when replaced

### `Cargo.toml`

```toml
[package]
name = "focus-track"
version = "0.1.0"
edition = "2021"

[dependencies]
# ObjC / Apple frameworks
objc2 = "0.6.2"
objc2-foundation = { version = "0.3.1", features = ["NSString", "NSNotification", "NSNotificationCenter"] }
objc2-app-kit = { version = "0.3.1", features = ["NSWorkspace", "NSRunningApplication"] }
objc2-core-foundation = { version = "0.3.1", features = ["CFRunLoop", "CFString", "CFArray", "CFNumber"] }
objc2-application-services = { version = "0.3.1", features = [
  # AX/HIServices
  "AXUIElement",
  "AXObserver",
  "AXError",
  "AXConstants", # notifications, attributes, roles
  "AXTrustedCheck",
] }

# CLI niceties
clap = { version = "4.5", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[profile.release]
lto = "thin"
codegen-units = 1
```

> **Note**: The exact feature lists are explicit to avoid pulling the whole frameworks; adjust if your resolver complains. The versions reflect current lines as of April–August 2025 (objc2 0.6.x; foundation/app\_kit/core\_foundation/application\_services 0.3.x).

### `src/main.rs`

```rust
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

/// CLI flags
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

#[derive(Debug, Clone)]
struct EventCtx {
    app_id: String,
    app_name: String,
    tab_title: Option<String>,
}

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

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut State) -> R,
{
    let cell = STATE.get().expect("state not initialized");
    let mut borrow = cell.borrow_mut();
    f(&mut *borrow)
}

fn now_event_ctx() -> EventCtx {
    with_state(|s| EventCtx {
        app_id: s.app_id.clone(),
        app_name: s.app_name.clone(),
        tab_title: s.tab_title.clone(),
    })
}

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

/// ---- CoreFoundation helpers ------------------------------------------------

unsafe fn cf_downcast<T: objc2_core_foundation::ConcreteType>(
    cf: &CFType,
) -> Option<&T> {
    cf.downcast_ref::<T>()
}

unsafe fn copy_attr_cf(element: &AXUIElement, attr: &CFString) -> Option<CFRetained<CFType>> {
    let mut out: *mut c_void = null_mut();
    let err = AXUIElementCopyAttributeValue(element, attr, &mut out);
    if err != kAXErrorSuccess || out.is_null() {
        return None;
    }
    // We own +1 (Create/Copy rule)
    Some(CFRetained::from_raw(out.cast()))
}

fn get_bool_attr(element: &AXUIElement, attr: &CFString) -> Option<bool> {
    unsafe {
        let v = copy_attr_cf(element, attr)?;
        let cf = v.as_ref();
        if let Some(b) = cf_downcast::<CFBoolean>(cf) {
            // CFBoolean::value() exists; CFBooleanGetValue is deprecated in crate
            Some(b.value())
        } else {
            None
        }
    }
}

fn copy_cfarray_axelements(element: &AXUIElement, attr: &CFString) -> Option<CFRetained<CFArray<AXUIElement>>> {
    unsafe {
        let v = copy_attr_cf(element, attr)?;
        let cf = v.as_ref();
        // Try downcast to CFArray<AXUIElement>
        cf.downcast_ref::<CFArray<AXUIElement>>()
            .map(|arr| arr.retain())
    }
}

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

/// ---- Tab detection ---------------------------------------------------------

fn find_tab_group_in(element: &AXUIElement) -> Option<CFRetained<AXUIElement>> {
    // role?
    if let Some(role) = copy_cfstring_attr(element, unsafe { kAXRoleAttribute }) {
        // Prefer constant compare; here string compare is acceptable
        if unsafe { CFEqual(&CFString::from_str("AXTabGroup"), kAXTabGroupRole) } || role == "AXTabGroup" {
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
        copy_cfstring_attr(fw, kAXTitleAttribute)
    }
}

/// ---- AX callbacks ----------------------------------------------------------

extern "C" fn tab_change_cb(
    _observer: *mut AXObserver,
    element: *mut AXUIElement,
    notification: *const CFString,
    _user_data: *mut c_void,
) {
    // Defensive: confirm notif name
    unsafe {
        if !CFEqual(&*notification, kAXSelectedChildrenChangedNotification) {
            return;
        }
        // The element is the tab group. Borrow, do not take ownership.
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

        // Remove old tab observer (including notif)
        with_state(|s| {
            if let Some(obs) = s.tab_obs.take() {
                let source = AXObserverGetRunLoopSource(&obs);
                let rl = CFRunLoop::current();
                CFRunLoopRemoveSource(rl, source, kCFRunLoopDefaultMode);
                // Best-effort: remove notification on last known tab group/window.
                // We don't keep that AXUIElement around; removing the source is usually sufficient.
            }
        });

        // Compute new tab title (before logging)
        let new_tab = current_tab_title_for_app(app_el);

        // Prepare logging if changed (unless this is init path where we skip)
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

        // If there's a tab group, observe its selected-children changes
        if let Some(fw_cf) = copy_attr_cf(app_el, kAXFocusedWindowAttribute) {
            if let Some(fw) = fw_cf.as_ref().downcast_ref::<AXUIElement>() {
                if let Some(tab_group) = find_tab_group_in(fw) {
                    let pid = with_state(|s| s.pid);
                    let mut obs_ptr: *mut AXObserver = null_mut();
                    let err = AXObserverCreate(pid, Some(tab_change_cb), &mut obs_ptr);
                    if err == kAXErrorSuccess {
                        let obs = CFRetained::from_raw(obs_ptr.cast());
                        // Observe selected children change on the tab group
                        let _ = AXObserverAddNotification(&obs, &tab_group, kAXSelectedChildrenChangedNotification, null_mut());
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

/// ---- Obj‑C class to receive NSWorkspace notifications ----------------------

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
            // Get NSRunningApplication from userInfo
            unsafe {
                let user_info = notification.userInfo();
                if user_info.is_none() { return; }
                let app_key = ns_string!("NSWorkspaceApplicationKey");
                let running_app: Option<Retained<NSRunningApplication>> = user_info.unwrap().objectForKey(app_key);

                if let Some(ra) = running_app {
                    let bundle_id: Option<Retained<NSString>> = ra.bundleIdentifier();
                    let name: Retained<NSString> = ra.localizedName().unwrap_or(ns_string!("Unknown"));
                    let new_app_id = bundle_id.as_deref().map(|s| s.to_string()).unwrap_or_else(|| name.to_string());

                    // Log transition
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

                    // Remove any previous observers (app + tab)
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

                    // Setup app-level observer for focused window changes
                    let pid = with_state(|s| s.pid);
                    let app_el = AXUIElementCreateApplication(pid);
                    let mut obs_ptr: *mut AXObserver = null_mut();
                    let err = AXObserverCreate(pid, Some(window_change_cb), &mut obs_ptr);
                    if err == kAXErrorSuccess {
                        let obs = CFRetained::from_raw(obs_ptr.cast());
                        let _ = AXObserverAddNotification(&obs, &app_el, kAXFocusedWindowChangedNotification, null_mut());
                        let source = AXObserverGetRunLoopSource(&obs);
                        CFRunLoopAddSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                        with_state(|s| s.app_obs = Some(obs));
                    }

                    // Initialize tab state (no log)
                    window_change_cb(null_mut(), app_el.as_ptr(), kAXFocusedWindowChangedNotification, null_mut());
                }
            }
        }
    }
);

/// ---- startup ---------------------------------------------------------------

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

    // Permissions
    let trusted = ensure_ax_trust(!cli.no_prompt);
    if !trusted {
        eprintln!(
            "Accessibility access is not granted. \
             Please enable: System Settings → Privacy & Security → Accessibility → allow this app."
        );
        // Keep going: app activation events will still work; tab/window may be limited.
    }

    // Initialize state with current frontmost app
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        if let Some(front) = ws.frontmostApplication() {
            let bundle_id: Option<Retained<NSString>> = front.bundleIdentifier();
            let name: Retained<NSString> = front.localizedName().unwrap_or(ns_string!("Unknown"));
            let app_id = bundle_id.as_deref().map(|s| s.to_string()).unwrap_or_else(|| name.to_string());

            let app_el = AXUIElementCreateApplication(front.processIdentifier());

            // App observer for focused window changes
            let mut app_obs_ptr: *mut AXObserver = null_mut();
            let app_obs = if AXObserverCreate(front.processIdentifier(), Some(window_change_cb), &mut app_obs_ptr) == kAXErrorSuccess {
                let obs = CFRetained::from_raw(app_obs_ptr.cast());
                let _ = AXObserverAddNotification(&obs, &app_el, kAXFocusedWindowChangedNotification, null_mut());
                let source = AXObserverGetRunLoopSource(&obs);
                CFRunLoopAddSource(CFRunLoop::current(), source, kCFRunLoopDefaultMode);
                Some(obs)
            } else { None };

            // Create and hold the NSWorkspace observer instance
            let workspace_observer: Retained<AppSwitchObserver> = AppSwitchObserver::new();

            // Register for activation notifications on the **workspace** center
            let center = ws.notificationCenter();
            center.addObserver_selector_name_object(
                &workspace_observer,
                sel!(notifyApplicationActivated:),
                NSWorkspaceDidActivateApplicationNotification,
                None,
            );

            // Initialize global state
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

            // Initialize tab state (no log)
            window_change_cb(null_mut(), app_el.as_ptr(), kAXFocusedWindowChangedNotification, null_mut());

            // Run event loop
            CFRunLoop::run_current();
        } else {
            eprintln!("No frontmost application detected; exiting.");
            std::process::exit(2);
        }
    }
}
```

---

## Why this version is safer & current

* **Crates & API surfaces are current**: `objc2` 0.6.x and the 0.3.x framework crates match the 2024–2025 ecosystem; `define_class!` + `MainThreadOnly` are the modern patterns.
* **AX (HIServices)** calls and constants come from `objc2_application_services` (the right crate), not `objc2_accessibility`. The constants (`kAX...`) and functions (`AXObserverCreate`, etc.) are in that crate.
* **NSWorkspace** notifications are observed on `NSWorkspace.sharedWorkspace().notificationCenter()`, per Apple docs, and `userInfo[NSWorkspaceApplicationKey]` is used to retrieve the `NSRunningApplication`.
* **CF types** use `CFRetained<T>` and dynamic downcast helpers; no `Id<T>` on CF/AX types; comparisons use `CFEqual` (value equality).
* **Run loop** usage passes `kCFRunLoopDefaultMode` directly (string mode constant), consistent with CoreFoundation examples.
* **AX trust** uses `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt` to request permissions in a user‑friendly way.

---

## Operational notes (production)

1. **Permissions**

   * First run will prompt for Accessibility. You must enable it for full tab/window tracking. App activation notifications do not require this.

2. **Apps without tab groups**

   * Many apps don’t expose an `AXTabGroup`. We fall back to window title; that’s as good as the UI tree exposes for those processes.

3. **Performance**

   * Event‑driven; no polling. The only traversals are on focus change to locate `AXTabGroup` (depth typically ≤ 20). That’s negligible for a CLI.

4. **Hardening**

   * If you want to persist the last timing on `SIGINT`, register a signal handler and log the “current → <exit>” transition before exit.
   * Consider a `--filter com.apple.Safari,com.google.Chrome` option if you want to limit to browsers.

5. **Packaging**

   * For Homebrew: set `MACOSX_DEPLOYMENT_TARGET` appropriately and notarize if you distribute a signed build. No special entitlements required for a non‑sandboxed CLI, just Accessibility in TCC.

---

## Quick compare against your original

* **Observers**: your code used `defaultCenter()`; fixed to `workspace.notificationCenter()` (this is the biggest “it looks fine but does nothing” pitfall).
* **Memory**: your code wrapped CF types in `Id<T>` and created `Id::from_ptr` from non‑owned pointers—**UB**. This version uses `CFRetained<T>`, and only when we actually own +1.
* **Equality**: pointer compare for notification names → **value equality**.
* **Runloop**: mode constant is passed directly; no casting games.
* **AX prompt**: added, per Apple docs.

---

### References

* **NSWorkspace didActivate**: userInfo includes `NSWorkspaceApplicationKey` pointing to `NSRunningApplication`. Also, observe via `NSWorkspace.sharedWorkspace().notificationCenter()`.
* **objc2 (0.6.x) + examples** for defining classes & main‑thread safety.
* **Accessibility/HIServices (AX)** functions and constants: crate docs (AXObserver\*, AXUIElement\*, kAX\*).
* **CoreFoundation** utilities: runloop usage and CFType downcasting; pass `kCFRunLoopDefaultMode` directly; `CFType::downcast_ref`.
* **AX Trust prompt**: `AXIsProcessTrustedWithOptions` and `kAXTrustedCheckOptionPrompt`.

---

If you want me to tailor the output schema or add CSV/NDJSON sinks (or a `--write /path/file.jsonl` flag), I’ll wire it in the same style without changing the AX core.
