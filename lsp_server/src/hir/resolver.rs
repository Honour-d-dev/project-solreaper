/*
The Rsolver currently is a combination of a lot of different parts.
Ideally these should be seperated in the future once we have a defined shape.
For now its a combination of a name resolver, an inference layer, a linearizer etc
*/
use std::vec;

use rustc_hash::FxHashSet;
use triomphe::Arc;

use crate::ast::{ EnumId, HasBases};
use crate::hir::builtins::{Builtin, BuiltinDB, BuiltinField, BuiltinFn, SUPER, THIS};
use crate::ir::def_map::{DefId, DefMap, Namespace, Scope as DefScope, ScopeData};
use crate::hir::body_map::{BodyMap, BodyOwnerId, BodySourceMap, ByteOffset, LocalId, Location, ScopeId, VariableKind};
use crate::hir::exprs::{BinaryOp, Expr, ExprId, Name};
use crate::hir::item_data::{ExprStore, FieldId, VariantId};
use crate::hir::types::{Fn, Path, Primitive, Type, TypeId, TypeKey, TypeName};
use crate::salsa::{File, HirDatabase, SalsaDb};
use crate::salsa::hir_db::{collect_using, UsingIndex};
use crate::salsa::interned_db::Id as InternedId;
use crate::utilities::log_info;



#[derive(Clone, PartialEq, Eq)]
pub enum CallableOrigin {
    Def(DefId),
    Builtin(BuiltinFn)

}

#[derive(Clone, PartialEq, Eq)]
pub struct Callable {
    def: CallableOrigin,
    bound_args: u8
}


impl Callable {
    pub fn new_def(def: DefId, bound: u8) -> Callable {
        Callable { def: CallableOrigin::Def(def), bound_args: bound }
    }

    pub fn new_builtin(builtin: BuiltinFn, bound: u8) -> Callable {
        Callable { def: CallableOrigin::Builtin(builtin), bound_args: bound }
    }

    pub fn def(&self) -> Option<DefId> {
        match &self.def {
            CallableOrigin::Def(def) => Some(*def),
            CallableOrigin::Builtin(_) => None
        }
    }

    pub fn builtin(&self) -> Option<BuiltinFn> {
        match &self.def {
            CallableOrigin::Builtin(b) => Some(b.clone()),
            CallableOrigin::Def(_) => None
        }
    }

    pub fn bound(&self) -> u8 {
        self.bound_args
    }
}


#[derive(Clone, PartialEq, Eq)]
pub enum FieldOrigin {
    Struct(DefId, FieldId),
    Builtin(BuiltinField)
}

#[derive(Clone, PartialEq, Eq)]
pub struct Field {
    origin: FieldOrigin,
    ty: TypeKey
}

impl Field {
    pub fn new(owner: DefId, field: FieldId, ty: TypeKey) -> Field {
        Field {
            ty,
            origin: FieldOrigin::Struct(owner, field),
        }
    }

    pub fn new_builtin(b: BuiltinField) -> Field {
        Field {
            ty: TypeKey(Type::Primitive(b.ty), b.loc),
            origin: FieldOrigin::Builtin(b),
        }
    }

    pub fn ty(&self) -> &TypeKey {
        &self.ty
    }

    pub fn struct_field(&self) -> Option<(DefId, FieldId)> {
        match &self.origin {
            FieldOrigin::Struct(owner, field) => Some((*owner, *field)),
            FieldOrigin::Builtin(_) => None,
        }
    }

    pub fn builtin(&self) -> Option<BuiltinField> {
        match &self.origin {
            FieldOrigin::Builtin(field) => Some(field.clone()),
            FieldOrigin::Struct(_, _) => None,
        }
    }
}



#[derive(Clone, PartialEq, Eq)]
pub enum Resolution {
    File(File),
    Local(LocalId),
    Var(DefId),
    Callable(Callable),
    Callables(Box<[Callable]>),
    /// A callable that has been called
    /// functions have 2 states (kind of)
    /// A callable (function_name) and a called (function_name())
    /// Each resolving to a different type.
    Called(Callable),
    Type(Type),
    /// A type with location awareness/info
    /// Type coversions won't entirely be correct without location, hence why we need a seperate repr.
    TypeKey(TypeKey),
    Variant(EnumId, VariantId),
    Field(Field),
    Builtin(Builtin),
    MetaType(Type),
    Super(DefId),
}

struct BodyCtx {
    body: Arc<BodyMap>,
}

pub struct Context {
    pub file: File,
    pub container: DefId,
}

impl Context {
    pub fn new(db: &SalsaDb, file: File, offset: ByteOffset) -> Context {
        let node = db.named_node_at(file, offset).unwrap();
        let containers = db.enclosing_containers(node.node(), file);
        Context { file, container: containers.last().unwrap().clone() }
    }

}

