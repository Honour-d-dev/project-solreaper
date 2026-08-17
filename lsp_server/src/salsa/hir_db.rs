use la_arena::Idx;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use triomphe::Arc;

use crate::ast::{self, AstNode, NodeRange, ToAstNode};
use crate::hir::types::{Type, TypeKey};
use crate::salsa::File;
use crate::ast::kinds::NodeKind;
use crate::hir::body_map::{BodyBuilder, BodyMap, BodyOwnerId, BodySourceMap, Local};
use crate::hir::item_data::{ContractData, EnumData, ErrorData, ErrorSignature, EventData, EventSignature, FunctionData, FunctionSignature, ImportData, InterfaceData, ItemBuilder, LibraryData, ModifierData, ModifierSignature, StructData, UdvtData, UsingData, VarData};
use crate::hir::exprs::Name;
use crate::hir::resolver::{Context, Resolution, Resolver};
use crate::ir::def_map::{DefId, DefMap, Namespace, Scope};
use crate::salsa::interned_db::Id;
use super::db::SalsaDatabase;
use super::interned_db::{BodyOwner, Contract, DefWithBases, DefWithBasesId, Enum, Error, Event, Function, Import, Interface, Library, Modifier, Struct, Udvt, Using, Var};
use super::root_db::RootDatabase;

#[salsa::tracked]
pub fn body_map<'db>(db: &'db dyn RootDatabase, owner: BodyOwner<'db>) -> (Arc<BodyMap>, Arc<BodySourceMap>) {
    tracing::debug!(?owner, "body_map: cache miss");

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
    tracing::debug!(?f, "function_data: cache miss");
    let id = f.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let func = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_fn(&func))
}

fn lower_signature_params(
    resolver: &Resolver,
    parameters: &la_arena::Arena<Local>,
    store: &crate::hir::item_data::ExprStore,
) -> Option<Box<[TypeKey]>> {
    parameters.iter()
        .map(|(_, local)| {
            resolver.lower_type_name(*local.type_name(), store)
                .map(|ty| ty.upcast_from(local.location()))
        })
        .collect::<Option<Vec<_>>>()
        .map(Vec::into_boxed_slice)
}

#[salsa::tracked]
pub fn function_signature<'db>(db: &'db dyn HirDatabase, f: Function<'db>) -> Option<Arc<FunctionSignature>> {
    let id = f.id(db);
    let data = function_data(db, f);
    let resolver = Resolver::build(db, &Context { file: id.file, container: DefId::Function(id) });
    Some(Arc::new(FunctionSignature {
        params: lower_signature_params(&resolver, &data.parameters, &data.expr_store)?,
        returns: lower_signature_params(&resolver, &data.return_parameters, &data.expr_store)?,
    }))
}

#[salsa::tracked]
pub fn event_signature<'db>(db: &'db dyn HirDatabase, e: Event<'db>) -> Option<Arc<EventSignature>> {
    let id = e.id(db);
    let data = event_data(db, e);
    let resolver = Resolver::build(db, &Context { file: id.file, container: DefId::Event(id) });
    Some(Arc::new(EventSignature {
        params: lower_signature_params(&resolver, &data.parameters, &data.expr_store)?,
    }))
}

#[salsa::tracked]
pub fn error_signature<'db>(db: &'db dyn HirDatabase, e: Error<'db>) -> Option<Arc<ErrorSignature>> {
    let id = e.id(db);
    let data = error_data(db, e);
    let resolver = Resolver::build(db, &Context { file: id.file, container: DefId::Error(id) });
    Some(Arc::new(ErrorSignature {
        params: lower_signature_params(&resolver, &data.parameters, &data.expr_store)?,
    }))
}

#[salsa::tracked]
pub fn modifier_signature<'db>(db: &'db dyn HirDatabase, m: Modifier<'db>) -> Option<Arc<ModifierSignature>> {
    let id = m.id(db);
    let data = modifier_data(db, m);
    let resolver = Resolver::build(db, &Context { file: id.file, container: DefId::Modifier(id) });
    Some(Arc::new(ModifierSignature {
        params: lower_signature_params(&resolver, &data.parameters, &data.expr_store)?,
    }))
}

