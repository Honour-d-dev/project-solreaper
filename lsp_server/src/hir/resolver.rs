#![allow(unused)]

use serde::de;
use triomphe::Arc;

use crate::ast::kinds::NodeKind;
use crate::ast::{self, AstNode, ContractId, FunctionId, HasBases, ModifierId, NodeRange};
use crate::ir::def_map::{DefId, DefMap, Namespace};
use crate::hir::body_map::{BodyMap, BodyOwnerId, BodySourceMap, ByteOffset, Local, LocalId, ScopeId, SemanticId};
use crate::hir::exprs::{Expr, ExprId, Literal, Name};
use crate::hir::types::{Path, Primitive, Type, TypeId, TypeName};
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

pub enum Resolution {
    Local(LocalId),
    Def(DefId),
    Primitive(Primitive),
    Type(DefId),
    Defs(Box<[DefId]>),
    // Field/Member(FieldId)
}

struct BodyCtx {
    body: Arc<BodyMap>,
    sourcemap: Arc<BodySourceMap>,
}

pub struct Context {
    pub file: File,
    pub node: AstNode,
    pub offset: ByteOffset,
    pub containers: Vec<DefId>,
}

impl Context {
    pub fn new(db: &SalsaDb, file: File, offset: ByteOffset) -> Context {
        let node = db.node_at(file, offset).unwrap();
        let mut containers = db.enclosing_containers(node.node(), file);
        Context { file, node, offset, containers }
    }

}



pub struct Resolver<'db> {
    db: &'db dyn HirDatabase,
    defmap: Arc<DefMap>,
    file: File,// for top level items file is container, but we need to distinguish for base resolution
    container: DefId,
    body: Option<BodyCtx>,
}

