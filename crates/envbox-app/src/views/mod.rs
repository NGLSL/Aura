//! Iced views for the dark three-column shell.
//! State lives in `crate::app`; this module only renders.

mod apps;
mod audit;
mod close_dialog;
mod detail;
mod instances;
mod nav;
mod picker;
mod profiles;
mod settings;
mod shell;
mod unsaved_dialog;
mod workspaces;

pub use shell::view;
