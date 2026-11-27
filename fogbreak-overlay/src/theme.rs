#[cfg(target_arch = "wasm32")]
use egui::{Color32, Context, Stroke, Visuals};

#[cfg(target_arch = "wasm32")]
pub const LIGHT_BG: Color32 = Color32::from_rgb(0xF7, 0xFA, 0xFC);
#[cfg(target_arch = "wasm32")]
pub const LIGHT_PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
#[cfg(target_arch = "wasm32")]
pub const LIGHT_EXTREME: Color32 = Color32::from_rgb(0xEF, 0xF5, 0xF8);
#[cfg(target_arch = "wasm32")]
pub const LIGHT_FAINT: Color32 = Color32::from_rgb(0xE6, 0xEF, 0xF5);
#[cfg(target_arch = "wasm32")]
pub const LIGHT_STROKE: Color32 = Color32::from_rgb(0xD9, 0xE2, 0xEC);
#[cfg(target_arch = "wasm32")]
pub const LIGHT_TEXT: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);

#[cfg(target_arch = "wasm32")]
pub const DARK_BG: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
#[cfg(target_arch = "wasm32")]
pub const DARK_PANEL: Color32 = Color32::from_rgb(0x1A, 0x24, 0x31);
#[cfg(target_arch = "wasm32")]
pub const DARK_EXTREME: Color32 = Color32::from_rgb(0x0B, 0x10, 0x17);
#[cfg(target_arch = "wasm32")]
pub const DARK_FAINT: Color32 = Color32::from_rgb(0x22, 0x2E, 0x3E);
#[cfg(target_arch = "wasm32")]
pub const DARK_STROKE: Color32 = Color32::from_rgb(0x2B, 0x39, 0x4A);
#[cfg(target_arch = "wasm32")]
pub const DARK_TEXT: Color32 = Color32::from_rgb(0xF7, 0xFA, 0xFC);

#[cfg(target_arch = "wasm32")]
pub const TEAL_600: Color32 = Color32::from_rgb(0x08, 0x7F, 0x9C);
#[cfg(target_arch = "wasm32")]
pub const TEAL_400: Color32 = Color32::from_rgb(0x14, 0xB8, 0xC4);
#[cfg(target_arch = "wasm32")]
pub const AMBER_400: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
#[cfg(target_arch = "wasm32")]
pub const CORAL_500: Color32 = Color32::from_rgb(0xEF, 0x44, 0x44);

#[cfg(target_arch = "wasm32")]
pub fn initial_dark_mode() -> bool {
    let Some(win) = web_sys::window() else { return false };
    if let Ok(search) = win.location().search()
        && let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search)
        && let Some(t) = params.get("theme")
    {
        if t.eq_ignore_ascii_case("dark") {
            return true;
        }
        if t.eq_ignore_ascii_case("light") {
            return false;
        }
    }
    win.match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .is_some_and(|m| m.matches())
}

#[cfg(target_arch = "wasm32")]
pub fn apply_palette(ctx: &Context, dark: bool) {
    let mut visuals = if dark { Visuals::dark() } else { Visuals::light() };
    let (bg, panel, extreme, faint, stroke, text) = if dark {
        (DARK_BG, DARK_PANEL, DARK_EXTREME, DARK_FAINT, DARK_STROKE, DARK_TEXT)
    } else {
        (LIGHT_BG, LIGHT_PANEL, LIGHT_EXTREME, LIGHT_FAINT, LIGHT_STROKE, LIGHT_TEXT)
    };
    let accent = if dark { TEAL_400 } else { TEAL_600 };

    visuals.window_fill = bg;
    visuals.panel_fill = panel;
    visuals.extreme_bg_color = extreme;
    visuals.faint_bg_color = faint;
    visuals.window_stroke = Stroke::new(1.0, stroke);
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.hyperlink_color = accent;
    visuals.warn_fg_color = AMBER_400;
    visuals.error_fg_color = CORAL_500;
    visuals.override_text_color = Some(text);
    visuals.widgets.noninteractive.bg_fill = panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, stroke);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);
    visuals.widgets.inactive.bg_fill = faint;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, stroke);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    visuals.widgets.hovered.bg_fill = faint;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);

    ctx.set_visuals(visuals);
}

#[cfg(target_arch = "wasm32")]
pub fn sync_document_theme(dark: bool) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    if let Some(root) = doc.document_element() {
        let _ = root.set_attribute("data-theme", if dark { "dark" } else { "light" });
    }
    if let Some(body) = doc.body() {
        let bg = if dark { "#111821" } else { "#F7FAFC" };
        let fg = if dark { "#F7FAFC" } else { "#111821" };
        let style = body.style();
        let _ = style.set_property("background", bg);
        let _ = style.set_property("color", fg);
    }
}
