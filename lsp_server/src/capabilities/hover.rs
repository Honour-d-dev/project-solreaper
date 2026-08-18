
use anyhow::{Context as _, Ok};
use la_arena::Arena;
use lsp_server::{Request, Response};
use lsp_types::{HoverContents, MarkupContent, MarkupKind, HoverParams};

use crate::ast::{ContractId, EnumId, ErrorId, EventId, FunctionId, FunctionKind, InterfaceId, LibraryId, ModifierId, NodeRange, StructId, VarId};
use crate::hir::body_map::{BodyOwnerId, BodySourceMap, Local, SemanticId, VariableKind};
use crate::hir::exprs::{Expr, Literal};
use crate::hir::item_data::{EnumData, ExprStore, Field, VariantId};
use crate::hir::resolver::{Callable, Context, Resolution, Resolver};
use crate::hir::types::{Mutability, Type, TypeName, Visibility};
use crate::ir::def_map::DefId;
use crate::salsa::{File, HirDatabase, RootDatabase, SalsaDb};
use crate::utilities::{log_info, to_url, to_utf8path};

use super::SemanticCtx;

fn code_block(code: &str) -> String {
    format!("```solidity \n\n{code}\n```")
}

struct HoverInfo {
    title: String,
    signature: Option<String>,
    documentation: Option<String>,
    definition_link: Option<String>,
}

impl HoverInfo {
    fn render(self) -> String {
        let mut parts = vec![format!("### {}\n___\n", self.title)];
        if let Some(signature) = self.signature {
            parts.push(code_block(&signature));
        }
        if let Some(documentation) = self.documentation.filter(|doc| !doc.is_empty()) {
            parts.push(documentation);
        }
        if let Some(link) = self.definition_link {
            let label = if self.title.starts_with("File") { "Go to file" } else { "Go to declaration" };
            parts.push(format!("[{label}]({link})"));
        }
        parts.join("\n\n")
    }

    fn declaration(title: impl Into<String>, body: String, link: Option<String>) -> String {
        Self {
            title: title.into(),
            signature: None,
            documentation: Some(body),
            definition_link: link,
        }.render()
    }
}

struct Hover<'db> {
    db: &'db SalsaDb,
    resolver: Resolver<'db>,
}

impl<'db> Hover<'db> {

