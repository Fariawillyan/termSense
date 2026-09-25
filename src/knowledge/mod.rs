//! Local knowledge base: model, loading and read-only access.

pub mod loader;
pub mod model;
pub mod repository;
pub mod template;

pub use model::{ArgKind, Argument, CommandOption, Entry, EntryKind, Example};
pub use repository::Repository;
pub use template::Vars;
