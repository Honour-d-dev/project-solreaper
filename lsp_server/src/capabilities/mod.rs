pub mod definition;
pub mod hover;

use la_arena::Arena;

use crate::hir::body_map::Local;
use crate::hir::item_data::{EnumData, Field};

/// Shared semantic context for capabilities (hover, go-to-definition, etc.).
/// Carries the owner data needed to resolve `SemanticId`s that are local to a
/// specific item (locals, struct fields, enum variants).
pub struct SemanticCtx<'a> {
    pub locals: Option<&'a Arena<Local>>,
    pub fields: Option<&'a Arena<Field>>,
    pub enum_data: Option<&'a EnumData>,
}

impl<'a> SemanticCtx<'a> {
    pub fn empty() -> Self {
        SemanticCtx { locals: None, fields: None, enum_data: None }
    }

    pub fn local(locals: &'a Arena<Local>) -> Self {
        SemanticCtx { locals: Some(locals), fields: None, enum_data: None }
    }

    pub fn field(fields: &'a Arena<Field>) -> Self {
        SemanticCtx { locals: None, fields: Some(fields), enum_data: None }
    }

    pub fn variant(enum_data: &'a EnumData) -> Self {
        SemanticCtx { locals: None, fields: None, enum_data: Some(enum_data) }
    }
}