#[salsa::tracked]
pub fn struct_data<'db>(db: &'db dyn RootDatabase, s: Struct<'db>) -> Arc<StructData> {
    tracing::debug!(?s, "struct_data: cache miss");
    let id = s.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let strukt = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_struct(&strukt))
}

#[salsa::tracked]
pub fn var_data<'db>(db: &'db dyn RootDatabase, v: Var<'db>) -> Arc<VarData> {
    tracing::debug!(?v, "var_data: cache miss");
    let id = v.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let var = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_var(&var))
}

#[salsa::tracked]
pub fn enum_data<'db>(db: &'db dyn RootDatabase, e: Enum<'db>) -> Arc<EnumData> {
    tracing::debug!(?e, "enum_data: cache miss");
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let enom = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_enum(&enom))
}

#[salsa::tracked]
pub fn event_data<'db>(db: &'db dyn RootDatabase, e: Event<'db>) -> Arc<EventData> {
    tracing::debug!(?e, "event_data: cache miss");
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let event = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_event(&event))
}

#[salsa::tracked]
pub fn error_data<'db>(db: &'db dyn RootDatabase, e: Error<'db>) -> Arc<ErrorData> {
    tracing::debug!(?e, "error_data: cache miss");
    let id = e.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let error = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_error(&error))
}

#[salsa::tracked]
pub fn modifier_data<'db>(db: &'db dyn RootDatabase, m: Modifier<'db>) -> Arc<ModifierData> {
    tracing::debug!(?m, "modifier_data: cache miss");
    let id = m.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let modifier = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_modifier(&modifier))
}

#[salsa::tracked]
pub fn contract_data<'db>(db: &'db dyn RootDatabase, c: Contract<'db>) -> Arc<ContractData> {
    tracing::debug!(?c, "contract_data: cache miss");
    let id = c.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let contract = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_contract(&contract))
}

#[salsa::tracked]
pub fn interface_data<'db>(db: &'db dyn RootDatabase, i: Interface<'db>) -> Arc<InterfaceData> {
    tracing::debug!(?i, "interface_data: cache miss");
    let id = i.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let interface = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_interface(&interface))
}

#[salsa::tracked]
pub fn library_data<'db>(db: &'db dyn RootDatabase, l: Library<'db>) -> Arc<LibraryData> {
    tracing::debug!(?l, "library_data: cache miss");
    let id = l.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let library = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_library(&library))
}

#[salsa::tracked]
pub fn import_data<'db>(db: &'db dyn RootDatabase, i: Import<'db>) -> Arc<ImportData> {
    tracing::debug!(?i, "import_data: cache miss");
    let id = i.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let import = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_import(&import))
}

#[salsa::tracked]
pub fn udvt_data<'db>(db: &'db dyn RootDatabase, u: Udvt<'db>) -> Arc<UdvtData> {
    tracing::debug!(?u, "udvt_data: cache miss");
    let id = u.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let udvt = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_udvt(&udvt))
}

#[salsa::tracked]
pub fn using_data<'db>(db: &'db dyn RootDatabase, u: Using<'db>) -> Arc<UsingData> {
    tracing::debug!(?u, "using_data: cache miss");
    let id = u.id(db);
    let ast_id_map = db.ast_id_map(id.file);
    let root = db.root(id.file);
    let using = ast_id_map.get(&root, id.id).unwrap();
    Arc::new(ItemBuilder::new(root).build_using(&using))
}

#[salsa::tracked]
pub fn bases<'db>(db: &'db dyn HirDatabase, def: DefWithBases<'db>) -> Vec<DefId> {
    tracing::debug!(?def, "bases: cache miss");
    let id = def.id(db);
    let contaier = match id {
        DefWithBasesId::Contract(c) =>DefId::Contract(c) ,
        DefWithBasesId::Interface(i) => DefId::Interface(i)
    };
    Resolver::build_linearizer(db, contaier).c3_linearize()
}