    fn hover_target(&self, def_id: &DefId, range: NodeRange) -> anyhow::Result<String> {
        match def_id {
            DefId::Function(id) => {
                let (body_map, sourcemap) = self.db.body_and_source_map(BodyOwnerId::Function(*id));
                self.hover_semantic(range, &body_map.expr_store, Some(&sourcemap), SemanticCtx::local(&body_map.locals)).context("No semantic Id at position")
            }
            DefId::Modifier(id) => {
                let (body_map, sourcemap) = self.db.body_and_source_map(BodyOwnerId::Modifier(*id));
                self.hover_semantic(range, &body_map.expr_store, Some(&sourcemap), SemanticCtx::local(&body_map.locals)).context("No semantic Id at position")
            }
            DefId::Struct(id) => {
                let data = self.db.struct_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::field(&data.fields)).context("No semantic Id at position")
            }
            DefId::Enum(id) => {
                let data = self.db.enum_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::variant(&data)).context("No semantic Id at position")
            }
            DefId::Event(id) => {
                let data = self.db.event_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::local(&data.parameters)).context("No semantic Id at position")
            }
            DefId::Error(id) => {
                let data = self.db.error_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::local(&data.parameters)).context("No semantic Id at position")
            }
            DefId::Udvt(id) => {
                let data = self.db.udvt_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Contract(id) => {
                let data = self.db.contract_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Interface(id) => {
                let data = self.db.interface_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Library(id) => {
                let data = self.db.library_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Var(id) => {
                let data = self.db.var_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Import(id) => {
                let data = self.db.import_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::Using(id) => {
                let data = self.db.using_data(*id);
                self.hover_semantic(range, &data.expr_store, None, SemanticCtx::empty()).context("No semantic Id at position")
            }
            DefId::File(_) => Err(anyhow::anyhow!("file has no semantic hover")),
        }
    }

    fn hover_semantic(&self, range: NodeRange, store: &ExprStore, sourcemap: Option<&BodySourceMap>, ctx: SemanticCtx<'_>) -> Option<String> {
        match store.range_to_semantic.get(&range)? {
            SemanticId::Local(local_id) => {
                let locals = ctx.locals?;
                Some(self.format_local(&locals[*local_id], store))
            }
            SemanticId::Field(field_id) => {
                let fields = ctx.fields?;
                Some(self.format_field(&fields[*field_id], store, self.resolver.file))
            }
            SemanticId::Variant(variant_id) => {
                let enum_data = ctx.enum_data?;
                Some(self.format_variant(enum_data, *variant_id, self.resolver.file))
            }
            SemanticId::Expr(expr_id) => match &store.exprs[*expr_id] {
                Expr::Literal(literal) => Some(self.format_literal(literal)),
                _ => self.resolver.resolve_expr(*expr_id, store, sourcemap).and_then(|res| self.hover_resolution(&res)),
            },
            SemanticId::Type(type_id) => self.hover_type(&store.types[*type_id], &store.types, u8::MAX),
            SemanticId::TypeSegment { ty, segment } => {
                self.hover_type(&store.types[*ty], &store.types, *segment)
            }
        }
    }

    fn hover_def(&self, def_id: &DefId) -> anyhow::Result<String> {
        let body = self.hover_def_body(def_id)?;
        Ok(HoverInfo::declaration(self.def_title(def_id), body, self.definition_link(def_id)))
    }

    fn hover_def_body(&self, def_id: &DefId) -> anyhow::Result<String> {
        match def_id {
            DefId::File(_) => Ok("File".into()),
            DefId::Udvt(id) => {
                let data = self.db.udvt_data(*id);
                Ok(code_block(&format!("type {} is {}", data.name, data.underlying)))
            }
            DefId::Contract(id) => self.hover_contract(id),
            DefId::Library(id) => self.hover_library(id),
            DefId::Interface(id) => self.hover_interface(id),
            DefId::Function(id) => self.hover_function(id),
            DefId::Modifier(id) => self.hover_modifier(id),
            DefId::Struct(id) => self.hover_struct(id),
            DefId::Enum(id) => self.hover_enum(id),
            DefId::Event(id) => self.hover_event(id),
            DefId::Error(id) => self.hover_error(id),
            DefId::Var(id) => self.hover_var(id),
            DefId::Import(_) | DefId::Using(_) => Err(anyhow::anyhow!("declaration has no hover signature")),
        }
    }

    fn hover_var(&self, var_id: &VarId) -> anyhow::Result<String> {
        let var_data = self.db.var_data(*var_id);
        let ty = var_data.expr_store.types[var_data.type_name].to_string(&var_data.expr_store.types);
        let doc = self.db.docs(DefId::Var(*var_id));
        let mut parts = Vec::new();
        parts.push(ty);
        match var_data.kind {
                VariableKind::Const => parts.push("constant".into()),
                VariableKind::Immutable => parts.push("immutable".into()),
                _ => {}
            }
        match var_data.vis {
                Visibility::Public => parts.push("public".into()),
                Visibility::Private => parts.push("private".into()),
                _ => {}
            }
        parts.push(var_data.name.as_str().to_string());
        let sig = parts.join(" ");
        let sig = if var_data.kind == VariableKind::Const {
            self.constant_signature(*var_id, &var_data, &sig)
        } else {
            sig
        };
        Ok(Self::format_with_doc(&sig, doc))
    }

    fn constant_signature(&self, var_id: VarId, var_data: &crate::hir::item_data::VarData, signature: &str) -> String {
        let Some(init) = var_data.init else { return signature.to_string(); };
        let Some(range) = var_data.expr_store.range_to_semantic.iter().find_map(|(range, semantic)| {
            matches!(semantic, SemanticId::Expr(expr) if *expr == init).then_some(*range)
        }) else { return signature.to_string(); };
        let (file, _) = DefId::Var(var_id).file_id();
        let text = self.db.text(file);
        let Some(source) = text.get(range.start as usize..range.end as usize) else {
            return signature.to_string();
        };
        format!("{signature} = {source}")
    }

    fn hover_function(&self, fn_id: &FunctionId) -> anyhow::Result<String> {
        let fn_data = self.db.function_data(*fn_id);
        // FIXME: fn names are lowered as ident. which means if fn name is overloaded the resolver won't be able to figure out the exact fn
        let params = fn_data.parameters.iter()
                    .map(|(_, local)| {
                        let ty = fn_data.expr_store.types[*local.type_name()].to_string(&fn_data.expr_store.types);
                        format!("{} {}", ty, local.name())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = fn_data.return_parameters.iter()
                    .map(|(_, local)| {
                        fn_data.expr_store.types[*local.type_name()].to_string(&fn_data.expr_store.types)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret_str = if ret.is_empty() { String::new() } else { format!(" returns ({ret})") };
                let doc = self.db.docs(DefId::Function(*fn_id));
                let sig = match fn_data.kind {
                    FunctionKind::Regular => format!("function {}({params}) {}{}{ret_str}", fn_data.name, fn_data.vis.as_str(), fn_data.mutability.as_str()),
                    FunctionKind::Constructor => format!("constructor({params}) {}{}", fn_data.vis.as_str(), fn_data.mutability.as_str()),
                    FunctionKind::Fallback => {
                        let mutability = (fn_data.mutability == Mutability::Payable).then_some("payable ").unwrap_or_default();
                        format!("fallback({params}) external {mutability}{ret_str}")
                    }
                    FunctionKind::Receive => "receive() external payable".to_string(),
                };
        Ok(Self::format_with_doc(&sig, doc))
    }

    fn hover_modifier(&self, mod_id: &ModifierId) -> anyhow::Result<String> {
        let mod_data = self.db.modifier_data(*mod_id);
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

    fn hover_struct(&self, struct_id: &StructId) -> anyhow::Result<String> {
        let struct_data = self.db.struct_data(*struct_id);
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

    fn hover_enum(&self, enum_id: &EnumId) -> anyhow::Result<String> {
        let enum_data = self.db.enum_data(*enum_id);
        let variants = enum_data.variants.iter()
                    .map(|(_, v)| v.name.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let doc = self.db.docs(DefId::Enum(*enum_id));
                let sig = format!("enum {} {{ {} }}", enum_data.name, variants);
        Ok(Self::format_with_doc(&sig, doc))
    }

    fn hover_event(&self, event_id: &EventId) -> anyhow::Result<String> {
        let event_data = self.db.event_data(*event_id);
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

    fn hover_error(&self, error_id: &ErrorId) -> anyhow::Result<String> {
        let error_data = self.db.error_data(*error_id);
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

    fn hover_contract(&self, contract_id: &ContractId) -> anyhow::Result<String> {
        let contract_data = self.db.contract_data(*contract_id);
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

    fn hover_interface(&self, interface_id: &InterfaceId) -> anyhow::Result<String> {
        let interface_data = self.db.interface_data(*interface_id);
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

    fn hover_library(&self, library_id: &LibraryId) -> anyhow::Result<String> {
        let library_data = self.db.library_data(*library_id);
        let doc = self.db.docs(DefId::Library(*library_id));
                let sig = format!("library {}", library_data.name);
        Ok(Self::format_with_doc(&sig, doc))
    }

    fn hover_type(&self, ty: &TypeName, types: &Arena<TypeName>, segment: u8) -> Option<String> {
        match ty {
            TypeName::UserDefined(path) => {
                let len = path.segments.len();
                let seg = (segment as usize).min(len.saturating_sub(1));
                let segments = &path.segments[..=seg];
                self.resolver
                    .resolve_path(segments)
                    .and_then(|res| self.hover_resolution(&res))
            }
            _ => Some(HoverInfo {
                title: "Type".into(),
                signature: Some(ty.to_string(types)),
                documentation: None,
                definition_link: None,
            }.render()),
        }
    }

    fn hover_callable(&self, callable: &Callable) -> Option<String> {
        if let Some(def) = callable.def() {
            return self.hover_def(&def).ok();
        }
        let builtin = callable.builtin()?;
        let params = builtin.params.iter().map(|ty| ty.display(self.db)).collect::<Vec<_>>().join(", ");
        let ret = builtin.return_type.as_ref()
            .map(|ty| format!(" -> {}", ty.display(self.db)))
            .unwrap_or_default();
        Some(HoverInfo {
            title: "Builtin_function".into(),
            signature: Some(format!("function {}({}){}", builtin.name, params, ret)),
            documentation: Some(builtin.doc.to_string()),
            definition_link: None,
        }.render())
    }

    fn hover_resolution(&self, res: &Resolution) -> Option<String> {
        match res {
            Resolution::File(f) => {
                let path = f.path(self.db);
                let file_name = path.file_name().unwrap_or_default();
                Some(HoverInfo {
                    title: format!("File: {file_name}"),
                    signature: None,
                    documentation: None,
                    definition_link: Some(to_url(path.as_path()).to_string()),
                }.render())
            }
            Resolution::Local(local_id) => {
                let body = self.resolver.body()?;
                Some(self.format_local(&body.locals[*local_id], &body.expr_store))
            }
            Resolution::Callable(callable) | Resolution::Called(callable) => self.hover_callable(callable),
            Resolution::Var(def) => self.hover_def(def).ok(),
            Resolution::TypeKey(tk) => {
                if let Some(def_id) = tk.def_id() {
                    self.hover_def(&def_id).ok()
                } else {
                    self.format_resolved_type(tk.typ())
                }
            }
            Resolution::Type(ty) => {
                if let Some(def_id) = ty.def_id() {
                    self.hover_def(&def_id).ok()
                } else {
                    self.format_resolved_type(ty)
                }
            }
            Resolution::Callables(callables) => {
                let hovers = callables.iter().filter_map(|callable| self.hover_callable(callable)).collect::<Vec<_>>();
                (!hovers.is_empty()).then(|| hovers.join("\n---\n"))
            },
            Resolution::Variant(enum_id, variant_id) => {
                let enum_data = self.db.enum_data(*enum_id);
                let (file, _) = DefId::Enum(*enum_id).file_id();
                Some(self.format_variant(&enum_data, *variant_id, file))
            }
            Resolution::Field(field) => {
                if let Some((DefId::Struct(struct_id), field_id)) = field.struct_field() {
                    let struct_data = self.db.struct_data(struct_id);
                    let (file, _) = DefId::Struct(struct_id).file_id();
                    Some(self.format_field(&struct_data.fields[field_id], &struct_data.expr_store, file))
                } else {
                    field.builtin().map(|builtin| {
                        Self::format_with_doc(&format!("{} {}", builtin.ty, builtin.name), Some(builtin.doc.into()))
                    })
                }
            }
            Resolution::Builtin(id) => {
                Some(HoverInfo {
                    title: "Builtin".into(),
                    signature: Some(id.name().to_string()),
                    documentation: Some(id.doc().to_string()),
                    definition_link: None,
                }.render())
            }
            Resolution::MetaType(_) => Some(HoverInfo {
                title: "Meta_type".into(),
                signature: Some("type(...)".into()),//we can show more here
                documentation: None,
                definition_link: None,
            }.render()),
            Resolution::Super(def) => self
                .hover_def(def)
                .ok()
                .map(|hover| format!("Inheriance lookup on: \n --- \n {hover}")),
        }
    }

    fn format_local(&self, local: &Local, store: &ExprStore) -> String {
        let ty = store.types[*local.type_name()].to_string(&store.types);
        HoverInfo {
            title: "Local".into(),
            signature: Some(format!("{ty} {} ({} {})", local.name(), local.location(), local.kind())),
            documentation: None,
            definition_link: None,
        }.render()
    }

    fn format_field(&self, field: &Field, store: &ExprStore, file: File) -> String {
        let ty = store.types[field.type_name].to_string(&store.types);
        let doc = self.db.inline_docs(file, field.range);
        HoverInfo {
            title: "Field".into(),
            signature: Some(format!("{ty} {}", field.name)),
            documentation: doc,
            definition_link: Some(self.range_link(file, field.range)),
        }.render()
    }

    fn format_variant(&self, enum_data: &EnumData, variant_id: VariantId, file: File) -> String {
        let variant = &enum_data.variants[variant_id];
        let doc = self.db.inline_docs(file, variant.range);
        HoverInfo {
            title: "Enum_variant".into(),
            signature: Some(format!("enum variant {}.{}", enum_data.name, variant.name)),
            documentation: doc,
            definition_link: Some(self.range_link(file, variant.range)),
        }.render()
    }

    fn format_literal(&self, literal: &Literal) -> String {
        let inferred = match literal.type_key().typ() {
            Type::Literal(value) => value.inferred_type()
                .map(|ty| ty.display(self.db))
                .unwrap_or_else(|| "literal".to_string()),
            _ => "literal".to_string(),
        };
        HoverInfo {
            title: match literal {
                Literal::Boolean(_) => "Boolean_literal",
                Literal::Number(_) => "Number_literal",
                Literal::String(_) => "String_literal",
                Literal::HexString(_) => "Hex_string_literal",
            }.to_string(),
            signature: Some(format!("{} : {}", literal.source_text(), inferred)),
            documentation: None,
            definition_link: None,
        }.render()
    }

    fn format_resolved_type(&self, ty: &Type) -> Option<String> {
        let inferred = match ty {
            Type::Literal(literal) => literal.inferred_type()?,
            _ => ty.clone(),
        };
        Some(HoverInfo {
            title: "type".into(),
            signature: Some(inferred.display(self.db)),
            documentation: None,
            definition_link: None,
        }.render())
    }
    
    fn format_with_doc(sig: &str, doc: Option<String>) -> String {
        match doc {
            Some(d) if !d.is_empty() => format!("{}\n---\n{d}", code_block(sig)),
            _ => code_block(sig),
        }
    }

    fn def_title(&self, def_id: &DefId) -> String {
        match def_id {
            DefId::Function(id) => match self.db.function_data(*id).kind {
                FunctionKind::Regular => "Function",
                FunctionKind::Constructor => "Constructor",
                FunctionKind::Fallback => "Fallback",
                FunctionKind::Receive => "Receive",
            }.into(),
            DefId::Modifier(_) => "Modifier".into(),
            DefId::Struct(_) => "Struct".into(),
            DefId::Enum(_) => "Enum".into(),
            DefId::Event(_) => "Event".into(),
            DefId::Error(_) => "Error".into(),
            DefId::Contract(_) => "Contract".into(),
            DefId::Interface(_) => "Interface".into(),
            DefId::Library(_) => "Library".into(),
            DefId::Udvt(_) => "Type".into(),
            DefId::Var(_) => "Variable".into(),
            DefId::File(_) => "File".into(),
            DefId::Import(_) => "Import".into(),
            DefId::Using(_) => "Using".into(),
        }
    }

    fn definition_link(&self, def_id: &DefId) -> Option<String> {
        let (file, Some(ast_id)) = def_id.file_id() else { return None; };
        let node = self.db.ast_id_map(file).get_node(&self.db.root(file), ast_id)?;
        Some(self.range_link(file, NodeRange::from(&node.node())))
    }

    fn range_link(&self, file: File, range: NodeRange) -> String {
        let path = file.path(self.db);
        let line = self.db.root(file)
            .named_child_node(range)
            .map(|node| node.node().start_position().row + 1)
            .unwrap_or(1);
        let mut uri = to_url(path.as_path());
        uri.set_fragment(Some(&format!("L{line}")));
        uri.to_string()
    }
}




pub fn hover(db: &SalsaDb, request: Request) -> anyhow::Result<Response> {
    let params: HoverParams = serde_json::from_value(request.params)?;
    let path = to_utf8path(&params.text_document_position_params.text_document.uri)?;
    log_info(format!("Hover request for {path}"));

    let position = params.text_document_position_params.position;
    let (file, offset) = db.convert(&path, position);
    let node = db.named_node_at(file, offset).context("No node at cursor")?;
    let ctx = Context::new(db, file, offset);
    let resolver = Resolver::build(db, &ctx);

    let range = NodeRange::from(&node.node());
    let h = Hover { db, resolver };
    let hover_str = h
        .hover_target(&ctx.container, range)
        .or_else(|_| h.hover_def(&ctx.container))?;
    

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