impl<'db> Resolver<'db> {
    pub fn build(db: &'db dyn HirDatabase, ctx: &Context) -> Resolver<'db> {
        let mut containers = ctx.containers.iter().rev();
        let (body, &container) = match containers.next().unwrap() {
            DefId::Function(id) => {
                let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Function(*id));
                let body = BodyCtx {
                    body,
                    sourcemap,
                };
                (Some(body), containers.next().unwrap())
            }
            DefId::Modifier(id) => {
                let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Modifier(*id));
                let body = BodyCtx {
                    body,
                    sourcemap,
                };
                (Some(body), containers.next().unwrap())
            }
            default => (None, containers.next().unwrap_or(default))// To accomodate linearization init
        };
        let defmap = db.root_def_map(db.file_source_root(ctx.file));
        Resolver { db, defmap, file: ctx.file, container, body }
    }

    pub fn body(&self) -> Option<&BodyMap> {
        self.body.as_ref().map(|b| b.body.as_ref())
    }

    pub fn sourcemap(&self) -> Option<&BodySourceMap> {
        self.body.as_ref().map(|b| b.sourcemap.as_ref())
    }

    pub fn expr_scope(&self, expr_id: ExprId) -> Option<ScopeId> {
        self.sourcemap()?.expr_scopes.get(expr_id).copied()
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
                return match def.defs.len() {
                    1 => Some(Resolution::Def(def.defs[0])),
                    _ => Some(Resolution::Defs(def.defs.clone().into_boxed_slice()))
                };
            } else {
                let resolution = match scope.owner {
                    id @ (DefId::Contract(_) | DefId::Interface(_)) => self.lookup_in_bases(id, name),
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
    fn resolve_path(&self, path: &[Name]) -> Option<Resolution> {
        let mut resolution = self.resolve_type_name(path.first()?)?;
        for name in path.iter().skip(1) {
            let Resolution::Type(ty) = resolution else {return None;};
            resolution = self.lookup_type(&ty, name)?;
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
    fn lookup_type(&self, def: &DefId, name: &Name) -> Option<Resolution> {
        let data = &self.defmap.defs[def];//FIXME: we should not assume defmap here always check the defid
        let scope = &self.defmap.scopes[data.child_scope?];//must have child scope

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
    fn lookup_name(&self, def: &DefId, name: &Name) -> Option<Resolution> {
        let data = &self.defmap.defs[def];
    
        if let Some(scope_id) = data.child_scope && 
           let Some(def) = self.defmap.scopes[scope_id].by_name.get(name) {
            match def.defs.len() {
                1 => Some(Resolution::Def(def.defs[0])),
                _ => Some(Resolution::Defs(def.defs.clone().into_boxed_slice()))
            }
        } else {
            match def {
                DefId::Enum(e) => {
                    //get enum data
                    // lookup in enum
                    // return
                    None
                }
                DefId::Struct(s) => {
                    //get struct data
                    // lookup in struct
                    // return
                    None
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
            let file = match base {
                DefId::Contract(c) => c.file,
                DefId::Interface(i) => i.file,
                _ => continue
            };
            let base_defmap = self.db.root_def_map(self.db.file_source_root(file));
            let d = &base_defmap.defs[base];
            let Some(child_scope) = d.child_scope else {
                continue;
            };
            let s = &base_defmap.scopes[child_scope];
            if let Some(def) = s.by_name.get(name) {
                return match def.defs.len() {
                    1 => Some(Resolution::Def(def.defs[0])),
                    _ => Some(Resolution::Defs(def.defs.clone().into_boxed_slice()))
                };
            }
        }
        None
    }

    fn lookup_in_imports(&self, file: File, name: &Name) -> Option<Resolution> {
        //get imported scopes and lookup imports
        let file_data = if file == self.file { 
            &self.defmap.files[&file] 
        } else { 
            &self.db.root_def_map(self.db.file_source_root(file)).files[&file]
        };
        for scope_entry in file_data.imported_scopes.iter() {
            let defmap = self.db.root_def_map(scope_entry.root);
            let scope = &defmap.scopes[scope_entry.scope];
            if let Some(def) = scope.by_name.get(name) {
                return match def.defs.len() {
                    1 => Some(Resolution::Def(def.defs[0])),
                    _ => Some(Resolution::Defs(def.defs.clone().into_boxed_slice()))
                };
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



    fn infer_expr(&self, expr: ExprId) -> Option<Type> {
        match &self.body()?.exprs[expr] {
            Expr::Ident(name) => {
                let scope = self.sourcemap()?.expr_scopes[expr];
                let offset = self.sourcemap()?.expr_to_node[expr].start;
                let res = self.resolve_name(name, Some(scope), offset)?;
                self.infer_type(res);
            }
            Expr::Literal(l) => {
                return Some(Self::literal_type(l));
            }
            Expr::Member {obj, prop } => {
                let ty = self.infer_expr(*obj)?;
                return self.infer_lookup_type(ty, prop);
            }
            Expr::Array { base, index } => {
                //Array expressions resolve to the base/underlying type, because array exprs are index/access exprs e.g. a[2]
                let ty = self.infer_expr(*base)?;
                let Type::Array(base) = ty else {return None;};
                return Some(*base);
            }
            Expr::Call { callee, args } => {//a(), a.b(), a()() --a returns a fn, a[]() --a is an array of fn pointers
                //call exprs resolve to their return type, but in the event of an overload how dowe resolve to exact?
                let scope = self.sourcemap()?.expr_scopes[*callee];//for fn pointers ie local fns
                let offset = self.sourcemap()?.expr_to_node[*callee].start;
                
                
            }
        }
        None
    }


    fn infer_type(&self, res: Resolution) -> Option<Type> {
        match res {
            Resolution::Local(l) => {
                let local = &self.body()?.locals[l];
                let ty_id = *local.type_name();
                return self.lower_type_name(ty_id);
            }
            Resolution::Def(d) => {
                //TODO get the type of the item
                //we would need itemData. eg if statevar get staevarData and get type
            }
            Resolution::Type(ty) => {
                return Some(Type::UserDefined(ty));

            }
            Resolution::Primitive(p) => {
                return Some(Type::Primitive(p));
            }
            Resolution::Defs(d) => {

            }
            //TODO this is where i'd handle the multi-resolutins ie Defs(Box<[DefId]>)
        }
        None
    }
    

    fn lower_type_name(&self, ty_id: TypeId) -> Option<Type> {
         let ty_name = &self.body()?.type_names[ty_id];
         match ty_name {
            TypeName::Primitive(p) => {
                Some(Type::Primitive(*p))
            }
            TypeName::UserDefined(p) => {
                let res = self.resolve_path(&p.segments)?;
                self.infer_type(res)
            }
            TypeName::Array { ty, size } => {
                let ty = self.lower_type_name(*ty)?;
                Some(Type::Array(Box::new(ty)))
            }
            TypeName::Mapping { key, value } => {
                let key_ty = self.lower_type_name(*key)?;
                let value_ty = self.lower_type_name(*value)?;
                Some(Type::Mapping { key: Box::new(key_ty), value: Box::new(value_ty)})
            }
            TypeName::Fn(f) => {
                let params = f.params.iter().filter_map(|p| {
                    self.lower_type_name(*p)//using filtermap skips types that failed to lower, i want to return none instead
                }).collect::<Box<_>>();
                let ret = f.ret.iter().filter_map(|p| {
                    self.lower_type_name(*p)
                }).collect::<Box<_>>();
                Some(Type::Fn { params, ret })
            }
        }
    }

    fn infer_lookup_type(&self, ty: Type, name: &Name) -> Option<Type> {
        match ty {
            Type::UserDefined(ty) => {
                let res = self.lookup_type(&ty, name)?;
                self.infer_type(res)
            }
            Type::Primitive(p) => {
                // TODO: impl primitive member lookups
                None
            }
            Type::Array(a) => {
                // TODO: impl array members i.e. push/pop etc
                None
            }
            Type::Mapping { key, value } => {
                // Mapping it self shouldn't/doesn't have members i believe
                None
            }
            Type::Fn { params, ret } => None
        }
    }

    fn literal_type(l: &Literal) -> Type {
        match l {
            Literal::Number(_) => Type::Primitive(Primitive::Uint(256)),
            Literal::String(_) => Type::Primitive(Primitive::String),
            Literal::Boolean(_) => Type::Primitive(Primitive::Boolean),
            Literal::HexString(_) => Type::Primitive(Primitive::Bytes),
        }
    }

}


pub struct Inference<'db> {
    db: &'db dyn HirDatabase,
    resolver: Resolver<'db>,
}

impl<'db> Inference<'db> {
    pub fn new(db: &'db dyn HirDatabase, ctx: Context) -> Self {
        let resolver = Resolver::build(db, &ctx);
        Self { db, resolver }
    }

   
}

