#![allow(unused)]

use la_arena::Arena;
use serde::de;
use triomphe::Arc;

use crate::ast::kinds::NodeKind;
use crate::ast::{self, AstNode, ContractId, EnumId, ErrorId, EventId, FunctionId, HasBases, ImportId, InterfaceId, LibraryId, ModifierId, NodeRange, StructId, VarId};
use crate::ir::def_map::{DefData, DefId, DefMap, Namespace, ScopeData};
use crate::hir::body_map::{BodyMap, BodyOwnerId, BodySourceMap, ByteOffset, Local, LocalId, ScopeId, SemanticId, VariableKind};
use crate::hir::exprs::{BinaryOp, Expr, ExprId, Literal, Name};
use crate::hir::item_data::{ExprStore, FieldId, FunctionData, VariantId};
use crate::hir::types::{Mutability, Path, Primitive, Type, TypeId, TypeName, Visibility, Fn};
use crate::salsa::{File, FileId, HirDatabase, RootDatabase, SalsaDb};


/*
- hover finds the enclosing container/body, by walking parent from cursor
- - offset -> name & nodeRange(walking ast)   
- - we need container context too so we know where to search. how do we find container? db.enclosing_containers
- - if last container is body
- - noderange -> semanticId
- - else not in a body
- -   -> defId
- then build the resolver with the things it needs 
- then builds inference context
*/

#[derive(Clone, PartialEq, Eq)]
pub enum Resolution {
    Local(LocalId),
    Def(DefId),
    Primitive(Primitive),
    Type(DefId),
    Defs(Box<[DefId]>),
    Variant(EnumId, VariantId),
    Field(StructId, FieldId),
}

struct BodyCtx {
    body: Arc<BodyMap>,
    sourcemap: Arc<BodySourceMap>,
}

pub struct Context {
    pub file: File,
    pub offset: ByteOffset,
    pub container: DefId,
}

