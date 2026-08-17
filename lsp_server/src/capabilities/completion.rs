use std::collections::HashSet;

use anyhow::{Context as _, Result};
use lsp_server::{Request, Response};
use lsp_types::{
    CompletionItem as LspCompletionItem, CompletionItemKind, CompletionItemLabelDetails,
    CompletionList, CompletionParams, CompletionResponse, CompletionTriggerKind,
    InsertTextFormat,
};
use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, NodeRange};
use crate::hir::body_map::{BodyOwnerId, Local, SemanticId};
use crate::hir::exprs::Name;
use crate::hir::item_data::ExprStore;
use crate::hir::builtins::{Builtin, BuiltinDB};
use crate::hir::resolver::{Context, Resolution, Resolver};
use crate::hir::types::Primitive;
use crate::ir::def_map::{DefId, Namespace};
use crate::salsa::{HirDatabase, SalsaDb};
use crate::utilities::{log_info, print_tree, to_utf8path};

/// What kind of completion position we're at.
#[derive(Debug)]
enum CompletionContextKind {
    /// Plain identifier in expression position: `foo$0`
    Expr,
    /// After a dot: `obj.$0`
    DotAccess { receiver_node: AstNode },
}

pub fn completion(db: &SalsaDb, request: Request) -> Result<Response> {
    log_info("Completion triggered");
    let params: CompletionParams = serde_json::from_value(request.params)?;
    let path = to_utf8path(&params.text_document_position.text_document.uri)?;
    let position = params.text_document_position.position;

    let trigger_kind = params
        .context
        .as_ref()
        .map(|c| c.trigger_kind)
        .unwrap_or(CompletionTriggerKind::INVOKED);

    let trigger_character = params
        .context
        .as_ref()
        .and_then(|c| c.trigger_character.as_deref());

    let (file, offset) = db.convert(&path, position);
    let node = db.named_node_at(file, offset).context("No node at cursor")?;
    let ctx = Context::new(db, file, offset);
    let resolver = Resolver::build(db, &ctx);
    log_info(format!("completion request for {} : {}", node.node().kind(), print_tree(&mut node.node().walk(), node.ast().source(), 2)));

    let kind = detect_ctx(&node, trigger_kind, trigger_character).context("unable to detect completion context")?;

    let items = match kind {
        CompletionContextKind::DotAccess { receiver_node } => {
            complete_dot(db, &resolver, &ctx, &receiver_node)
        }
        CompletionContextKind::Expr => complete_expr(db, &resolver, &ctx, offset),
    };
    
    let result = CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items: dedupe_items(items),
    });

    Ok(Response {
        id: request.id,
        result: Some(serde_json::to_value(result)?),
        error: None,
    })
}

fn detect_ctx(node: &AstNode, trigger_kind: CompletionTriggerKind, trigger_character: Option<&str>) -> Option<CompletionContextKind> {
    match trigger_kind {
        CompletionTriggerKind::TRIGGER_CHARACTER if trigger_character == Some(".") => {
            extract_receiver(node).map(|r| CompletionContextKind::DotAccess { receiver_node: r })
        }
        _ => {//Invoked
            //is invoked on a member? or we treat as an incomplete expression
            extract_receiver(node).map(|r| CompletionContextKind::DotAccess { receiver_node: r }).or(Some(CompletionContextKind::Expr))
        }
    }
}


// MARK: Dot completions
fn complete_dot(db: &SalsaDb, resolver: &Resolver, ctx: &Context, receiver_node: &AstNode ) -> Vec<LspCompletionItem> {
    let mut acc = Vec::new();

    // Try to resolve the receiver as an expression and infer its type.
    let Some(receiver) = infer_receiver(db, resolver, ctx, receiver_node) else {
        return acc;
    };

    // Member enumeration belongs in the resolver. Keep this completion-side hook
    // explicit until the resolver exposes that API.
    acc.extend(list_members(db, resolver, &receiver));

    acc
}


fn valid_member_expr(node: &tree_sitter::Node) -> bool {
    node.kind_id() == NodeKind::INCOMPLETE_MEMBER_EXPRESSION || node.kind_id() == NodeKind::MEMBER_EXPRESSION
}

fn find_missing_member_property<'a>(node: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    if node.kind_id() == NodeKind::MISSING_MEMBER_PROPERTY || (node.kind_id() == NodeKind::IDENTIFIER && node.is_missing()) {
        return Some(node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(property) = find_missing_member_property(child) {
            return Some(property);
        }
    }
    None
}