pub struct Resolver<'db> {
    db: &'db dyn HirDatabase,
    defmap: Arc<DefMap>,
    pub file: File,// for top level items file is container, but we need to distinguish for base resolution
    container: DefId,
    body: Option<BodyCtx>,
}

impl<'db> Resolver<'db> {
    pub fn build(db: &'db dyn HirDatabase, ctx: &Context) -> Resolver<'db> {
        let body = match ctx.container {
            DefId::Function(id) => {
                let (body, _) = db.body_and_source_map(BodyOwnerId::Function(id));
                let body = BodyCtx { body };
                Some(body)
            }
            DefId::Modifier(id) => {
                let (body, _) = db.body_and_source_map(BodyOwnerId::Modifier(id));
                let body = BodyCtx { body };
                Some(body)
            }
            _ => None
        };

        let defmap = db.root_def_map(db.file_source_root(ctx.file));
        let data = defmap.defs.get(&ctx.container).unwrap();
        let container = defmap.scopes[data.scope].owner;
        
        Resolver { db, defmap, file: ctx.file, container, body }
    }
    
    pub fn build_linearizer(db: &'db dyn HirDatabase, container: DefId ) -> Resolver<'db> {
        let (file,_) = container.file_id();
        let defmap = db.root_def_map(db.file_source_root(file));
        Resolver{ db, defmap, file, container, body: None}

    }

    pub fn body(&self) -> Option<&BodyMap> {
        self.body.as_ref().map(|b| b.body.as_ref())
    }

    /// In some cases we can't make assumptions on the defmap since resolution can jump between defmaps
    /// Used When resulution may not be in the current defmap
    pub fn def_map(&self, def: &DefId) -> Arc<DefMap>  {
        let (file, _) = def.file_id();
        if file == self.file {
            self.defmap.clone()
        } else {
            self.db.root_def_map(self.db.file_source_root(file))
        }
    }

    #[inline]
    fn resolution(&self, def: &ScopeData) -> Resolution {
        match def.namespace {
            Namespace::Type => Resolution::Type(Type::Def(def.defs[0])),//can i remove type from res totally? resolve directly to tk here?
            Namespace::Variable => Resolution::Var(def.defs[0]),
            Namespace::Error | Namespace::Event | Namespace::Function => {
                match def.defs.len() {
                    1 => Resolution::Callable(Callable::new_def(def.defs[0], 0)),
                    _ => Resolution::Callables(def.defs.iter().map(|d| Callable::new_def(*d, 0)).collect())
                }
            }
        }
    }

    #[inline]
    fn builtin_resolution(&self, m: Builtin) -> Resolution {
        match m {
            Builtin::Obj(b) => Resolution::Builtin(Builtin::Obj(b)),
            Builtin::Field(b) => Resolution::Field(Field::new_builtin(b)),
            Builtin::Fn(f) => Resolution::Callable(Callable::new_builtin(f, 0)),
        }
    }

    // MARK: resolve name
    pub fn resolve_name(&self, name: &Name, local_scope: Option<ScopeId>, offset: ByteOffset) -> Option<Resolution> {
        match Primitive::parse(name.as_str()) {
            p if p != Primitive::Unknown => return Some(Resolution::Type(Type::Primitive(p))),
            _ => {
                let globals = BuiltinDB::resolve_name(name.as_str());
                match globals.as_slice() {
                    [Builtin::Obj(object)] => Some(Resolution::Builtin(Builtin::Obj(object.clone()))),
                    [Builtin::Fn(function)] => Some(Resolution::Callable(Callable::new_builtin(function.clone(), 0))),
                    globals if !globals.is_empty() => Some(Resolution::Callables(
                        globals.iter().filter_map(|global| match global {
                            Builtin::Fn(function) => Some(Callable::new_builtin(function.clone(), 0)),
                            Builtin::Obj(_) | Builtin::Field(_) => None,
                        }).collect()
                    )),
                    _ => None,
                }
                .or_else(|| self.resolve_local(name, local_scope, offset).map(Resolution::Local))
                .or_else(|| self.resolve_def(name))
            }
        }
    }

    fn resolve_local(&self, name: &Name, local_scope: Option<ScopeId>, offset: ByteOffset) -> Option<LocalId> {
        let body = self.body()?;
        let mut scope_id = local_scope?;
        loop {
            let scope = &body.scopes[scope_id];
    
            if let Some(local_id) = scope.get(name) {
                let local = &body.locals[*local_id];
    
                if local.offset() <= offset {
                    return Some(*local_id);
                }
            }
    
            scope_id = scope.parent()?
        }
    }


    fn resolve_def(&self, name: &Name) -> Option<Resolution> {
        let data = self.defmap.defs.get(&self.container)?;
        let start = data.child_scope.unwrap_or(data.scope);
        let mut scope = &self.defmap.scopes[start];

        loop {
            if let Some(def) = scope.by_name.get(name) {
                return Some(self.resolution(def));
                // I think resolve def should resolve everything as a def. 
                // the defs we resolve as types are those tied to variables where we can extract location info
            } else {
                // resolver container are scope owners(see build), which means container can only ever be file/contract/interface/lib
                // and libraries dont have bases, so we ignore them in else branch
                let resolution = match scope.owner {
                    id @ (DefId::Contract(_) | DefId::Interface(_)) => {
                        match name.as_str() {
                            THIS => Some(Resolution::Type(Type::Def(id))),
                            SUPER => {
                                Some(Resolution::Super(id))
                            }
                            _ =>  self.lookup_in_bases(id, name)
                        }
                        
                    },
                    DefId::File(f) => self.lookup_in_imports(f, name),
                    _ => None,
                };
                if resolution.is_some() {
                    return resolution;
                }
            }
            scope = &self.defmap.scopes[scope.parent?];
        }
    }


    /// Strictly type path resolution i.e. A.B.C
    /// Returns None if any part of the path is not a type
    pub fn resolve_path(&self, path: &[Name]) -> Option<Resolution> {
        let mut resolution = self.resolve_type_name(path.first()?)?;
        for name in path.iter().skip(1) {
            let Resolution::Type(ty) = resolution else {return None;};
            resolution = self.lookup_type(&ty.def_id()?, name)?;//or  else lookup name? enum variants
        }
        Some(resolution)
    }

    fn resolve_type_name(&self, name: &Name) -> Option<Resolution> {
        if let Some(p) = self.resolve_primitive(name) {
            return Some(Resolution::Type(Type::Primitive(p)));
        }
        self.resolve_def(name)
    }

    fn resolve_primitive(&self, name: &Name) -> Option<Primitive> {
        let primitive = Primitive::parse(name.as_str());
 
        match primitive {
            Primitive::Unknown => None,
            _ => Some(primitive),
        }
    }

    pub fn resolve_base(&self, path: Path) -> Option<Resolution> {
        let segments = path.segments;
        let mut resolution = self.lookup_type(&DefId::File(self.file), segments.first()?)?;
        for name in segments.iter().skip(1) {
            let Resolution::Type(ty) = resolution else {return None;};
            resolution = self.lookup_type(&ty.def_id()?, name)?;
        }
        Some(resolution)
    }

    /// Look up a nested type in its parent
    /// Similar to `lookup_name` but we can take advantage of the fact that we're looking for a type to be more efficient
    /// However, Enum variants (although types) should use `lookup_name` instead since the field itself is the type
    pub fn lookup_type(&self, def: &DefId, name: &Name) -> Option<Resolution> {
        let defmap = self.def_map(def);
        let data = defmap.defs.get(def)?;
        let scope = &defmap.scopes[data.child_scope?];

        if let Some(def) = scope.by_name.get(name) && def.namespace == Namespace::Type {
            return Some(Resolution::Type(Type::Def(def.defs[0])));
        } else {
            match def {
                id @ (DefId::Contract(_) | DefId::Interface(_)) => {
                    self.lookup_in_bases(*id, name)
                }
                DefId::File(f) => {
                    self.lookup_in_imports(*f, name)
                }
                _ => None
            }
        }
    }

    /// Look up a member in a resolved type
    /// `loc` is the data location to assign to field members (threaded from parent)
    fn lookup_in_type(&self, typekey: &TypeKey, name: &Name) -> Option<Resolution> {
        match typekey.typ() {
            Type::Def(def) => {
                let defmap = self.def_map(def);
                let data = defmap.defs.get(def)?;
    
                if let Some(scope_id) = data.child_scope && 
                let Some(def) = defmap.scopes[scope_id].by_name.get(name) {
                    Some(self.resolution(def))
                } else {
                    match def {
                        DefId::Enum(e) => {
                            let enum_data = self.db.enum_data(*e);
                            enum_data.variants.iter()
                                .find(|(_, v)| v.name == *name)
                                .map(|(id, _)| Resolution::Variant(*e, id))
                        }
                        DefId::Struct(s) => {
                            let struct_data = self.db.struct_data(*s);
                            struct_data.fields.iter()
                                .find(|(_, f)| f.name == *name)
                                .and_then(|(id, field)| self.lower_type_name(field.type_name, &struct_data.expr_store).map(|ty| {
                                    Resolution::Field(Field {
                                        origin: FieldOrigin::Struct(DefId::Struct(*s), id),
                                        ty: ty.upcast_from(typekey.loc()),
                                    })
                                }))
                        }
                        DefId::Udvt(u) => {
                            let data = self.db.udvt_data(*u);
                            BuiltinDB::lookup_in_udvt(*def, data.name.clone(), data.underlying, name.as_str()).map(|m| self.builtin_resolution(m))
                        }
                        id @ (DefId::Contract(_) | DefId::Interface(_)) => {
                            self.lookup_in_bases(*id, name)
                        }
                        DefId::File(f) => {
                            self.lookup_in_imports(*f, name)
                        }
                        _ => None
                    }
                }
            }
            Type::Primitive(_) | Type::Array { .. } | Type::Fn(_) | Type::Error => {
                BuiltinDB::lookup_in_type(typekey.typ(), name).map(|m| self.builtin_resolution(m))
            }
            Type::Mapping{..} => None,
            Type::Tuple(_) | Type::Literal(_) => None,
        }
    }

    /// Fallback: check using directives visible in the current container for a function
    /// matching `name` that's attached to `tk`.
    fn using_defs(&self, tk: &TypeKey, name: &Name, index: &UsingIndex) -> Vec<DefId> {
        let mut defs = index
            .exact
            .get(tk.typ())
            .and_then(|members| members.get(name))
            .cloned()
            .unwrap_or_default();
        defs.extend(index.any.get(name).into_iter().flatten().copied());
        defs.retain(|def| {
            let callable = Callable::new_def(*def, 1);
            self.param_types(&callable)
                .first()
                .is_some_and(|param| tk.converts_to(param, self.db).is_some())
        });
        let mut seen = FxHashSet::default();
        defs.retain(|def| seen.insert(*def));
        defs.to_vec()
    }

    /// Fallback: check using directives visible in the current container for a function
    /// matching `name` that's attached to `tk`.
    fn lookup_using(&self, tk: &TypeKey, name: &Name) -> Option<Resolution> {
        let id = InternedId::new(self.db, self.container);
        let index = collect_using(self.db, id);
        let defs = self.using_defs(tk, name, &index);
        match defs.as_slice() {
            [] => None,
            [def] => Some(Resolution::Callable(Callable::new_def(*def, 1))),
            defs => Some(Resolution::Callables(defs.iter().map(|def| Callable::new_def(*def, 1)).collect())),
        }
    }

    fn append_using(&self, tk: &TypeKey) -> Vec<Resolution> {
        let id = InternedId::new(self.db, self.container);
        let index = collect_using(self.db, id);
        let mut names = index.exact.get(tk.typ()).into_iter().flat_map(|members| members.keys().cloned()).collect::<Vec<_>>();
        names.extend(index.any.keys().cloned());
        names.sort();
        names.dedup();
        names.into_iter().flat_map(|name| self.lookup_using(tk, &name)).collect()
    }

    /// Member lookup — threads the parent's location to child resolutions.
    pub fn lookup_in_resolution(&self, res: Resolution, name: &Name) -> Option<Resolution> {
        match res {
            Resolution::Builtin(global) => {
                BuiltinDB::lookup_in_global(&global, name).map(|m| self.builtin_resolution(m))
            }
            Resolution::MetaType(ty) => {
                BuiltinDB::lookup_in_meta(&ty, name).map(|m| self.builtin_resolution(m))
            }
            Resolution::Var(_) | Resolution::Local(_) | Resolution::Callable(_) | Resolution::Called(_) => {
                let tk = self.infer_type(res)?;
                self.lookup_in_type(&tk, name,)
                    .or_else(|| self.lookup_using(&tk, name))
            }
            Resolution::TypeKey(tk) => {
                self.lookup_in_type(&tk, name)
                    .or_else(|| self.lookup_using(&tk, name))
            }
            Resolution::Type(ty) => {
                self.lookup_in_type(&ty.upcast(), name)
            }
            Resolution::Field(field) => {
                self.lookup_in_type(field.ty(), name)
                    .or_else(|| self.lookup_using(field.ty(), name))
            }
            Resolution::Variant(_,_) => {
                let tk = self.infer_type(res)?;
                self.lookup_in_type(&tk, name)
                    .or_else(|| self.lookup_using(&tk, name))//cant have using on variant??
            }
            Resolution::Super(id) => {
                self.lookup_in_bases(id, name)
            }
            // Overload sets are resolved when called, so only a single Callable
            // should reach member lookup after call resolution.
            Resolution::Callables(_) | Resolution::File(_) => None,
        }
    }


    fn builtin_members(&self, members: &[Builtin]) -> Vec<Resolution> {
        members.iter().cloned().map(|member| self.builtin_resolution(member)).collect()
    }

    pub fn members(&self, res: Resolution) -> Vec<Resolution> {
        match res {
            Resolution::File(file) => self.file_members(file),
            Resolution::Local(_) | Resolution::Var(_) | Resolution::Callable(_) | Resolution::Called(_) | Resolution::Field(_) | Resolution::Variant(_, _) => {
                self.infer_type(res).map(|tk| self.type_members(&tk)).unwrap_or_default()
            }
            Resolution::Callables(_) => Vec::new(),
            Resolution::Type(ty) => self.type_members(&ty.upcast()),
            Resolution::TypeKey(tk) => self.type_members(&tk),
            Resolution::Builtin(global) => self.builtin_members(&BuiltinDB::members_in_global(&global)),
            Resolution::MetaType(ty) => self.builtin_members(&BuiltinDB::meta_type_members(&ty)),
            Resolution::Super(def) => {
                let mut members = vec![];
                for base in self.db.bases(def).iter().skip(1) {
                    let base_map = self.def_map(base);
                    if let Some(data) = base_map.defs.get(base) {
                        if let Some(scope_id) = data.child_scope {
                            self.append_scope_members(&mut members, &base_map, scope_id);
                        }
                    }
                }
                members
            },
        }
    }

    fn append_scope_members(&self, members: &mut Vec<Resolution>, defmap: &DefMap, scope_id: la_arena::Idx<DefScope>) {
        members.extend(defmap.scopes[scope_id].by_name.values().map(|scope| self.resolution(scope)));
    }

    fn file_members(&self, file: File) -> Vec<Resolution> {
        let defmap = self.def_map(&DefId::File(file));
        let Some(file_data) = defmap.files.get(&file) else { return Vec::new(); };
        let mut members = Vec::new();
        self.append_scope_members(&mut members, &defmap, file_data.scope);
        for entry in &file_data.imported_scopes {
            let imported = if entry.root == defmap.root {
                defmap.clone()
            } else {
                self.db.root_def_map(entry.root)
            };
            self.append_scope_members(&mut members, &imported, entry.scope);
        }
        members
    }

    fn type_members(&self, tk: &TypeKey) -> Vec<Resolution> {
        let mut members = Vec::new();
        match tk.typ() {
            Type::Def(def) => {
                let defmap = self.def_map(def);
                if let Some(data) = defmap.defs.get(def) {
                    if let Some(scope_id) = data.child_scope {
                        self.append_scope_members(&mut members, &defmap, scope_id);
                    }
                }
                match def {
                    DefId::Enum(e) => {
                        let data = self.db.enum_data(*e);
                        members.extend(data.variants.iter().map(|(id, _)| Resolution::Variant(*e, id)));
                    }
                    DefId::Struct(s) => {
                        let data = self.db.struct_data(*s);
                        members.extend(data.fields.iter().filter_map(|(id, field)| {
                            self.lower_type_name(field.type_name, &data.expr_store).map(|ty| {
                                Resolution::Field(Field::new(*def, id, ty.upcast_from(tk.loc())))
                            })
                        }));
                    }
                    DefId::Udvt(u) => {
                        let data = self.db.udvt_data(*u);
                        members.extend(self.builtin_members(&BuiltinDB::udvt_members(
                            *def,
                            data.name.clone(),
                            data.underlying,
                        )));
                    }
                    DefId::Contract(_) | DefId::Interface(_) => {
                        for base in self.db.bases(*def).iter().skip(1) {
                            let base_map = self.def_map(base);
                            if let Some(data) = base_map.defs.get(base) {
                                if let Some(scope_id) = data.child_scope {
                                    self.append_scope_members(&mut members, &base_map, scope_id);
                                }
                            }
                        }
                    }
                    DefId::File(file) => members.extend(self.file_members(*file)),
                    _ => {}
                }
            }
            Type::Primitive(_) | Type::Array { .. } | Type::Fn(_) | Type::Error => {
                members.extend(self.builtin_members(&BuiltinDB::members_in_type(tk.typ())));
            }
            Type::Tuple(_) | Type::Mapping { .. } | Type::Literal(_) => {}
        }
        members.extend(self.append_using(tk));
        members
    }
        
    

    fn lookup_in_bases(&self, id: DefId, name: &Name) -> Option<Resolution> {
        //get linearized bases and lookup bases
        let bases = self.db.bases(id);
        for base in bases.iter().skip(1) {
            let base_defmap = self.def_map(base);
            let Some(d) = base_defmap.defs.get(base) else {
                continue;
            };
            let Some(child_scope) = d.child_scope else {
                continue;
            };
            let s = &base_defmap.scopes[child_scope];
            if let Some(def) = s.by_name.get(name) {
                return Some(self.resolution(def));
            }
        }
        None
    }

    fn lookup_in_imports(&self, file: File, name: &Name) -> Option<Resolution> {
        //get imported scopes and lookup imports
        let defmap = self.def_map(&DefId::File(file));
        let file_data = defmap.files.get(&file)?;
        for scope_entry in file_data.imported_scopes.iter() {
            let defmap = self.db.root_def_map(scope_entry.root);
            let scope = &defmap.scopes[scope_entry.scope];
            if let Some(def) = scope.by_name.get(name) {
                return Some(self.resolution(def));
            }
        }
        None
    }


    /// This fn makes certain assumptions on the resolvers context.
    /// The container must be the contract/interface whose bases we are resloving
    pub fn c3_linearize(&self) -> Vec<DefId> {

        let bases = match self.container {
            DefId::Contract(id) => {
                self.db.ast_id_map(id.file).get(&self.db.root(id.file), id.id).unwrap().bases()
            }
            DefId::Interface(id) => {
                self.db.ast_id_map(id.file).get(&self.db.root(id.file), id.id).unwrap().bases()
            }
            _ => return vec![],
        };

        let mut linearized = vec![self.container.clone()];

        if !bases.is_empty() {
            let mut linearized_bases = Vec::new();
            // precedennce is from right to left in solidity's c3 implementation
            for base in bases.into_iter().rev() {
                let Some(Resolution::Type(ty)) = self.resolve_base(base) else { continue;};
                let Some(d) = ty.def_id() else { continue;};
                let linearized = self.db.bases(d);
                linearized_bases.push(linearized);
            }

            Self::c3_merge(linearized_bases, &mut linearized);
        }
        linearized
    }

    fn c3_merge(mut bases: Vec<Vec<DefId>>, linearized: &mut Vec<DefId>) {

        while !bases.is_empty() {
            let mut candidate: Option<DefId> = None;
            for list in bases.iter() {
                let head = list.first().unwrap();
                let appears_in_tail = bases
                    .iter()
                    .any(|list| list.iter().skip(1).any(|v| v == head));
                if !appears_in_tail {
                    candidate = Some(head.clone());
                    break;
                }
            }
            
            let Some(candidate) = candidate else {
                return;// should return with some diagnostic in future
            };
            linearized.push(candidate);
            
            bases.retain_mut(|list| {
                // could we optimize further here? candidate can only be the first item of the list, so no need to check the rest of the list as retain does
                list.retain(|item| item != &candidate);
                !list.is_empty()
            });
        }
    }


    /// MARK:  ___INFERENCE ___
    /// this will be moved to inference eventually just prototyping to see what inference needs


    /// Answers: What type does this expression resolve to?
    pub fn infer_expr(&self, expr: ExprId, store: &ExprStore, sourcemap: Option<&BodySourceMap>) -> Option<TypeKey> {
        let res = self.resolve_expr(expr, store, sourcemap)?;
        self.infer_type(res)
    }

    /// Resolves an expression to a semantic object e.g contract, local, struct field, enum variant etc
    pub fn resolve_expr(&self, expr: ExprId, store: &ExprStore, sourcemap: Option<&BodySourceMap>) -> Option<Resolution> {
        match &store.exprs[expr] {
            Expr::Ident(name) => {
                log_info(format!("Resolving Identifier {} ", name.clone()));
                let scope = sourcemap.and_then(|sm| sm.expr_scopes.get(expr).copied());
                let offset = sourcemap.and_then(|sm| sm.expr_to_node.get(expr).map(|r| r.start)).unwrap_or_default();
                let res = self.resolve_name(name, scope, offset)?;
                Some(res)
            },
            Expr::Literal(l) => Some(Resolution::TypeKey(l.type_key())),
            Expr::Binary { op, left, .. } => {
                match op {
                    BinaryOp::And | BinaryOp::Or | BinaryOp::Eq | BinaryOp::Ne
                    | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
                        Some(Resolution::TypeKey(TypeKey(Type::Primitive(Primitive::Boolean), Location::Stack)))
                    }
                    _ => self.resolve_expr(*left, store, sourcemap),
                }
            }
            Expr::ArrayAccess { base, index: _ } => {
                let res = self.resolve_expr(*base, store, sourcemap)?;
                let tk = self.infer_type(res)?;
                let loc = tk.loc();
                match tk.as_typ() {
                    Type::Array { ty, .. } => Some(Resolution::TypeKey(ty.upcast_from(loc))),
                    Type::Mapping { value, .. } => Some(Resolution::TypeKey(value.upcast_from(Location::Storage))),
                    Type::Primitive(Primitive::Bytes) => {
                        Some(Resolution::TypeKey(TypeKey(Type::Primitive(Primitive::FixedBytes(1)), Location::Stack)))
                    }
                    _ => None,
                }
            }
            Expr::Member { obj, prop } => {
                log_info("Resolving a member expr");
                let res = self.resolve_expr(*obj, store, sourcemap)?;
                self.lookup_in_resolution(res, prop)
            },
            Expr::MetaType(ty) => {
                match self.resolve_expr(*ty, store, sourcemap)? {
                    Resolution::Type(t) => Some(Resolution::MetaType(t)),
                    Resolution::TypeKey(tk) => Some(Resolution::MetaType(tk.as_typ())),
                    _ => None,
                }
            }
            Expr::Call { callee, args } => {
                log_info("Resolving a call expr");
                let callee_res = self.resolve_expr(*callee, store, sourcemap)?;
                let arg_types: Vec<TypeKey> = args.iter()
                    .filter_map(|a| self.infer_expr(*a, store, sourcemap))
                    .collect();
                if arg_types.len() != args.len() { return None;}

                match callee_res {
                    Resolution::Type(ty) => { 
                        //TypeCast branch, we upcast to typekey
                        // On diagnostics this should be where to validate cast/contruction args
                        Some(Resolution::TypeKey(ty.upcast()))
                    }
                    Resolution::Callable(c) => {
                        let param_types = self.param_types(&c);
                        if param_types.len() != (arg_types.len() + c.bound() as usize) {
                            return None;
                        }
                        if arg_types.iter().zip(param_types.iter().skip(c.bound().into())).all(|(a, p)| a.converts_to(p, self.db).is_some()) {
                            Some(Resolution::Called(c))
                        } else {
                            None
                        }
                    }
                    Resolution::Callables(cs) => {
                        let c = self.resolve_overload(&cs[..], &arg_types)?;
                        Some(Resolution::Called(c))
                    },
                    _ => None
                }
            }
            Expr::Path(p) =>  Some(Resolution::File(self.db.resolve_to_file(self.file, p)?)),
        }
    }

    pub fn param_types(&self, c: &Callable) -> Vec<TypeKey> {
        match &c.def {
            CallableOrigin::Def(def) => match def {
                DefId::Function(id) => self.db.function_signature(*id).map(|sig| sig.params.to_vec()).unwrap_or_default(),
                DefId::Event(id) => self.db.event_signature(*id).map(|sig| sig.params.to_vec()).unwrap_or_default(),
                DefId::Error(id) => self.db.error_signature(*id).map(|sig| sig.params.to_vec()).unwrap_or_default(),
                DefId::Modifier(id) => self.db.modifier_signature(*id).map(|sig| sig.params.to_vec()).unwrap_or_default(),
                _ => Vec::new(),
            }
            CallableOrigin::Builtin(b) => {
                b.params.iter().map(|ty| ty.clone().upcast()).collect()
            }
        }
    }

    pub fn resolve_overload(&self, candidates: &[Callable], arg_types: &[TypeKey]) -> Option<Callable> {
        let mut best: Option<(&Callable, u32)> = None;
        'candidates: for c in candidates {
            let param_types = self.param_types(c);
            let bound = c.bound() as usize;
            if param_types.len() != arg_types.len() + bound {
                continue;
            }
            let mut total_cost: u32 = 0;
            for (arg, param) in arg_types.iter().zip(param_types.iter().skip(bound)) {
                match arg.converts_to(param, self.db) {
                    Some(cost) => total_cost += cost as u32,
                    None => { continue 'candidates; }
                }
            }
            match best {
                Some((_, prev_cost)) if total_cost >= prev_cost => {}
                _ => best = Some((c, total_cost)),
            }//make this more robust, candidates with equal costs are returned in the error branch
        }
        best.map(|(c, _)| c.clone())
    }

    fn callable_type(&self, callable: &Callable) -> Option<TypeKey> {
        match &callable.def {
            CallableOrigin::Def(DefId::Function(id)) => {
                let data = self.db.function_data(*id);
                let signature = self.db.function_signature(*id)?;
                Some(Type::Fn(Fn {
                    vis: data.vis,
                    mutability: data.mutability,
                    params: signature.params.iter().map(|param| param.0.clone()).collect(),
                    ret: signature.returns.iter().map(|ret| ret.0.clone()).collect(),
                }).upcast())
            }
            CallableOrigin::Def(DefId::Error(_)) => Some(Type::Error.upcast()),
            CallableOrigin::Def(_) => None,
            CallableOrigin::Builtin(builtin) => Some(Type::Fn(Fn {
                vis: Default::default(),
                mutability: Default::default(),
                params: builtin.params.iter().cloned().collect(),
                ret: builtin.return_type.iter().cloned().collect(),
            }).upcast()),
        }
    }

    fn callable_result_type(&self, callable: &Callable) -> Option<TypeKey> {
        match &callable.def {
            CallableOrigin::Def(DefId::Function(id)) => {
                let returns = self.db.function_signature(*id)?.returns.clone();
                match returns.as_ref() {
                    [] => None,
                    [ret] => Some(ret.clone()),
                    _ => Some(TypeKey(Type::Tuple(returns.iter().map(|ret| ret.0.clone()).collect()), Location::Stack)),
                }
            }
            CallableOrigin::Def(DefId::Error(_)) => Some(Type::Error.upcast()),
            CallableOrigin::Def(_) => None,
            CallableOrigin::Builtin(builtin) => builtin.return_type.clone().map(|ty| {
                let loc = match &ty {
                    Type::Primitive(primitive) => primitive.default_loc(),
                    _ => Location::Memory,
                };
                TypeKey(ty, loc)
            }),
        }
    }

    pub fn infer_type(&self, res: Resolution) -> Option<TypeKey> {
        match res {
            Resolution::Local(l) => {
                let body = self.body()?;
                let local = &body.locals[l];
                self.lower_type_name(*local.type_name(), &body.expr_store).map(|ty| TypeKey(ty, local.location()))
            }
            Resolution::Var(d) => {
                let DefId::Var(id) = d else { return None; };
                let var = self.db.var_data(id);
                self.lower_type_name(var.type_name, &var.expr_store).map(|ty| ty.upcast_from_kind(var.kind))
            }
            Resolution::Callable(c) => self.callable_type(&c),
            Resolution::Called(c) => self.callable_result_type(&c),
            Resolution::TypeKey(tk) => Some(tk),
            Resolution::Type(ty) => Some(ty.upcast()),
            Resolution::Callables(_) => None, // unable to infer type due to ambiguity
            Resolution::Variant(enum_id, _) => {
                Some(TypeKey(Type::Def(DefId::Enum(enum_id)), Location::Stack))
            }
            Resolution::Field(field) => Some(field.ty().clone()),
            Resolution::Super(_) | Resolution::Builtin(_) | Resolution::MetaType(_) | Resolution::File(_) => None,
        }
    }

    /// lower typeId to actual type (no location — location is attached by the caller)
    pub fn lower_type_name(&self, ty_id: TypeId, store: &ExprStore) -> Option<Type> {
         match &store.types[ty_id] {
            TypeName::Primitive(p) => {
                Some(Type::Primitive(*p))
            }
            TypeName::UserDefined(p) => {
                let res = self.resolve_path(&p.segments)?;
                match res {
                    Resolution::Type(ty) => Some(ty),
                    _ => None,
                }
            }
            TypeName::Array { ty, size } => {
                let inner = self.lower_type_name(*ty, store)?;
                let size = size.and_then(|expr| self.eval_array_size(expr, store, None));
                Some(Type::Array{ty: Box::new(inner), size})
            }
            TypeName::Mapping { key, value } => {
                let key_ty = self.lower_type_name(*key, store)?;
                let value_ty = self.lower_type_name(*value, store)?;
                Some(Type::Mapping { key: Box::new(key_ty), value: Box::new(value_ty)})
            }
            TypeName::Fn(f) => {
                let params = f.params.iter().filter_map(|p| {
                    self.lower_type_name(*p, store)
                }).collect::<Box<_>>();
                let ret = f.ret.iter().filter_map(|p| {
                    self.lower_type_name(*p, store)
                }).collect::<Box<_>>();
                let f = Fn {
                    params,
                    ret,
                    vis: f.vis,
                    mutability: f.mutability
                };
                Some(Type::Fn(f))
                
            }
        }
    }

    fn eval_array_size(&self, expr: ExprId, store: &ExprStore, sourcemap: Option<&BodySourceMap>) -> Option<usize> {
        match &store.exprs[expr] {
            Expr::Literal(literal) => literal.integer_value().and_then(|value| value.try_into().ok()),
            Expr::Binary { op, left, right } => {
                let l = self.eval_array_size(*left, store, sourcemap)?;
                let r = self.eval_array_size(*right, store, sourcemap)?;
                match op {
                    BinaryOp::Add => l.checked_add(r),
                    BinaryOp::Sub => l.checked_sub(r),
                    BinaryOp::Mul => l.checked_mul(r),
                    BinaryOp::Div => if r != 0 { Some(l / r) } else { None },
                    BinaryOp::Mod => if r != 0 { Some(l % r) } else { None },
                    BinaryOp::Pow => (0..r).try_fold(1usize, |acc, _| acc.checked_mul(l)),
                    BinaryOp::Shl => l.checked_shl(r as u32),
                    BinaryOp::Shr => l.checked_shr(r as u32),
                    _ => None,
                }
            }
            Expr::Ident(name) => {
                let scope = sourcemap.and_then(|sm| sm.expr_scopes.get(expr).copied());
                let offset = sourcemap.and_then(|sm| sm.expr_to_node.get(expr).map(|r| r.start)).unwrap_or(0);
                let res = self.resolve_name(name, scope, offset)?;
                self.eval_constant(res)
            }
            Expr::Member { obj, prop } => {
                let res = self.resolve_expr(*obj, store, sourcemap)?;
                let res = self.lookup_in_resolution(res, prop)?;
                self.eval_constant(res)
            }
            _ => None,
        }
    }

    fn eval_constant(&self, res: Resolution) -> Option<usize> {
        match res {
            Resolution::Var(DefId::Var(var_id)) => {
                let var_data = self.db.var_data(var_id);
                if var_data.kind != VariableKind::Const {
                    return None;
                }
                let init = var_data.init?;
                self.eval_array_size(init, &var_data.expr_store, None)
            }//@TODO add builtin field resolution suport. builtinfields should have a value param. uint[type(uint8).max] is valid but max does not currently hold values
            _ => None,
        }
    }

}

