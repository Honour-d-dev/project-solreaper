
use crate::ast;
use crate::hir::body_map::BodyOwnerId;

#[derive(PartialEq, Eq, Clone, Hash)]
pub enum DefWithBasesId {
    Contract(ast::ContractId),
    Interface(ast::InterfaceId),
}

#[salsa::interned]
pub struct DefWithBases<'db> {
    pub id: DefWithBasesId
}

#[salsa::interned]
pub struct BodyOwner<'db> {
    pub id: BodyOwnerId
}

#[salsa::interned]
pub struct Contract<'db> {
    pub id: ast::ContractId
}

#[salsa::interned]
pub struct Function<'db> {
    pub id: ast::FunctionId
}

#[salsa::interned]
pub struct Modifier<'db> {
    pub id: ast::ModifierId
}

#[salsa::interned]
pub struct Struct<'db> {
    pub id: ast::StructId
}

#[salsa::interned]
pub struct Event<'db> {
    pub id: ast::EventId
}

#[salsa::interned]
pub struct Enum<'db> {
    pub id: ast::EnumId
}

#[salsa::interned]
pub struct Error<'db> {
    pub id: ast::ErrorId
}

#[salsa::interned]
pub struct Var<'db> {
    pub id: ast::VarId
}








