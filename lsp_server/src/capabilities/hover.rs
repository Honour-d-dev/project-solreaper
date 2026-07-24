use std::format;

use anyhow::{Context as _, Ok, anyhow};
use la_arena::Arena;
use lsp_server::{Request, Response};
use lsp_types::{HoverContents, MarkupContent, MarkupKind, HoverParams};

use crate::ast::{ContractId, EnumId, ErrorId, EventId, FunctionId, ImportId, InterfaceId, LibraryId, ModifierId, NodeRange, StructId, VarId};
use crate::hir::body_map::{BodyOwnerId, BodySourceMap, ByteOffset, Local, SemanticId, VariableKind};
use crate::hir::exprs::{Expr, ExprId, Literal, Name};
use crate::hir::item_data::{ExprStore, Field, Variant};
use crate::hir::resolver::{Context, Resolution, Resolver};
use crate::hir::types::{Mutability, Type, TypeName, Visibility};
use crate::ir::def_map::DefId;
use crate::salsa::{HirDatabase, SalsaDb};
use crate::utilities::{log_info, to_utf8path};



struct SemanticCtx<'a> {
    locals: Option<&'a Arena<Local>>,
    fields: Option<&'a Arena<Field>>,
    variants: Option<&'a Arena<Variant>>,
}

impl<'a> SemanticCtx<'a> {
    pub fn empty() -> Self {
        SemanticCtx { locals: None, fields: None, variants: None }
    }

    pub fn local(locals: &'a Arena<Local>) -> Self {
        SemanticCtx { locals: Some(locals), fields: None, variants: None }
    }

    pub fn field(fields: &'a Arena<Field>) -> Self {
        SemanticCtx { locals: None, fields: Some(fields), variants: None }
    }

    pub fn variant(variants: &'a Arena<Variant>) -> Self {
        SemanticCtx { locals: None, fields: None, variants: Some(variants) }
    }
}



struct Hover<'db> {
    db: &'db SalsaDb, 
    resolver: Resolver<'db>,
    offset: ByteOffset
}

impl<'db> Hover<'db> {

    fn hover_semantic(
        &self,
        range: NodeRange,
        store: &ExprStore,
        sourcemap: Option<&BodySourceMap>,
        ctx: SemanticCtx<'_>,
    ) -> Option<String> {
    
        match store.range_to_semantic.get(&range)? {
            SemanticId::Local(local_id) => {
                let locals = ctx.locals?;
                let local = &locals[*local_id];
                let ty = store.types[*local.type_name()].to_string(&store.types);
                Some(Self::code_block(&format!("{ty} {} ({} {})", local.name(), local.location(), local.kind())))
            }
            SemanticId::Field(field_id) => {
                let fields = ctx.fields?;
                let field = &fields[*field_id];
                let ty = store.types[field.type_name].to_string(&store.types);
                Some(Self::code_block(&format!("{} {}", ty, field.name)))
            }
            SemanticId::Variant(variant_id) => {
                let variants = ctx.variants?;
                let variant = &variants[*variant_id];
                Some(Self::code_block(&format!("enum variant {}", variant.name)))//just return the variant name . the enum data will add the enum name and format in codeblock
            }
            SemanticId::Expr(expr_id) => {
                Some(self.hover_expr(*expr_id, store, sourcemap))
            }
            SemanticId::Type(type_id) => {
                Some(self.hover_type(&store.types[*type_id], &store.types, u8::MAX))
            }
            SemanticId::TypeSegment { ty, segment } => {
                Some(self.hover_type(&store.types[*ty], &store.types, *segment))
            }
        }
    }

    fn hover_def(&self, def_id: &DefId, range: Option<NodeRange>) -> anyhow::Result<String> {
        match def_id {
            DefId::File(_) => Ok("file".into()),
            DefId::Import(id) => self.hover_import(id, range),
            DefId::Contract(id) => self.hover_contract(id, range),
            DefId::Library(id) => self.hover_library(id, range),
            DefId::Interface(id) => self.hover_interface(id, range),
            DefId::Function(id) => self.hover_function(id, range),
            DefId::Modifier(id) => self.hover_modifier(id, range),
            DefId::Struct(id) => self.hover_struct(id, range),
            DefId::Enum(id) => self.hover_enum(id, range),
            DefId::Event(id) => self.hover_event(id, range),
            DefId::Error(id) => self.hover_error(id, range),
            DefId::Var(id) => self.hover_var(id, range),
        }
    }

