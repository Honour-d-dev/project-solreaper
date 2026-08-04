
//@TODO unify the ids. might use this directly in the defmaps
use crate::ast;
use crate::hir::body_map::BodyOwnerId;
use crate::ir::def_map::DefId;

#[salsa::interned]
pub struct Import<'db> {
    pub id: ast::ImportId
}

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
pub struct Interface<'db> {
    pub id: ast::InterfaceId
}

#[salsa::interned]
pub struct Library<'db> {
    pub id: ast::LibraryId
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

#[salsa::interned]
pub struct Udvt<'db> {
    pub id: ast::UdvtId
}

#[salsa::interned]
pub struct Using<'db> {
    pub id: ast::UsingId
}


#[salsa::interned]
pub struct Id {
    pub id: DefId
}

// pub enum DefId {
//     Var(Var),
//     Error(Error),
//     Enum(Enum),
//     Event(Event),
//     Struct(Struct),
//     Modifier(Modifier)
// }




