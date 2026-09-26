//! Single source of truth for product/engine version labels in the GUI.
//! Version comes from the workspace crate (`workspace.package.version`).

/// Product mark shown in chrome (sidebar, settings, title).
pub const PRODUCT_NAME: &str = "Aura";

/// Marketing version — keep every UI surface on this string.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Engine / core name (EnvBox process-level virtualization).
pub const ENGINE_NAME: &str = "EnvBox Core";

/// `Aura 0.3.0`
pub fn product_label() -> String {
    format!("{PRODUCT_NAME} {PRODUCT_VERSION}")
}

/// `EnvBox Core · 0.3.0`
pub fn engine_label() -> String {
    format!("{ENGINE_NAME} · {PRODUCT_VERSION}")
}