#[salsa::tracked(lru=100)]
pub fn docs<'db>(db: &'db dyn RootDatabase, def: Id<'db>) -> Option<String> {//TODO make tracked when we move ids into salsa
    let (file, erased_id) = def.id(db).file_id();
    let ast_id_map = db.ast_id_map(file);
    let root = db.root(file);
    let node = ast_id_map.get_node(&root, erased_id?)?;
    collect_doc_comments(&root, &node)
}

/// Doc comments for a declaration at an exact range (e.g. struct fields, enum variants)
/// Untracked: only used on demand by capabilities like hover
pub fn inline_docs(db: &dyn RootDatabase, file: File, range: NodeRange) -> Option<String> {
    let root = db.root(file);
    let node = root.named_child_node(range)?;
    collect_doc_comments(&root, &node)
}

fn collect_doc_comments(root: &AstNode, node: &AstNode) -> Option<String> {
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
    Some(normalize_doc(&comments.join("\n")))
}

fn normalize_doc(raw: &str) -> String {
    let mut lines = Vec::new();
    let mut code_block = false;
    for raw_line in raw.lines() {
        let mut trimmed_line = raw_line.trim().trim_matches(&['/','*']);
        
        let line = if code_block {
            trimmed_line.strip_prefix(' ').unwrap_or(trimmed_line).to_string()
        } else {
            trimmed_line = trimmed_line.trim();
            if let Some(text) = trimmed_line.strip_prefix("@notice") {
                format!("**Notice:**{}", text)
            } else if let Some(text) = trimmed_line.strip_prefix("@dev") {
                format!("**Dev note:**{}", text)
            } else if let Some(text) = trimmed_line.strip_prefix("@param") {
                let mut parts = text.trim().splitn(2, char::is_whitespace);
                let name = parts.next().unwrap_or_default();
                let description = parts.next().unwrap_or_default().trim();
                if description.is_empty() {
                    format!("- **`{name}`**")
                } else {
                    format!("- **`{name}`**: {description}")
                }
            } else if let Some(text) = trimmed_line.strip_prefix("@return") {
                format!("- **Returns**{}", text)
            } else if let Some(text) = trimmed_line.strip_prefix("@title") {
                format!("**Title:**{}", text)
            } else if let Some(text) = trimmed_line.strip_prefix("@author") {
                format!("**Author:**{}", text)
            } else if let Some(text) = trimmed_line.strip_prefix("@custom:") {
                format!("**Custom:** {}", text.trim())
            } else {
                trimmed_line.to_string()
            }
        };

        if line.starts_with("```") {code_block = !code_block};
        lines.push(line);
    }
    lines.join("\n")
}


#[derive(Default, Clone, PartialEq, Eq)]
pub struct UsingIndex {
    pub exact: FxHashMap<Type, FxHashMap<Name, SmallVec<[DefId; 1]>>>,
    pub any: FxHashMap<Name, SmallVec<[DefId; 1]>>,
}

