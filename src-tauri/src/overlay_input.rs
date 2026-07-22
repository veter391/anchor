//! Per-region click-through for the overlay window.
//!
//! Tauri has no native per-region hit-testing (tauri#2090), so a Rust loop
//! polls the cursor at ~80 Hz and toggles `set_ignore_cursor_events`
//! depending on whether the cursor is over a zone the webview reported as
//! interactive. Measured in the Phase-0 spike: 10–44 µs per toggle, no
//! flicker at zone borders.
//!
//! Unit contract: the webview reports zones in CSS (logical) pixels relative
//! to the viewport. The loop converts the cursor's physical screen position
//! into the same space each tick (fresh `outer_position` + `scale_factor`,
//! frameless window ⇒ outer origin == viewport origin), so window drags and
//! DPI scaling need no re-reporting from the webview.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, WebviewWindow};

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Zone {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Default)]
pub struct Zones(Mutex<Vec<Zone>>);

/// Poison-tolerant lock: a panic elsewhere must never freeze click-through.
fn lock_zones(zones: &Zones) -> std::sync::MutexGuard<'_, Vec<Zone>> {
    zones.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
pub fn set_interactive_zones(state: tauri::State<'_, Zones>, zones: Vec<Zone>) {
    *lock_zones(&state) = zones;
}

pub fn spawn_poll_loop(window: WebviewWindow) {
    window.set_ignore_cursor_events(true).ok();
    std::thread::spawn(move || {
        let mut ignoring = true;
        loop {
            std::thread::sleep(Duration::from_millis(12));
            let Ok(cursor) = window.cursor_position() else { continue };
            let Ok(origin) = window.outer_position() else { continue };
            let Ok(scale) = window.scale_factor() else { continue };

            let rel_x = (cursor.x - origin.x as f64) / scale;
            let rel_y = (cursor.y - origin.y as f64) / scale;

            let inside = {
                let zones = window.state::<Zones>();
                let zones = lock_zones(&zones);
                zones.iter().any(|z| {
                    rel_x >= z.x && rel_x <= z.x + z.w && rel_y >= z.y && rel_y <= z.y + z.h
                })
            };

            let desired_ignore = !inside;
            if desired_ignore != ignoring && window.set_ignore_cursor_events(desired_ignore).is_ok()
            {
                ignoring = desired_ignore;
            }
        }
    });
}