impl Context {
    pub fn new(db: &SalsaDb, file: File, offset: ByteOffset) -> Context {
        let node = db.node_at(file, offset).unwrap();
        let mut containers = db.enclosing_containers(node.node(), file);
        Context { file, offset, container: containers.last().unwrap().clone() }
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
                let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Function(id));
                let body = BodyCtx {
                    body,
                    sourcemap,
                };
                Some(body)
            }
            DefId::Modifier(id) => {
                let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Modifier(id));
                let body = BodyCtx {
                    body,
                    sourcemap,
                };
                Some(body)
            }
            default => None
        };

        let defmap = db.root_def_map(db.file_source_root(ctx.file));
        let data = defmap.defs.get(&ctx.container).unwrap();
        let container = defmap.scopes[data.scope].owner;

        Resolver { db, defmap, file: ctx.file, container, body }
    }

    pub fn body(&self) -> Option<&BodyMap> {
        self.body.as_ref().map(|b| b.body.as_ref())
    }

    pub fn sourcemap(&self) -> Option<&BodySourceMap> {
        self.body.as_ref().map(|b| b.sourcemap.as_ref())
    }

    /// In some cases we can't make assumptions on the defmap since resolution can jump between defmaps
    /// Used When resulution may not be in the current defmap
    fn def_map(&self, def: &DefId) -> Arc<DefMap>  {
        let file = match def {
            DefId::File(file) |
            DefId::Import(ImportId { file, ..}) |
            DefId::Contract(ContractId { file, .. }) |
            DefId::Interface(InterfaceId { file, .. }) |
            DefId::Library(LibraryId { file, .. }) |
            DefId::Enum(EnumId { file, .. }) |
            DefId::Struct(StructId { file, .. }) |
            DefId::Function(FunctionId { file, .. }) |
            DefId::Modifier(ModifierId { file, .. }) |
            DefId::Event(EventId { file, .. }) |
            DefId::Error(ErrorId{file, ..}) |
            DefId::Var(VarId{file, ..}) => file,
        };
        if *file == self.file {
            self.defmap.clone()
        } else {
            self.db.root_def_map(self.db.file_source_root(*file))
        }
    }

    pub fn expr_scope(&self, expr_id: ExprId) -> Option<ScopeId> {
        self.sourcemap()?.expr_scopes.get(expr_id).copied()
    }

    #[inline]
    fn resolution(def: &ScopeData) -> Resolution {
        match def.namespace {
            Namespace::Type => Resolution::Type(def.defs[0]),
            _ => {
                match def.defs.len() {
                    1 => Resolution::Def(def.defs[0]),
                    _ => Resolution::Defs(def.defs.clone().into_boxed_slice())
                }
            }
        }
    }

    
    pub fn resolve_name(&self, name: &Name, local_scope: Option<ScopeId>, offset: ByteOffset) -> Option<Resolution> {
        if let Some(local) = self.resolve_local(name, local_scope, offset) {
            return Some(Resolution::Local(local));
        }
        self.resolve_def(name)
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
        let data = &self.defmap.defs[&self.container];
        let mut scope = &self.defmap.scopes[data.child_scope?];

        loop {
            if let Some(def) = scope.by_name.get(name) {
                return Some(Self::resolution(def));
            } else {
                let resolution = match scope.owner {
                    id @ (DefId::Contract(_) | DefId::Interface(_)) => {
                        match name.as_str() {
                            "this" => Some(Resolution::Type(id)),
                            "super" => {
                                let bases = self.db.bases(id);
                                let supr = bases.iter().nth(1)?;
                                Some(Resolution::Type(*supr))
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
            resolution = self.lookup_type(&ty, name)?;//or  else lookup name? enum variants
        }
        Some(resolution)
    }

    fn resolve_type_name(&self, name: &Name) -> Option<Resolution> {
        if let Some(p) = self.resolve_primitive(name) {
            return Some(Resolution::Primitive(p));
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
            resolution = self.lookup_type(&ty, name)?;
        }
        Some(resolution)
    }

    /// Look up a nested type in its parent
    /// Similar to `lookup_name` but we can take advantage of the fact that we're looking for a type to be more efficient
    /// However, Enum variants (although types) should use `lookup_name` instead since the field itself is the type
    pub fn lookup_type(&self, def: &DefId, name: &Name) -> Option<Resolution> {
        let defmap = self.def_map(def);
        let data = &defmap.defs[def];
        let scope = &defmap.scopes[data.child_scope?];

        if let Some(def) = scope.by_name.get(name) && def.namespace == Namespace::Type {
            return Some(Resolution::Type(def.defs[0]));
        } else {
            match *def {
                id @ (DefId::Contract(_) | DefId::Interface(_)) => {
                    self.lookup_in_bases(id, name)
                }
                DefId::File(f) => {
                    self.lookup_in_imports(f, name)
                }
                _ => None
            }
        }
    }

    /// Look up a name in a parent definition
    /// Mainly for looking up values (functions, variables, events, etc.)
    /// for types see lookup_type.
    pub fn lookup_name(&self, def: &DefId, name: &Name) -> Option<Resolution> {
        let defmap = self.def_map(def);
        let data = &defmap.defs[def];
    
        if let Some(scope_id) = data.child_scope && 
           let Some(def) = defmap.scopes[scope_id].by_name.get(name) {
            Some(Self::resolution(def))
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
                        .map(|(id, _)| Resolution::Field(*s, id))
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

    

    fn lookup_in_bases(&self, id: DefId, name: &Name) -> Option<Resolution> {
        //get linearized bases and lookup bases
        let bases = self.db.bases(id);
        for base in bases.iter().skip(1) {
            let base_defmap = self.def_map(base);
            let d = &base_defmap.defs[base];
            let Some(child_scope) = d.child_scope else {
                continue;
            };
            let s = &base_defmap.scopes[child_scope];
            if let Some(def) = s.by_name.get(name) {
                return Some(Self::resolution(def));
            }
        }
        None
    }

    fn lookup_in_imports(&self, file: File, name: &Name) -> Option<Resolution> {
        //get imported scopes and lookup imports
        let defmap = self.def_map(&DefId::File(file));
        let file_data = &defmap.files[&file];
        for scope_entry in file_data.imported_scopes.iter() {
            let defmap = self.db.root_def_map(scope_entry.root);
            let scope = &defmap.scopes[scope_entry.scope];
            if let Some(def) = scope.by_name.get(name) {
                return Some(Self::resolution(def));
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
                let Some(Resolution::Type(d)) = self.resolve_base(base) else { continue;};
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


    ///////////////////////////INfERENCE LAYER//////////////////////
    /// this will be moved to inference eventually just prototyping to see what inference needs



    pub fn infer_expr(&self, expr: ExprId, store: &ExprStore, sourcemap: Option<&BodySourceMap>) -> Option<Type> {
        match &store.exprs[expr] {
            Expr::Ident(name) => {
                let (scope, offset) = match (name.as_str(), sourcemap) {
                    ("this" | "super", _) => (None, 0),
                    (_, None) => (None, 0),
                    (_, Some(sm)) => (Some(sm.expr_scopes[expr]), sm.expr_to_node[expr].start) 
                };
                let res = self.resolve_name(name, scope, offset)?;
                self.infer_type(res)
            },
            Expr::Path(p) => None,
            Expr::Literal(l) => Some(Type::Primitive(Self::literal_primitive(l))),
            Expr::Binary { left, .. } => self.infer_expr(*left, store, sourcemap),
            Expr::Member { obj, prop } => {
                let ty = self.infer_expr(*obj, store, sourcemap)?;
                self.infer_lookup_type(ty, prop)
            }
            Expr::Array { base, index: _ } => {
                let ty = self.infer_expr(*base, store, sourcemap)?;
                let Type::Array{ ty: base, .. } = ty else { return None; };
                Some(*base)
            }
            Expr::Call { callee, args } => {
                let res = self.resolve_callee(*callee, store, sourcemap)?;
                let arg_types: Vec<Type> = args.iter()
                    .filter_map(|a| self.infer_expr(*a, store, sourcemap))
                    .collect();
                if arg_types.len() != args.len() {
                    return None;
                }
                match res {
                    Resolution::Def(DefId::Function(id)) => {
                        let fn_data = self.db.function_data(id);
                        let param_types = self.param_types(&fn_data);
                        if param_types.len() != arg_types.len() {
                            return None;
                        }
                        if arg_types.iter().zip(param_types.iter()).all(|(a, p)| a.converts_to(p).is_some()) {
                            self.fn_return_type(&fn_data)
                        } else {
                            None
                        }
                    }
                    Resolution::Defs(candidates) => {
                        let def = self.resolve_overload(&candidates[..], &arg_types)?;
                        match def {
                            DefId::Function(id) => {
                                let fn_data = self.db.function_data(id);
                                self.fn_return_type(&fn_data)
                            }
                            _ => None,
                        }
                    }
                    _ => None,//Errors and Event calls dont resolve to a type
                }
            }
        }
    }

    pub fn resolve_callee(&self, callee: ExprId, store: &ExprStore, sourcemap: Option<&BodySourceMap>) -> Option<Resolution> {
        match &store.exprs[callee] {
            Expr::Ident(name) => {
                let scope = sourcemap.and_then(|sm| sm.expr_scopes.get(callee).copied());
                let offset = sourcemap.and_then(|sm| sm.expr_to_node.get(callee).map(|r| r.start)).unwrap_or(0);
                self.resolve_name(name, scope, offset)
            }
            Expr::Member { obj, prop } => {
                let ty = self.infer_expr(*obj, store, sourcemap)?;
                let res = self.lookup_name(&match ty {
                    Type::UserDefined(def) => def,
                    _ => return None,
                }, prop)?;
                Some(res)
            }//@TODO add more calle support call/array/literal?
            _ => None,
        }
    }

    pub fn fn_return_type(&self, fn_data: &FunctionData) -> Option<Type> {
        let ret: Vec<Type> = fn_data.ret_params.iter()
            .filter_map(|p| self.lower_type_name(*fn_data.parameters[*p].type_name(), &fn_data.expr_store))
            .collect();
        match ret.len() {
            0 => None,
            1 => Some(ret.into_iter().next().unwrap()),
            _ => Some(Type::Tuple(ret.into_boxed_slice())),
        }
    }

    pub fn param_types(&self, fn_data: &FunctionData) -> Vec<Type> {
        fn_data.arg_params.iter()
            .filter_map(|p| self.lower_type_name(*fn_data.parameters[*p].type_name(), &fn_data.expr_store))
            .collect()
    }

    pub fn def_param_types(&self, def: DefId) -> Vec<Type> {
        match def {
            DefId::Function(id) => {
                let fn_data = self.db.function_data(id);
                self.param_types(&fn_data)
            }
            DefId::Event(id) => {
                let data = self.db.event_data(id);
                data.parameters.iter()
                    .filter_map(|(_, p)| self.lower_type_name(*p.type_name(), &data.expr_store))
                    .collect()
            }
            DefId::Error(id) => {
                let data = self.db.error_data(id);
                data.parameters.iter()
                    .filter_map(|(_, p)| self.lower_type_name(*p.type_name(), &data.expr_store))
                    .collect()
            }
            DefId::Modifier(id) => {
                let data = self.db.modifier_data(id);
                data.parameters.iter()
                    .filter_map(|(_, p)| self.lower_type_name(*p.type_name(), &data.expr_store))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn resolve_overload(&self, candidates: &[DefId], arg_types: &[Type]) -> Option<DefId> {
        let mut best: Option<(DefId, u32)> = None;
        'candidates: for def in candidates {
            let param_types = self.def_param_types(*def);
            if param_types.len() != arg_types.len() {
                continue;
            }
            let mut total_cost: u32 = 0;
            'cost: for (arg, param) in arg_types.iter().zip(param_types.iter()) {
                match arg.converts_to(param) {
                    Some(cost) => total_cost += cost as u32,
                    None => { continue 'candidates; }
                }
            }
            match best {
                Some((_, prev_cost)) if total_cost >= prev_cost => {}
                _ => best = Some((*def, total_cost)),
            }//make this more robust, candidates with equal costs are returned in the error branch
        }
        best.map(|(def, _)| def)
    }

    pub fn infer_lookup_type(&self, ty: Type, name: &Name) -> Option<Type> {
        match ty {
            Type::UserDefined(ty) => {
                let res = self.lookup_name(&ty, name)?;
                self.infer_type(res)
            }
            Type::Primitive(p) => {
                // TODO: impl primitive member lookups
                None
            }
            Type::Array{ ty: a, .. } => {
                // TODO: impl array members i.e. push/pop etc
                None
            }
            Type::Mapping { key, value } => {
                // Mapping it self shouldn't/doesn't have members i believe
                None
            }
            Type::Fn(_) => None,
            Type::Tuple(_) => None,
        }
    }


    pub fn infer_type(&self, res: Resolution) -> Option<Type> {
        match res {
            Resolution::Local(l) => {
                let body = self.body()?;
                let local = &body.locals[l];
                self.lower_type_name(*local.type_name(), &body.expr_store)
            }
            Resolution::Def(d) => {
                match Namespace::from(&d) {
                    Namespace::Type => Some(Type::UserDefined(d)),
                    Namespace::Variable => {
                        let DefId::Var(id) = d else { return None; };
                        let var = self.db.var_data(id);
                        self.lower_type_name(var.type_name, &var.expr_store)
                    }
                    Namespace::Function => {
                        let DefId::Function(id) = d else { return None; };
                        let fn_data = self.db.function_data(id);
                        let params = fn_data.arg_params.iter()
                            .filter_map(|p| self.lower_type_name(*fn_data.parameters[*p].type_name(), &fn_data.expr_store))
                            .collect::<Box<_>>();
                        let ret = fn_data.ret_params.iter()
                            .filter_map(|p| self.lower_type_name(*fn_data.parameters[*p].type_name(), &fn_data.expr_store))
                            .collect::<Box<_>>();
                        let f = Fn {
                            params,
                            ret,
                            vis: fn_data.vis,
                            mutability: fn_data.mutability
                        };
                        Some(Type::Fn(f))
                    }
                    Namespace::Error | Namespace::Event => None,
                }
            }
            Resolution::Type(ty) => {
                Some(Type::UserDefined(ty))
            }
            Resolution::Primitive(p) => {
                Some(Type::Primitive(p))
            }
            Resolution::Defs(_) => None,
            Resolution::Variant(enum_id, _) => {
                Some(Type::UserDefined(DefId::Enum(enum_id)))
            }
            Resolution::Field(struct_id, field_id) => {
                let struct_data = self.db.struct_data(struct_id);
                let field = &struct_data.fields[field_id];
                self.lower_type_name(field.type_name, &struct_data.expr_store)
            }
        }
    }
    
    /// lower typeId to actual type
    fn lower_type_name(&self, ty_id: TypeId, store: &ExprStore) -> Option<Type> {
         match &store.types[ty_id] {
            TypeName::Primitive(p) => {
                Some(Type::Primitive(*p))
            }
            TypeName::UserDefined(p) => {
                let res = self.resolve_path(&p.segments)?;
                self.infer_type(res)
            }
            TypeName::Array { ty, size } => {
                let ty = self.lower_type_name(*ty, store)?;
                let size = size.and_then(|expr| self.eval_array_size(expr, store, None));
                Some(Type::Array{ty: Box::new(ty), size})
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
            Expr::Literal(Literal::Number(n)) => n.parse::<usize>().ok(),
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
                let ty = self.infer_expr(*obj, store, sourcemap)?;
                let def = match ty {
                    Type::UserDefined(def) => def,
                    _ => return None,
                };
                let res = self.lookup_name(&def, prop)?;
                self.eval_constant(res)
            }
            _ => None,
        }
    }

    fn eval_constant(&self, res: Resolution) -> Option<usize> {
        match res {
            Resolution::Def(DefId::Var(var_id)) => {
                let var_data = self.db.var_data(var_id);
                if var_data.kind != VariableKind::Const {
                    return None;
                }
                let init = var_data.init?;
                self.eval_array_size(init, &var_data.expr_store, None)
            }
            _ => None,
        }
    }


    fn literal_primitive(l: &Literal) -> Primitive {
        match l {
            Literal::Number(_) => Primitive::Uint(256),
            Literal::String(_) => Primitive::String,
            Literal::Boolean(_) => Primitive::Boolean,
            Literal::HexString(_) => Primitive::Bytes,
        }
    }

    fn literal_type(l: &Literal) -> Type {
        Type::Primitive(Self::literal_primitive(l))
    }

}


pub struct Inference<'db> {
    db: &'db dyn HirDatabase,
    resolver: Resolver<'db>,
    arg_str: String,
}

impl<'db> Inference<'db> {
    pub fn new(db: &'db dyn HirDatabase, ctx: Context) -> Self {
        let resolver = Resolver::build(db, &ctx);
        Self { db, resolver, arg_str: String::new() }
    }

    
    fn as_string(ty_id: TypeId, types: &Arena<TypeName>) -> String {
        types[ty_id].to_string(types)
    }
   
}