/// Extract the receiver node from a complete or incomplete member expression.
fn extract_receiver(node: &AstNode) -> Option<AstNode> {
    let current = node.node();

    let member = find_missing_member_property(current)
        .and_then(|property| property.parent())
        .filter(valid_member_expr)
        .or_else(||valid_member_expr(&current).then_some(current))
        .or_else(||current.parent().filter(valid_member_expr))?;

    member.child_by_field_id(FieldKind::OBJECT.into()).map(|n| node.upcast(n))
}


/// Attempt to infer the type of a receiver expression for dot completion.
/// This is the hard part — for the scaffold we try to resolve via the body map.
fn infer_receiver(db: &SalsaDb, resolver: &Resolver, ctx: &Context, receiver_node: &AstNode ) -> Option<Resolution> {
    let range = NodeRange::from(&receiver_node.node());
    let (body, sourcemap) = match ctx.container {
        DefId::Function(f) => db.body_and_source_map(BodyOwnerId::Function(f)),
        DefId::Modifier(m) => db.body_and_source_map(BodyOwnerId::Modifier(m)),
        _ => return None,
    };

    let SemanticId::Expr(expr_id) = body.expr_store.range_to_semantic.get(&range)? else {
        return None;
    };
    resolver.resolve_expr(*expr_id, &body.expr_store, Some(&sourcemap))
}

fn list_members( db: &SalsaDb, resolver: &Resolver, resolution: &Resolution ) -> Vec<LspCompletionItem> {
    resolver
        .members(resolution.clone())
        .into_iter()
        .filter_map(|member| completion_item_for_resolution(db, member))
        .collect()
}

fn completion_item_for_resolution(db: &SalsaDb, resolution: Resolution) -> Option<LspCompletionItem> {
    match resolution {
        Resolution::Callable(callable) | Resolution::Called(callable) => {
            if let Some(def) = callable.def() {
                let name = match def {
                    DefId::Function(id) => db.function_data(id).name.to_string(),
                    DefId::Modifier(id) => db.modifier_data(id).name.to_string(),
                    _ => return None,
                };
                let (label, insert_text) = callable_label_and_insert(db, &def, &name);
                return Some(LspCompletionItem {
                    label,
                    kind: Some(CompletionItemKind::METHOD),
                    label_details: label_details_for_def(db, &def),
                    documentation: documentation_for_def(db, &def),
                    filter_text: Some(name),
                    insert_text: Some(insert_text),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                });
            }
            let builtin = callable.builtin()?;
            let params = builtin.params.iter().map(|ty| ty.display(db)).collect::<Vec<_>>().join(", ");
            let ret = builtin.return_type.as_ref().map(|ty| format!(" returns {}", ty.display(db))).unwrap_or_default();
            let label = if builtin.params.is_empty() {
                format!("{}()", builtin.name)
            } else {
                format!("{}(\u{2026})", builtin.name)
            };
            Some(LspCompletionItem {
                label,
                kind: Some(CompletionItemKind::METHOD),
                label_details: Some(CompletionItemLabelDetails {
                    detail: Some(" (builtin)".to_string()),
                    description: Some(format!("function ({params}){ret}")),
                }),
                filter_text: Some(builtin.name.to_string()),
                insert_text: Some(format!("{}($0)", builtin.name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                documentation: Some(lsp_types::Documentation::String(builtin.doc.to_string())),
                ..Default::default()
            })
        }
        Resolution::Callables(callables) => {
            callables.into_iter().find_map(|callable| completion_item_for_resolution(db, Resolution::Callable(callable)))
        }
        Resolution::Field(field) => {
            if let Some((DefId::Struct(struct_id), field_id)) = field.struct_field() {
                let data = db.struct_data(struct_id);
                let field_data = &data.fields[field_id];
                let ty = data.expr_store.types[field_data.type_name].to_string(&data.expr_store.types);
                return Some(LspCompletionItem {
                    label: field_data.name.to_string(),
                    kind: Some(CompletionItemKind::FIELD),
                    label_details: Some(CompletionItemLabelDetails {
                        detail: None,
                        description: Some(ty),
                    }),
                    ..Default::default()
                });
            }
            let builtin = field.builtin()?;
            Some(LspCompletionItem {
                label: builtin.name.to_string(),
                kind: Some(CompletionItemKind::FIELD),
                label_details: Some(CompletionItemLabelDetails {
                    detail: Some(" (builtin)".to_string()),
                    description: Some(builtin.ty.to_string()),
                }),
                documentation: Some(lsp_types::Documentation::String(builtin.doc.to_string())),
                ..Default::default()
            })
        }
        Resolution::Variant(enum_id, variant_id) => {
            let data = db.enum_data(enum_id);
            Some(LspCompletionItem {
                label: data.variants[variant_id].name.to_string(),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(format!("{}.{}", data.name, data.variants[variant_id].name)),
                }),
                ..Default::default()
            })
        }
        _ => None,
    }
}


