use triomphe::Arc;

use crate::ast::{self, ToAstNode};
use crate::ast::kinds::NodeKind;
use crate::hir::body_map::{BodyBuilder, BodyMap, BodyOwnerId, BodySourceMap};
use crate::hir::item_data::{ContractData, EnumData, ErrorData, EventData, FunctionData, ImportData, InterfaceData, ItemBuilder, LibraryData, ModifierData, StructData, VarData};
use crate::hir::resolver::{Context, Resolver};
use crate::ir::def_map::DefId;
use crate::salsa::interned_db::Id;
use super::db::SalsaDatabase;
use super::interned_db::{BodyOwner, Contract, DefWithBases, DefWithBasesId, Enum, Error, Event, Function, Import, Interface, Library, Modifier, Struct, Var};
use super::root_db::RootDatabase;

#[salsa::tracked]
pub fn body_map<'db>(db: &'db dyn RootDatabase, owner: BodyOwner<'db>) -> (Arc<BodyMap>, Arc<BodySourceMap>) {

    let (owner, ast_root)  = match owner.id(db) {
        BodyOwnerId::Function(id) => {
            let ast_root = db.ast(id.file).root();
            let ast_id_map = db.ast_id_map(id.file);
            (ast_id_map.get(&ast_root, id.id).unwrap().to_node(), ast_root)
        },
        BodyOwnerId::Modifier(id) => {
            let ast_root = db.ast(id.file).root();
            let ast_id_map = db.ast_id_map(id.file);
            (ast_id_map.get(&ast_root, id.id).unwrap().to_node(), ast_root)
        },
    };
    let (body, source_map) = BodyBuilder::build(ast_root, owner).unwrap();
    (body.into(), source_map.into())
}

#[salsa::tracked]
pub fn function_data<'db>(db: &'db dyn RootDatabase, f: Function<'db>) -> Arc<FunctionData> {
    let id = f.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let func = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_fn(&func))
}

#[salsa::tracked]
pub fn struct_data<'db>(db: &'db dyn RootDatabase, s: Struct<'db>) -> Arc<StructData> {
    let id = s.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let strukt = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_struct(&strukt))
}

#[salsa::tracked]
pub fn var_data<'db>(db: &'db dyn RootDatabase, v: Var<'db>) -> Arc<VarData> {
    let id = v.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let var = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_var(&var))
}

#[salsa::tracked]
pub fn enum_data<'db>(db: &'db dyn RootDatabase, e: Enum<'db>) -> Arc<EnumData> {
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let enom = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_enum(&enom))
}

#[salsa::tracked]
pub fn event_data<'db>(db: &'db dyn RootDatabase, e: Event<'db>) -> Arc<EventData> {
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let event = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_event(&event))
}

#[salsa::tracked]
pub fn error_data<'db>(db: &'db dyn RootDatabase, e: Error<'db>) -> Arc<ErrorData> {
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let error = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_error(&error))
}

#[salsa::tracked]
pub fn modifier_data<'db>(db: &'db dyn RootDatabase, m: Modifier<'db>) -> Arc<ModifierData> {
    let id = m.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let modifier = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_modifier(&modifier))
}

#[salsa::tracked]
pub fn contract_data<'db>(db: &'db dyn RootDatabase, c: Contract<'db>) -> Arc<ContractData> {
    let id = c.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let contract = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_contract(&contract))
}

#[salsa::tracked]
pub fn interface_data<'db>(db: &'db dyn RootDatabase, i: Interface<'db>) -> Arc<InterfaceData> {
    let id = i.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let interface = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_interface(&interface))
}

#[salsa::tracked]
pub fn library_data<'db>(db: &'db dyn RootDatabase, l: Library<'db>) -> Arc<LibraryData> {
    let id = l.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let library = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_library(&library))
}

#[salsa::tracked]
pub fn import_data<'db>(db: &'db dyn RootDatabase, i: Import<'db>) -> Arc<ImportData> {
    let id = i.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let import = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_import(&import))
}

#[salsa::tracked]
pub fn bases<'db>(db: &'db dyn HirDatabase, def: DefWithBases<'db>) -> Vec<DefId> {
    let id = def.id(db);
    let ctx = match id {
        DefWithBasesId::Contract(c) => {
            Context {
                file: c.file,
                offset: 0,
                container: DefId::Contract(c),
            }
        }
        DefWithBasesId::Interface(i) => {
            Context {
                file: i.file,
                offset: 0,
                container: DefId::Interface(i),
            }
        }
    };
    Resolver::build(db, &ctx).c3_linearize()
}

