//! Campaign-driven content: schema, loader, registry, expressions and checks.
//!
//! The engine and assets stay generic; everything campaign-specific (conditions,
//! masteries, abilities, creatures, AI templates, scenes and encounters) is data
//! loaded from `LetsPlay/Rules/combat`, adventure `combat/` directories, character sheets
//! and a campaign manifest.

pub mod check;
pub mod expr;
pub mod full_pack;
pub mod loader;
pub mod pack;
pub mod schema;

pub use check::{check, CheckReport, Issue, Severity};
pub use expr::{evaluate, validate_template, ExprContext};
pub use loader::{
    AnyItem, ContentRegistry, ContentSource, ContentSources, ItemKind, ItemStore, ResolvedAbility,
    ShownItem, Sourced,
};
pub use pack::{ContentPack, Distribution, PackEntry, PackImage, PackIndex, PackMeta};