// MARK: Expr completions

fn complete_expr(db: &SalsaDb, resolver: &Resolver, ctx: &Context, offset: u32) -> Vec<LspCompletionItem> {
    let mut acc = Vec::new();

    // 1. Locals (if inside a function/modifier body)
    if let Some(body) = resolver.body() {
        for (_, local) in body.locals.iter().filter(|(_, local)| local.offset() <= offset) {
            if let Some(item) = completion_item_for_local(local, &body.expr_store) {
                acc.push(item);
            }
        }
    }

    // 2. Def-map scope walk (contracts, functions, types, events, errors, vars)
    acc.extend(iterate_scope(resolver, ctx, |name, def_id, _namespace| {
        let kind = completion_kind_for_def(&def_id);
        let (label, insert_text, filter_text) = callable_completion_text(db, &def_id, name);
        let insert_text_format = insert_text.as_ref().map(|_| InsertTextFormat::SNIPPET);
        Some(LspCompletionItem {
            label,
            kind: Some(kind),
            label_details: label_details_for_def(db, &def_id),
            documentation: documentation_for_def(db, &def_id),
            filter_text,
            insert_text,
            insert_text_format,
            ..Default::default()
        })
    }));

    // 3. Builtins (msg, block, tx, abi, this, super, primitives)
    acc.extend(completion_builtins(db));

    // 4. Declaration snippets where the current container permits them.
    acc.extend(complete_item_list(ctx));

    acc
}


fn complete_item_list(ctx: &Context) -> Vec<LspCompletionItem> {
    let keywords = [
        ("contract", "contract $1 {\n\t$0\n}"),
        ("interface", "interface $1 {\n\t$0\n}"),
        ("library", "library $1 {\n\t$0\n}"),
        ("function", "function $1($2) $3 {\n\t$0\n}"),
        ("modifier", "modifier $1($2) {\n\t$0\n}"),
        ("constructor", "constructor($1) {\n\t$0\n}"),
        ("fallback", "fallback() external $1 {\n\t$0\n}"),
        ("receive", "receive() external payable {\n\t$0\n}"),
        ("event", "event $1($2);"),
        ("error", "error $1($2);"),
        ("struct", "struct $1 {\n\t$0\n}"),
        ("enum", "enum $1 {\n\t$0\n}"),
        ("mapping", "mapping($1 => $2) $3;"),
        ("using", "using $1 for $2;"),
        ("type", "type $1 is $2;"),
    ];

    let is_file = matches!(ctx.container, DefId::File(_));
    let is_container = matches!(ctx.container, DefId::Contract(_) | DefId::Interface(_) | DefId::Library(_));
    if !is_file && !is_container {
        return Vec::new();
    }

    keywords.iter().filter(|(keyword, _)| {
        if is_file {
            !matches!(*keyword, "constructor" | "fallback" | "receive" | "modifier" | "mapping")
        } else {
            !matches!(*keyword, "contract" | "interface" | "library")
        }
    })
        .map(|(keyword, snippet)| LspCompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some(snippet.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        })
        .collect()
}


/// Walk the def-map scope chain from the current container upward,
/// calling `f` for each visible name.
fn iterate_scope(
    resolver: &Resolver,
    ctx: &Context,
    mut f: impl FnMut(&Name, DefId, &Namespace) -> Option<LspCompletionItem>,
) -> Vec<LspCompletionItem> {
    let mut acc = Vec::new();
    let mut seen: rustc_hash::FxHashSet<Name> = rustc_hash::FxHashSet::default();

    // Walk def-map scopes from container upward
    let defmap = resolver.def_map(&ctx.container);
    if let Some(data) = defmap.defs.get(&ctx.container) {
        let start = data.child_scope.unwrap_or(data.scope);
        let mut scope_id = start;

        loop {
            let scope = &defmap.scopes[scope_id];
            for (name, scope_data) in &scope.by_name {
                if seen.contains(name) {
                    continue;
                }
                for def_id in &scope_data.defs {
                    if let Some(item) = f(name, *def_id, &scope_data.namespace) {
                        seen.insert(name.clone());
                        acc.push(item);
                        break;
                    }
                }
            }
            scope_id = match scope.parent {
                Some(p) => p,
                None => break,
            };
        }
    }

    acc
}