    fn hover_var(&self, var_id: &VarId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let var_data = self.db.var_data(*var_id); 
        match range {
            Some(range) => {
                self.hover_semantic(range, &var_data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            None => {
                let ty = var_data.expr_store.types[var_data.type_name].to_string(&var_data.expr_store.types);
                //bind  defid,  collect doc here and pass to format_var_decl
                let doc = self.db.docs(DefId::Var(*var_id));
                Ok(Self::format_var_declaration(&var_data.name, &ty, &var_data.vis, &var_data.kind, doc))
            }
        }
    }

    fn hover_function(&self, fn_id: &FunctionId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let fn_data = self.db.function_data(*fn_id);
        let (body_map, sourcemap) = self.db.body_and_source_map(BodyOwnerId::Function(*fn_id));
        match range {
            Some(range) => {
                self.hover_semantic(range, &body_map.expr_store, Some(&sourcemap), SemanticCtx::local(&body_map.locals)).context("No semantic Id at position")
            }
            None => {
                let params = fn_data.arg_params.iter()
                    .map(|id| {
                        let local = &fn_data.parameters[*id];
                        let ty = fn_data.expr_store.types[*local.type_name()].to_string(&fn_data.expr_store.types);
                        format!("{} {}", ty, local.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = fn_data.ret_params.iter()
                    .map(|id| {
                        let local = &fn_data.parameters[*id];
                        fn_data.expr_store.types[*local.type_name()].to_string(&fn_data.expr_store.types)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret_str = if ret.is_empty() { String::new() } else { format!(" returns ({ret})") };
                let vis_str = match fn_data.vis {
                    Visibility::Public => "public ",
                    Visibility::Private => "private ",
                    Visibility::External => "external ",
                    Visibility::Internal => "",
                };
                let mut_str = match fn_data.mutability {
                    Mutability::Payable => "payable ",
                    Mutability::View => "view ",
                    Mutability::Pure => "pure ",
                    Mutability::NonPayable => "",
                };
                let doc = self.db.docs(DefId::Function(*fn_id));
                let sig = format!("function {}({params}) {}{}{ret_str}", fn_data.name, vis_str, mut_str);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_modifier(&self, mod_id: &ModifierId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let mod_data = self.db.modifier_data(*mod_id);
        let (body_map, sourcemap) = self.db.body_and_source_map(BodyOwnerId::Modifier(*mod_id));
        match range {
            Some(range) => {
                self.hover_semantic(range, &body_map.expr_store, Some(&sourcemap), SemanticCtx::local(&body_map.locals)).context("No semantic Id at position")
            }
            None => {
                let params = mod_data.parameters.iter()
                    .map(|(_, local)| {
                        let ty = mod_data.expr_store.types[*local.type_name()].to_string(&mod_data.expr_store.types);
                        format!("{} {}", ty, local.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Modifier(*mod_id));
                let sig = format!("modifier {}({params})", mod_data.name);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_struct(&self, struct_id: &StructId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let struct_data = self.db.struct_data(*struct_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &struct_data.expr_store, None, SemanticCtx::field(&struct_data.fields)).context("No semantic Id at position")
            }
            None => {
                let fields = struct_data.fields.iter()
                    .map(|(_, field)| {
                        let ty = struct_data.expr_store.types[field.type_name].to_string(&struct_data.expr_store.types);
                        format!("{} {}", ty, field.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Struct(*struct_id));
                let sig = format!("struct {} {{ {} }}", struct_data.name, fields);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_enum(&self, enum_id: &EnumId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let enum_data = self.db.enum_data(*enum_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &enum_data.expr_store, None, SemanticCtx::variant(&enum_data.variants)).context("No semantic Id at position")
            }
            None => {
                let variants = enum_data.variants.iter()
                    .map(|(_, v)| v.name.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Enum(*enum_id));
                let sig = format!("enum {} {{ {} }}", enum_data.name, variants);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_event(&self, event_id: &EventId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let event_data = self.db.event_data(*event_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &event_data.expr_store, None, SemanticCtx::local(&event_data.parameters)).context("No semantic Id at position")
            }
            None => {
                let params = event_data.parameters.iter()
                    .map(|(_, p)| {
                        let ty = event_data.expr_store.types[*p.type_name()].to_string(&event_data.expr_store.types);
                        format!("{} {}", ty, p.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Event(*event_id));
                let sig = format!("event {}({params})", event_data.name);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_error(&self, error_id: &ErrorId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let error_data = self.db.error_data(*error_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &error_data.expr_store, None, SemanticCtx::local(&error_data.parameters)).context("No semantic Id at position")
            }
            None => {
                let params = error_data.parameters.iter()
                    .map(|(_, p)| {
                        let ty = error_data.expr_store.types[*p.type_name()].to_string(&error_data.expr_store.types);
                        format!("{} {}", ty, p.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Error(*error_id));
                let sig = format!("error {}({params})", error_data.name);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_contract(&self, contract_id: &ContractId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let contract_data = self.db.contract_data(*contract_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &contract_data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            None => {
                let doc = self.db.docs(DefId::Contract(*contract_id));
                let bases = contract_data.bases.iter()
                    .map(|ty_id| contract_data.expr_store.types[*ty_id].to_string(&contract_data.expr_store.types))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sig = if bases.is_empty() {
                    format!("contract {}", contract_data.name)
                } else {
                    format!("contract {} is {}", contract_data.name, bases)
                };
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_interface(&self, interface_id: &InterfaceId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let interface_data = self.db.interface_data(*interface_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &interface_data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            None => {
                let doc = self.db.docs(DefId::Interface(*interface_id));
                let bases = interface_data.bases.iter()
                    .map(|ty_id| interface_data.expr_store.types[*ty_id].to_string(&interface_data.expr_store.types))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sig = if bases.is_empty() {
                    format!("interface {}", interface_data.name)
                } else {
                    format!("interface {} is {}", interface_data.name, bases)
                };
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_library(&self, library_id: &LibraryId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let library_data = self.db.library_data(*library_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &library_data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            None => {
                let doc = self.db.docs(DefId::Library(*library_id));
                let sig = format!("library {}", library_data.name);
                Ok(Self::format_with_doc(&sig, doc))
            }
        }
    }

    fn hover_import(&self, import_id: &ImportId, range: Option<NodeRange>) -> anyhow::Result<String> {
        let import_data = self.db.import_data(*import_id);
        match range {
            Some(range) => {
                self.hover_semantic(range, &import_data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            None => {
                Err(anyhow!("no symbol"))
            }
        }
    }

    fn hover_type(&self, ty: &TypeName, types: &Arena<TypeName>, segment: u8) -> String {
        match ty {
            TypeName::Primitive(p) => Self::code_block(&p.to_string()),
            TypeName::UserDefined(path) => {
                let len = path.segments.len();
                let seg = (segment as usize).min(len.saturating_sub(1));
                let segments = &path.segments[..=seg];
                let name = segments.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(".");
                if let Some(resolution) = self.resolver.resolve_path(segments) {
                    match resolution {
                        Resolution::Type(def_id) | Resolution::Def(def_id) => {
                            self.hover_def(&def_id, None).unwrap_or_else(|_| Self::code_block(&name))
                        }
                        Resolution::Primitive(p) => Self::code_block(&p.to_string()),
                        _ => Self::code_block(&name),
                    }
                } else {
                    Self::code_block(&name)
                }
            }
            _ => Self::code_block(&ty.to_string(types)),
        }
    }

    fn hover_expr(
        &self,
        expr_id: ExprId,
        store: &ExprStore,
        sourcemap: Option<&BodySourceMap>,
    ) -> String {
        match &store.exprs[expr_id] {
            Expr::Ident(name) => {
                let scope = sourcemap.and_then(|sm| sm.expr_scopes.get(expr_id).copied());
                match self.resolver.resolve_name(name, scope, self.offset) {
                    Some(res) => self.hover_resolution(&res),
                    None => format!("### unresolved: {name}"),
                }
            }
            Expr::Path(p) => {
                let Some(f) = self.db.resolve_path(self.resolver.file, p) else {return "".into();};
                let path = f.path(self.db);
                let file_name = path.file_name().unwrap_or_default();
                format!("### File: {file_name} \n---\n [{path}](file://{path})")            }
            Expr::Literal(lit) => {
                match lit {
                    Literal::Boolean(b) => Self::code_block(&format!("bool {b}")),
                    Literal::String(s) => Self::code_block(&format!("string {s}")),
                    Literal::Number(n) => Self::code_block(&format!("uint256 {n}")),
                    Literal::HexString(h) => Self::code_block(&format!("bytes {h}")),
                }
            }
            Expr::Binary { op, left, right } => {
                let l = self.format_expr(*left, &store.exprs, &store.types);
                let r = self.format_expr(*right, &store.exprs, &store.types);
                format!(
                    "Binary Expression \n---\n {}",
                    Self::code_block(&format!("{l} {} {r}", op.as_str()))
                )
            }
            Expr::Member { obj, prop } => {
                let obj_ty = self.resolver.infer_expr(*obj, store, sourcemap);
                match obj_ty {
                    Some(Type::UserDefined(def)) => {
                        match self.resolver.lookup_name(&def, prop) {
                            Some(res) => self.hover_resolution(&res),
                            None => {
                                let obj_str = self.format_expr(*obj, &store.exprs, &store.types);
                                Self::code_block(&format!("{obj_str}.{prop}"))
                            }
                        }
                    }
                    _ => {//@TODO add member support for fn/builtin types like array,mapping,primitives
                        let obj_str = self.format_expr(*obj, &store.exprs, &store.types);
                        Self::code_block(&format!("{obj_str}.{prop}"))
                    }
                }
            }
            Expr::Call { callee, args } => {
                let res = self.resolver.resolve_callee(*callee, store, sourcemap);
                match res {
                    Some(Resolution::Def(def)) => {
                        self.hover_def(&def, None).unwrap_or_else(|_| "### Unknown def".into())
                    }
                    Some(Resolution::Defs(candidates)) => {
                        let arg_types: Vec<Type> = args.iter()
                            .filter_map(|a| self.resolver.infer_expr(*a, store, sourcemap))
                            .collect();
                        if arg_types.len() == args.len() {
                            if let Some(def) = self.resolver.resolve_overload(&candidates[..], &arg_types) {
                                self.hover_def(&def, None).unwrap_or_else(|_| "### Unknown def".into())
                            } else {
                                self.hover_def_candidates(&candidates)
                            }
                        } else {
                            self.hover_def_candidates(&candidates)
                        }
                    }
                    _ => {
                        Self::code_block(&self.format_expr(*callee, &store.exprs, &store.types))
                    }
                }
            }
            Expr::Array { base, index: _ } => {
                Self::code_block(&self.format_expr(*base, &store.exprs, &store.types))
            }
        }
    }

    fn hover_resolution(&self, res: &Resolution) -> String {
        match res {
            Resolution::Local(local_id) => {
                if let Some(body) = self.resolver.body() {
                    let local = &body.locals[*local_id];
                    let ty = body.expr_store.types[*local.type_name()].to_string(&body.expr_store.types);
                    Self::code_block(&format!("{ty} {} ({} {})", local.name(), local.location(), local.kind()))
                } else {
                    Self::code_block("local variable")//no need
                }
            }
            Resolution::Def(def_id) => self.hover_def(def_id, None).unwrap_or_else(|_| "### Unknown def".into()),
            Resolution::Primitive(p) => Self::code_block(&p.to_string()),
            Resolution::Type(def_id) => self.hover_def(def_id, None).unwrap_or_else(|_| "### Unknown def".into()),
            Resolution::Defs(defs) => self.hover_def_candidates(defs),
            Resolution::Variant(enum_id, variant_id) => {
                let enum_data = self.db.enum_data(*enum_id);
                let variant = &enum_data.variants[*variant_id];
                Self::code_block(&format!("enum variant {}.{}", enum_data.name, variant.name))
            }
            Resolution::Field(struct_id, field_id) => {
                let struct_data = self.db.struct_data(*struct_id);
                let field = &struct_data.fields[*field_id];
                let ty = struct_data.expr_store.types[field.type_name].to_string(&struct_data.expr_store.types);
                Self::code_block(&format!("{ty} {}", field.name))
            }
        }
    }

    fn format_expr(
        &self,
        expr_id: ExprId,
        exprs: &Arena<Expr>,
        types: &Arena<TypeName>,
    ) -> String {
        match &exprs[expr_id] {
            Expr::Ident(name) => name.to_string(),
            Expr::Literal(lit) => {
                match lit {
                    Literal::Boolean(b) => b.to_string(),
                    Literal::String(s) => s.to_string(),
                    Literal::Number(n) => n.to_string(),
                    Literal::HexString(h) => h.to_string(),
                }
            }
            Expr::Path(p) => {
                let Some(f) = self.db.resolve_path(self.resolver.file, p) else {return "".into();};
                let path = f.path(self.db);
                let file_name = path.file_name().unwrap_or_default();
                format!("### File: {file_name} \n---\n @ {path}")
            }
            Expr::Binary { op, left, right } => {
                let l = self.format_expr(*left, exprs, types);
                let r = self.format_expr(*right, exprs, types);
                format!("{l} {} {r}", op.as_str())
            }
            Expr::Member { obj, prop } => {
                let obj_str = self.format_expr(*obj, exprs, types);
                format!("{obj_str}.{prop}")
            }
            Expr::Call { callee, args: _ } => {
                self.format_expr(*callee, exprs, types)
            }
            Expr::Array { base, index: _ } => {
                self.format_expr(*base, exprs, types)
            }
        }
    }

    fn format_var_declaration(name: &Name, ty: &str, vis: &Visibility, kind: &VariableKind, docs: Option<String>) -> String {
        let mut parts = Vec::new();
        parts.push(ty.to_string());
        match kind {
            VariableKind::Const => parts.push("constant".into()),
            VariableKind::Immutable => parts.push("immutable".into()),
            _ => {}
        }
        match vis {
            Visibility::Public => parts.push("public".into()),
            Visibility::Private => parts.push("private".into()),
            _ => {}
        }
        parts.push(name.as_str().to_string());
        let sig = parts.join(" ");
        Self::format_with_doc(&sig, docs)
    }

    fn hover_def_candidates(&self, candidates: &[DefId]) -> String {
        candidates.iter().filter_map(|d| {
            self.hover_def(d, None).ok()
        }).collect::<Vec<_>>().join("\n---\n")
    }
    
    fn format_with_doc(sig: &str, doc: Option<String>) -> String {
        match doc {
            Some(d) if !d.is_empty() => format!("{}\n---\n{d}", Self::code_block(sig)),
            _ => Self::code_block(sig),
        }
    }
    
    fn code_block(code: &str) -> String {
        format!("```solidity\n{code}\n```")
    }
}




pub fn hover(db: &SalsaDb, request: Request) -> anyhow::Result<Response> {
    let params: HoverParams = serde_json::from_value(request.params)?;
    let path = to_utf8path(&params.text_document_position_params.text_document.uri)?;
    log_info(format!("Hover request for {path}"));

    let position = params.text_document_position_params.position;
    let (file, offset) = db.convert(&path, position);
    let node = db.node_at(file, offset).context("No node at cursor")?;
    let ctx = Context::new(db, file, offset);
    let resolver = Resolver::build(db, &ctx);

    let range = NodeRange::from(&node.node());
    let h = Hover { db, resolver, offset };
    let hover_str = h.hover_def(&ctx.container, Some(range))?;
    

    let result = lsp_types::Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("\n\n {hover_str} \n\n"),
        }),
        range: None,
    };

    Ok(Response {
        id: request.id,
        result: Some(serde_json::to_value(&result)?),
        error: None,
    })
}