#[salsa::tracked(lru=100)]
pub fn docs<'db>(db: &'db dyn RootDatabase, def: Id<'db>) -> Option<String> {//TODO make tracked when we move ids into salsa
    let (file, erased_id) = def.id(db).ast_id()?;
    let ast_id_map = db.ast_id_map(file);
    let root = db.root(file);
    let node = ast_id_map.get_node(&root, erased_id)?;
    
    let mut comments = Vec::new();
    let mut current = node.node().prev_named_sibling();
    while let Some(sibling) = current {
        if sibling.kind_id() != NodeKind::COMMENT {
            break;
        }
        let text = root.text_by_range(sibling.byte_range());
        comments.push(text.to_string());
        current = sibling.prev_named_sibling();
    }
    
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join("\n"))
}

#[salsa::db]
pub trait HirDatabase: RootDatabase {
    fn body_map(&self, owner: BodyOwnerId) -> Arc<BodyMap>;
    fn body_source_map(&self, owner: BodyOwnerId) -> Arc<BodySourceMap>;
    fn body_and_source_map(&self, owner: BodyOwnerId) -> (Arc<BodyMap>, Arc<BodySourceMap>);
    fn function_data(&self, id: ast::FunctionId) -> Arc<FunctionData>;
    fn struct_data(&self, id: ast::StructId) -> Arc<StructData>;
    fn enum_data(&self, id: ast::EnumId) -> Arc<EnumData>;
    fn var_data(&self, id: ast::VarId) -> Arc<VarData>;
    fn error_data(&self, id: ast::ErrorId) -> Arc<ErrorData>;
    fn modifier_data(&self, id: ast::ModifierId) -> Arc<ModifierData>;
    fn event_data(&self, id: ast::EventId) -> Arc<EventData>;
    fn contract_data(&self, id: ast::ContractId) -> Arc<ContractData>;
    fn interface_data(&self, id: ast::InterfaceId) -> Arc<InterfaceData>;
    fn library_data(&self, id: ast::LibraryId) -> Arc<LibraryData>;
    fn import_data(&self, id: ast::ImportId) -> Arc<ImportData>;
    fn bases(&self, def: DefId) -> Vec<DefId>;
    fn docs(&self, def: DefId) -> Option<String>;
}

#[salsa::db]
impl HirDatabase for SalsaDatabase {
    fn body_map(&self, owner: BodyOwnerId) -> Arc<BodyMap> {
        body_map(self, BodyOwner::new(self, owner)).0
    }

    fn body_source_map(&self, owner: BodyOwnerId) -> Arc<BodySourceMap> {
        body_map(self, BodyOwner::new(self, owner)).1
    }

    fn body_and_source_map(&self, owner: BodyOwnerId) -> (Arc<BodyMap>, Arc<BodySourceMap>) {
        body_map(self, BodyOwner::new(self, owner))
    }

    fn function_data(&self, id: ast::FunctionId) -> Arc<FunctionData> {
        function_data(self, Function::new(self, id))
    }

    fn struct_data(&self, id: ast::StructId) -> Arc<StructData> {
        struct_data(self, Struct::new(self, id))
    }

    fn enum_data(&self, id: ast::EnumId) -> Arc<EnumData> {
        enum_data(self, Enum::new(self, id))
    }

    fn var_data(&self, id: ast::VarId) -> Arc<VarData> {
        var_data(self, Var::new(self, id))
    }

    fn error_data(&self, id: ast::ErrorId) -> Arc<ErrorData> {
        error_data(self, Error::new(self, id))
    }

    fn modifier_data(&self, id: ast::ModifierId) -> Arc<ModifierData> {
        modifier_data(self, Modifier::new(self, id))
    }

    fn event_data(&self, id: ast::EventId) -> Arc<EventData> {
        event_data(self, Event::new(self, id))
    }

    fn contract_data(&self, id: ast::ContractId) -> Arc<ContractData> {
        contract_data(self, Contract::new(self, id))
    }

    fn interface_data(&self, id: ast::InterfaceId) -> Arc<InterfaceData> {
        interface_data(self, Interface::new(self, id))
    }

    fn library_data(&self, id: ast::LibraryId) -> Arc<LibraryData> {
        library_data(self, Library::new(self, id))
    }

    fn import_data(&self, id: ast::ImportId) -> Arc<ImportData> {
        import_data(self, Import::new(self, id))
    }

    fn bases(&self, def: DefId) -> Vec<DefId> {
        let def = match def {
            DefId::Contract(c) => DefWithBases::new(self, DefWithBasesId::Contract(c)),
            DefId::Interface(i) => DefWithBases::new(self, DefWithBasesId::Interface(i)),
            _ => return vec![],
        };

        bases(self, def)
    }

    fn docs(&self, def: DefId) -> Option<String> {
        docs(self, Id::new(self, def))
    }
}
