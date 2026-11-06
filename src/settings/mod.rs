//! Headless agent settings: schema-driven fields, the editable draft and the
//! presentation catalog rendered by `ui::settings`.

pub mod addons;
pub mod catalog;
mod draft;
pub mod history;
pub mod mcp;
pub mod reconcile;
mod schema;

pub use draft::{Draft, FieldPath};
pub use schema::{FieldKind, TextFormat, choice_title, field_kind, format_value, parse_text};
pub use schema::{field_sections, section_scope};

#[cfg(test)]
mod tests;