#[salsa::tracked]
pub fn resolve_using<'db>(db: &'db dyn HirDatabase, using: Using<'db>) -> UsingIndex {
    tracing::debug!(?using, "resolve_using: cache miss");
    let mut using_index = UsingIndex::default();
    let using_data = using_data(db, using);
    let id = using.id(db);
    let def_id = DefId::Using(id);
    let ctx = Context {file: id.file, container: def_id};
    let resolver = Resolver::build(db, &ctx);
    let target = using_data.target.and_then(|ty_id| resolver.lower_type_name(ty_id, &using_data.expr_store));

    let mut collect = |fn_def: DefId| {
        let DefId::Function(f_id) = fn_def else { return; };
        let fn_data = function_data(db, Function::new(db, f_id));
        if fn_data.parameters.is_empty() {
            return;
        }
        match &target {
            Some(target_ty) => {
                using_index
                    .exact
                    .entry(target_ty.clone())
                    .or_default()
                    .entry(fn_data.name.clone())
                    .or_default()
                    .push(fn_def);
            }
            None => {
                using_index
                    .any
                    .entry(fn_data.name.clone())
                    .or_default()
                    .push(fn_def);
            }
        }
    };

    for source in &using_data.sources {
        let Some(res) = resolver.resolve_expr(*source, &using_data.expr_store, None) else {
            continue;
        };
        match res {
            // using sources can only be fns and libs, fns are resolved as defs and libs as types
            Resolution::Callable(callable) => {
                if let Some(def @ DefId::Function(_)) = callable.def() {
                    collect(def);
                }
            },
            Resolution::Callables(callables) => {
                for callable in callables.iter() {
                    if let Some(def @ DefId::Function(_)) = callable.def() {
                        collect(def);
                    }
                }
            },
            Resolution::Type(Type::Def(def @ DefId::Library(_))) => {
                let defmap = resolver.def_map(&def);
                let data = defmap.defs.get(&def).unwrap();
                let scope = &defmap.scopes[data.child_scope.unwrap()];

                let fns: Vec<DefId> = scope.by_name.values()
                    .filter(|sd| sd.namespace == Namespace::Function)
                    .flat_map(|sd| sd.defs.iter().copied())
                    .collect();

                for fn_def in fns {
                    collect(fn_def);
                }
            },
            _ => {},
        };
    }

    using_index
}


#[salsa::tracked]
pub fn collect_file_using<'db>(db: &'db dyn HirDatabase, file: File) -> UsingIndex {
    let mut index = UsingIndex::default();
    let defmap = db.root_def_map(db.file_source_root(file));
    if let Some(file_data) = defmap.files.get(&file) {
        collect_scope_usings(db, &defmap, file_data.scope, &mut index);
    }
    index
}

fn collect_scope_usings<'db>(
    db: &'db dyn HirDatabase,
    defmap: &DefMap,
    scope_id: Idx<Scope>,
    index: &mut UsingIndex,
) {
    let scope = &defmap.scopes[scope_id];
    if let Some(usings) = &scope.usings {
        for using_def in usings {
            if let DefId::Using(using_id) = using_def {
                let using = Using::new(db, *using_id);
                let resolved = resolve_using(db, using);
                merge_using_index(index, resolved);
            }
        }
    }
}

fn merge_using_index(target: &mut UsingIndex, source: UsingIndex) {
    for (ty, by_name) in source.exact {
        let entry = target.exact.entry(ty).or_default();
        for (name, defs) in by_name {
            entry.entry(name).or_default().extend(defs);
        }
    }
    for (name, defs) in source.any {
        target.any.entry(name).or_default().extend(defs);
    }
}

#[salsa::tracked]
pub fn collect_contract_using<'db>(db: &'db dyn HirDatabase, contract: Contract<'db>) -> UsingIndex {
    tracing::debug!(?contract, "collect_contract_using: cache miss");
    let def_id = DefId::Contract(contract.id(db));
    let (file, _) = def_id.file_id();
    let mut index = collect_file_using(db, file);

    for base in db.bases(def_id) {
        let (base_file, _) = base.file_id();
        let base_defmap = db.root_def_map(db.file_source_root(base_file));
        if let Some(base_data) = base_defmap.defs.get(&base) {
            if let Some(base_child_scope) = base_data.child_scope {
                collect_scope_usings(db, &base_defmap, base_child_scope, &mut index);
            }
        }
    }
    index
}

#[salsa::tracked]
pub fn collect_using<'db>(db: &'db dyn HirDatabase, id: Id<'db>) -> UsingIndex {
    let def = id.id(db);
    match def {
        DefId::File(f) => collect_file_using(db, f),
        DefId::Contract(c) => {
            let contract = Contract::new(db, c);
            collect_contract_using(db, contract)
        }
        _ => UsingIndex::default(),
    }
}