fn dedupe_items(items: Vec<LspCompletionItem>) -> Vec<LspCompletionItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.label.clone()))
        .collect()
}

/// Map a DefId to an LSP CompletionItemKind.
fn completion_kind_for_def(def_id: &DefId) -> CompletionItemKind {
    match def_id {
        DefId::Function(_) => CompletionItemKind::FUNCTION,
        DefId::Modifier(_) => CompletionItemKind::FUNCTION,
        DefId::Contract(_) => CompletionItemKind::CLASS,
        DefId::Interface(_) => CompletionItemKind::INTERFACE,
        DefId::Library(_) => CompletionItemKind::CLASS,
        DefId::Struct(_) => CompletionItemKind::STRUCT,
        DefId::Enum(_) => CompletionItemKind::ENUM,
        DefId::Event(_) => CompletionItemKind::FUNCTION,
        DefId::Error(_) => CompletionItemKind::FUNCTION,
        DefId::Var(_) => CompletionItemKind::VARIABLE,
        DefId::Udvt(_) => CompletionItemKind::TYPE_PARAMETER,
        DefId::Import(_) => CompletionItemKind::MODULE,
        DefId::Using(_) => CompletionItemKind::KEYWORD,
        DefId::File(_) => CompletionItemKind::FILE,
    }
}

/// Collect NatSpec documentation for a DefId, formatted as markdown for the
/// completion popup sidebar.
fn documentation_for_def(db: &SalsaDb, def_id: &DefId) -> Option<lsp_types::Documentation> {
    db.docs(*def_id).map(|d| {
        lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: d,
        })
    })
}

/// Generate label details for a DefId.
/// `detail` is rendered right after the label (e.g. `(address, uint256)` for functions).
/// `description` is rendered faded at the end (e.g. the containing contract name).
fn label_details_for_def(db: &SalsaDb, def_id: &DefId) -> Option<CompletionItemLabelDetails> {
    match def_id {
        DefId::Function(id) => {
            let data = db.function_data(*id);
            let params = format_param_types(&data.parameters, &data.expr_store);
            let ret = format_return_types(&data.return_parameters, &data.expr_store);
            let mut_str = data.mutability.as_str();
            let ret_str = if ret.is_empty() { String::new() } else { format!(" returns {ret}") };
            Some(CompletionItemLabelDetails {
                detail: Some(format!(" (function)")),
                description: Some(format!("function ({params}) {mut_str}{ret_str}")),
            })
        }
        DefId::Modifier(id) => {
            let data = db.modifier_data(*id);
            let params = format_param_types(&data.parameters, &data.expr_store);
            Some(CompletionItemLabelDetails {
                detail: Some(" (modifier)".to_string()),
                description: Some(format!("modifier ({params})")),
            })
        }
        DefId::Event(id) => {
            let data = db.event_data(*id);
            let params = format_param_types(&data.parameters, &data.expr_store);
            Some(CompletionItemLabelDetails {
                detail: Some(" (event)".to_string()),
                description: Some(format!("event ({params})")),
            })
        }
        DefId::Error(id) => {
            let data = db.error_data(*id);
            let params = format_param_types(&data.parameters, &data.expr_store);
            Some(CompletionItemLabelDetails {
                detail: Some(" (error)".to_string()),
                description: Some(format!("error ({params})")),
            })
        }
        DefId::Var(id) => {
            let data = db.var_data(*id);
            let ty = data.expr_store.types[data.type_name].to_string(&data.expr_store.types);
            Some(CompletionItemLabelDetails {
                detail: Some(format!(" ({})", data.kind)),
                description: Some(ty),
            })
        }
        DefId::Struct(id) => {
            let data = db.struct_data(*id);
            Some(CompletionItemLabelDetails {
                detail: None,
                description: Some(format!("struct {}", data.name)),
            })
        }
        DefId::Enum(id) => {
            let data = db.enum_data(*id);
            Some(CompletionItemLabelDetails {
                detail: None,
                description: Some(format!("enum {}", data.name)),
            })
        }
        DefId::Udvt(id) => {
            let data = db.udvt_data(*id);
            Some(CompletionItemLabelDetails {
                detail: None,
                description: Some(format!("is {}", data.underlying)),
            })
        }
        DefId::Contract(_) => Some(CompletionItemLabelDetails {
            detail: None,
            description: Some("contract".to_string()),
        }),
        DefId::Interface(_) => Some(CompletionItemLabelDetails {
            detail: None,
            description: Some("interface".to_string()),
        }),
        DefId::Library(_) => Some(CompletionItemLabelDetails {
            detail: None,
            description: Some("library".to_string()),
        }),
        _ => None,
    }
}