#[salsa::db]
pub trait HirDatabase: RootDatabase {
    fn body_map(&self, owner: BodyOwnerId) -> Arc<BodyMap>;
    fn body_source_map(&self, owner: BodyOwnerId) -> Arc<BodySourceMap>;
    fn body_and_source_map(&self, owner: BodyOwnerId) -> (Arc<BodyMap>, Arc<BodySourceMap>);
    fn function_data(&self, id: ast::FunctionId) -> Arc<FunctionData>;
    fn function_signature(&self, id: ast::FunctionId) -> Option<Arc<FunctionSignature>>;
    fn struct_data(&self, id: ast::StructId) -> Arc<StructData>;
    fn enum_data(&self, id: ast::EnumId) -> Arc<EnumData>;
    fn var_data(&self, id: ast::VarId) -> Arc<VarData>;
    fn error_data(&self, id: ast::ErrorId) -> Arc<ErrorData>;
    fn error_signature(&self, id: ast::ErrorId) -> Option<Arc<ErrorSignature>>;
    fn modifier_data(&self, id: ast::ModifierId) -> Arc<ModifierData>;
    fn modifier_signature(&self, id: ast::ModifierId) -> Option<Arc<ModifierSignature>>;
    fn event_data(&self, id: ast::EventId) -> Arc<EventData>;
    fn event_signature(&self, id: ast::EventId) -> Option<Arc<EventSignature>>;
    fn contract_data(&self, id: ast::ContractId) -> Arc<ContractData>;
    fn interface_data(&self, id: ast::InterfaceId) -> Arc<InterfaceData>;
    fn library_data(&self, id: ast::LibraryId) -> Arc<LibraryData>;
    fn import_data(&self, id: ast::ImportId) -> Arc<ImportData>;
    fn udvt_data(&self, id: ast::UdvtId) -> Arc<UdvtData>;
    fn using_data(&self, id: ast::UsingId) -> Arc<UsingData>;
    fn bases(&self, def: DefId) -> Vec<DefId>;
    fn docs(&self, def: DefId) -> Option<String>;
    fn inline_docs(&self, file: File, range: NodeRange) -> Option<String>;
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

    fn function_signature(&self, id: ast::FunctionId) -> Option<Arc<FunctionSignature>> {
        function_signature(self, Function::new(self, id))
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

    fn error_signature(&self, id: ast::ErrorId) -> Option<Arc<ErrorSignature>> {
        error_signature(self, Error::new(self, id))
    }

    fn modifier_data(&self, id: ast::ModifierId) -> Arc<ModifierData> {
        modifier_data(self, Modifier::new(self, id))
    }

    fn modifier_signature(&self, id: ast::ModifierId) -> Option<Arc<ModifierSignature>> {
        modifier_signature(self, Modifier::new(self, id))
    }

    fn event_data(&self, id: ast::EventId) -> Arc<EventData> {
        event_data(self, Event::new(self, id))
    }

    fn event_signature(&self, id: ast::EventId) -> Option<Arc<EventSignature>> {
        event_signature(self, Event::new(self, id))
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

    fn udvt_data(&self, id: ast::UdvtId) -> Arc<UdvtData> {
        udvt_data(self, Udvt::new(self, id))
    }

    fn using_data(&self, id: ast::UsingId) -> Arc<UsingData> {
        using_data(self, Using::new(self, id))
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

    fn inline_docs(&self, file: File, range: NodeRange) -> Option<String> {
        inline_docs(self, file, range)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_doc;

    #[test]
    fn normalizes_doc_comment_prefixes_and_code_fences() {
        let raw = "/// @notice Example\n/// @param value The value\n/// ```solidity\n///     uint256 value;\n/// ```";
        let normalized = normalize_doc(raw);
        assert_eq!(normalized, "**Notice:** Example\n- **`value`**: The value\n```solidity\n    uint256 value;\n```");
    }
}