/// Format parameter types only as `type, type` (for label details).
fn format_param_types(params: &la_arena::Arena<Local>, store: &ExprStore) -> String {
    params.iter()
        .map(|(_, local)| store.types[*local.type_name()].to_string(&store.types))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format return types only as `type, type` (for label details).
fn format_return_types(params: &la_arena::Arena<Local>, store: &ExprStore) -> String {
    params.iter()
        .map(|(_, local)| store.types[*local.type_name()].to_string(&store.types))
        .collect::<Vec<_>>()
        .join(", ")
}


/// Build a completion item for a local variable.
fn completion_item_for_local(local: &Local, store: &ExprStore) -> Option<LspCompletionItem> {
    let ty = store.types[*local.type_name()].to_string(&store.types);
    Some(LspCompletionItem {
        label: local.name().to_string(),
        kind: Some(CompletionItemKind::VARIABLE),
        label_details: Some(CompletionItemLabelDetails {
            detail: Some(ty),
            description: Some(local.kind().to_string()),
        }),
        ..Default::default()
    })
}

/// Build `(label, insert_text)` for a callable def (function/modifier/event/error).
///
/// - Label shows `name(...)` if it has parameters, `name()` otherwise.
/// - `insert_text` is a snippet `name($0)` so the cursor lands inside the parens.
fn callable_label_and_insert(db: &SalsaDb, def_id: &DefId, name: &str) -> (String, String) {
    let has_params = match def_id {
        DefId::Function(id) => !db.function_data(*id).parameters.is_empty(),
        DefId::Modifier(id) => !db.modifier_data(*id).parameters.is_empty(),
        DefId::Event(id) => !db.event_data(*id).parameters.is_empty(),
        DefId::Error(id) => !db.error_data(*id).parameters.is_empty(),
        _ => false,
    };
    let label = if has_params {
        format!("{name}(\u{2026})")
    } else {
        format!("{name}()")
    };
    let insert_text = format!("{name}($0)");
    (label, insert_text)
}

/// Build `(label, insert_text, filter_text)` for a def in the scope walk.
///
/// For callables (functions, modifiers, events, errors): label gets parens,
/// insert_text is a snippet, filter_text is the bare name.
/// For everything else: label is the bare name, no insert_text, no filter_text.
fn callable_completion_text(db: &SalsaDb, def_id: &DefId, name: &Name) -> (String, Option<String>, Option<String>) {
    match def_id {
        DefId::Function(_) | DefId::Modifier(_) | DefId::Event(_) | DefId::Error(_) => {
            let name_str = name.to_string();
            let (label, insert_text) = callable_label_and_insert(db, def_id, &name_str);
            (label, Some(insert_text), Some(name_str))
        }
        _ => (name.to_string(), None, None),
    }
}

/// Builtin globals and primitives for completion.
fn completion_builtins(db: &SalsaDb) -> Vec<LspCompletionItem> {
    let mut acc = Vec::new();
    for global in BuiltinDB::globals() {
        match global {
            Builtin::Obj(object) => acc.push(LspCompletionItem {
                label: object.name.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                label_details: Some(CompletionItemLabelDetails {
                    detail: None,
                    description: Some(object.doc.to_string()),
                }),
                ..Default::default()
            }),
            Builtin::Fn(function) => acc.push(LspCompletionItem {
                label: format!("{}()", function.name),
                kind: Some(CompletionItemKind::FUNCTION),
                label_details: Some(CompletionItemLabelDetails {
                    detail: Some(function.params.iter().map(|ty| ty.display(db)).collect::<Vec<_>>().join(", ")),
                    description: Some(function.doc.to_string()),
                }),
                ..Default::default()
            }),
            Builtin::Field(_) => {}
        }
    }

    // Primitives
    for prim in Primitive::all_primitives() {
        acc.push(LspCompletionItem {
            label: prim.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }

    // this / super
    acc.push(LspCompletionItem {
        label: "this".to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        label_details: Some(CompletionItemLabelDetails {
            detail: None,
            description: Some("Current contract instance".to_string()),
        }),
        ..Default::default()
    });
    acc.push(LspCompletionItem {
        label: "super".to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        label_details: Some(CompletionItemLabelDetails {
            detail: None,
            description: Some("Inherited contract".to_string()),
        }),
        ..Default::default()
    });

    acc
}

