use anyhow::{Context as _, Result};
use la_arena::Arena;
use lsp_server::{Request, Response};
use lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location as LspLocation, Position, Range, Url,
};
use ropey::Rope;

use crate::ast::NodeRange;
use crate::hir::body_map::{BodyOwnerId, BodySourceMap, SemanticId};
use crate::hir::item_data::{EnumData, ExprStore, Field};
use crate::hir::resolver::{Context, Resolution, Resolver};
use crate::hir::types::{Type, TypeName};
use crate::ir::def_map::DefId;
use crate::salsa::root_db::RootDatabase;
use crate::salsa::{File, HirDatabase, SalsaDb};
use crate::utilities::to_utf8path;

struct SemanticOwner<'a> {
    fields: Option<&'a Arena<Field>>,
    enum_data: Option<&'a EnumData>,
}

enum DefinitionTarget {
    Def(DefId),
    Range(File, NodeRange),
}

pub fn definition(db: &SalsaDb, request: Request) -> Result<Response> {
    let params: GotoDefinitionParams = serde_json::from_value(request.params)?;
    let path = to_utf8path(&params.text_document_position_params.text_document.uri)?;
    let position = params.text_document_position_params.position;
    let (file, offset) = db.convert(&path, position);
    let node = db.node_at(file, offset).context("No node at cursor")?;//we do node_at inside context already
    let ctx = Context::new(db, file, offset);
    let resolver = Resolver::build(db, &ctx);
    let range = NodeRange::from(&node.node());

    let locations = resolve_at(db, &resolver, &ctx, range)
        .into_iter()
        .flat_map(|resolution| locations_for_target(db, resolution))
        .collect::<Vec<_>>();

    let result = match locations.as_slice() {
        [] => serde_json::Value::Null,
        [location] => serde_json::to_value(GotoDefinitionResponse::Scalar(location.clone()))?,
        _ => serde_json::to_value(GotoDefinitionResponse::Array(locations))?,
    };

    Ok(Response {
        id: request.id,
        result: Some(result),
        error: None,
    })
}

fn resolve_at<'db>(
    db: &'db SalsaDb,
    resolver: &Resolver<'db>,
    ctx: &Context,
    range: NodeRange,
) -> Vec<DefinitionTarget> {
    match ctx.container {
        DefId::Function(id) => {
            let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Function(id));
            resolve_semantic(
                db,
                resolver,
                ctx.file,
                body.expr_store.range_to_semantic.get(&range),
                &body.expr_store,
                Some(&sourcemap),
                SemanticOwner {
                    fields: None,
                    enum_data: None,
                },
            )
        }
        DefId::Modifier(id) => {
            let (body, sourcemap) = db.body_and_source_map(BodyOwnerId::Modifier(id));
            resolve_semantic(
                db,
                resolver,
                ctx.file,
                body.expr_store.range_to_semantic.get(&range),
                &body.expr_store,
                Some(&sourcemap),
                SemanticOwner {
                    fields: None,
                    enum_data: None,
                },
            )
        }
        DefId::Struct(id) => {
            let data = db.struct_data(id);
            resolve_semantic(
                db,
                resolver,
                ctx.file,
                data.expr_store.range_to_semantic.get(&range),
                &data.expr_store,
                None,
                SemanticOwner {
                    fields: Some(&data.fields),
                    enum_data: None,
                },
            )
        }
        DefId::Enum(id) => {
            let data = db.enum_data(id);
            resolve_semantic(
                db,
                resolver,
                ctx.file,
                data.expr_store.range_to_semantic.get(&range),
                &data.expr_store,
                None,
                SemanticOwner {
                    fields: None,
                    enum_data: Some(&data),
                },
            )
        }
        DefId::Contract(id) => {
            let data = db.contract_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        DefId::Interface(id) => {
            let data = db.interface_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        DefId::Library(id) => {
            let data = db.library_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        DefId::Var(id) => {
            let data = db.var_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        DefId::Import(id) => {
            let data = db.import_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        DefId::Using(id) => {
            let data = db.using_data(id);
            resolve_item_semantic(db, resolver, ctx.file, range, &data.expr_store)
        }
        _ => Vec::new(),
    }
}

fn resolve_item_semantic<'db>(
    db: &'db SalsaDb,
    resolver: &Resolver<'db>,
    file: File,
    range: NodeRange,
    store: &ExprStore,
) -> Vec<DefinitionTarget> {
    resolve_semantic(
        db,
        resolver,
        file,
        store.range_to_semantic.get(&range),
        store,
        None,
        SemanticOwner {
            fields: None,
            enum_data: None,
        },
    )
}

fn resolve_semantic<'db>(
    db: &'db SalsaDb,
    resolver: &Resolver<'db>,
    file: File,
    semantic: Option<&SemanticId>,
    store: &ExprStore,
    sourcemap: Option<&BodySourceMap>,
    owner: SemanticOwner<'_>,
) -> Vec<DefinitionTarget> {
    let Some(semantic) = semantic else {
        return Vec::new();
    };

    match semantic {
        SemanticId::Local(local_id) => local_definition(file, store, *local_id),
        SemanticId::Field(field_id) => owner
            .fields
            .map(|fields| &fields[*field_id])
            .map(|field| vec![DefinitionTarget::Range(file, field.range)])
            .unwrap_or_default(),
        SemanticId::Variant(variant_id) => owner
            .enum_data
            .map(|data| &data.variants[*variant_id])
            .map(|variant| vec![DefinitionTarget::Range(file, variant.range)])
            .unwrap_or_default(),
        SemanticId::Expr(expr_id) => resolver
            .resolve_expr(*expr_id, store, sourcemap)
            .map(|resolution| match resolution {
                Resolution::Local(local_id) => local_definition(file, store, local_id),
                resolution => resolution_targets(db, resolution),
            })
            .unwrap_or_default(),
        SemanticId::Type(type_id) => type_path(store, *type_id, u8::MAX)
            .and_then(|path| resolver.resolve_path(path))
            .map(|resolution| resolution_targets(db, resolution))
            .unwrap_or_default(),
        SemanticId::TypeSegment { ty, segment } => type_path(store, *ty, *segment)
            .and_then(|path| resolver.resolve_path(path))
            .map(|resolution| resolution_targets(db, resolution))
            .unwrap_or_default(),
    }
}

fn local_definition(
    file: File,
    store: &ExprStore,
    local_id: crate::hir::body_map::LocalId,
) -> Vec<DefinitionTarget> {
    store
        .range_to_semantic
        .iter()
        .find_map(|(range, semantic)| match semantic {
            SemanticId::Local(id) if id == &local_id => {
                Some(vec![DefinitionTarget::Range(file, *range)])
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn type_path(
    store: &ExprStore,
    type_id: la_arena::Idx<TypeName>,
    segment: u8,
) -> Option<&[crate::hir::exprs::Name]> {
    let TypeName::UserDefined(path) = &store.types[type_id] else {
        return None;
    };
    let end = if segment == u8::MAX {
        path.segments.len()
    } else {
        (segment as usize + 1).min(path.segments.len())
    };
    Some(&path.segments[..end])
}

fn resolution_targets(db: &SalsaDb, resolution: Resolution) -> Vec<DefinitionTarget> {
    match resolution {
        Resolution::Def(def) => vec![DefinitionTarget::Def(def)],
        Resolution::Defs(defs) => defs.iter().copied().map(DefinitionTarget::Def).collect(),
        Resolution::Type(ty) | Resolution::MetaType(ty) => ty
            .def_id()
            .map(|def| vec![DefinitionTarget::Def(def)])
            .unwrap_or_default(),
        Resolution::TypeKey(type_key) => type_key
            .def_id()
            .map(|def| vec![DefinitionTarget::Def(def)])
            .unwrap_or_default(),
        Resolution::Field(type_key, field_id) => match type_key.typ() {
            Type::Def(DefId::Struct(struct_id)) => {
                let data = db.struct_data(*struct_id);
                vec![DefinitionTarget::Range(
                    DefId::Struct(*struct_id).file_id().0,
                    data.fields[field_id].range,
                )]
            }
            _ => Vec::new(),
        },
        Resolution::Variant(enum_id, variant_id) => {
            let data = db.enum_data(enum_id);
            vec![DefinitionTarget::Range(
                DefId::Enum(enum_id).file_id().0,
                data.variants[variant_id].range,
            )]
        }
        Resolution::Super(def) => db
            .bases(def)
            .get(1)
            .copied()
            .map(|base| vec![DefinitionTarget::Def(base)])
            .unwrap_or_default(),
        Resolution::Local(_)
        | Resolution::Builtin(_)
        | Resolution::BuiltinField(_)
        | Resolution::BuiltinFn(_) => Vec::new(),
    }
}

fn locations_for_target(db: &SalsaDb, target: DefinitionTarget) -> Option<LspLocation> {
    match target {
        DefinitionTarget::Def(def) => {
            let (file, Some(ast_id)) = def.file_id() else {
                return None;
            };
            let node = db.ast_id_map(file).get_node(&db.root(file), ast_id)?;
            location_for_range(db, file, NodeRange::from(&node.node()))
        }
        DefinitionTarget::Range(file, range) => location_for_range(db, file, range),
    }
}

fn location_for_range(db: &SalsaDb, file: File, range: NodeRange) -> Option<LspLocation> {
    let uri = Url::from_file_path(file.path(db).as_std_path()).ok()?;
    let rope = db.rope(file);
    Some(LspLocation {
        uri,
        range: Range {
            start: byte_to_position(&rope, range.start as usize),
            end: byte_to_position(&rope, range.end as usize),
        },
    })
}

fn byte_to_position(rope: &Rope, byte: usize) -> Position {
    let byte = byte.min(rope.len_bytes());
    let line = rope.byte_to_line(byte);
    let char_index = rope.byte_to_char(byte) - rope.line_to_char(line);
    let line_slice = rope.line(line);
    let character = line_slice.char_to_utf16_cu(char_index);
    Position {
        line: line as u32,
        character: character as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_byte_offsets_to_utf16_positions() {
        let rope = Rope::from_str("αx\nvalue");
        assert_eq!(
            byte_to_position(&rope, 2),
            Position {
                line: 0,
                character: 1
            }
        );
        assert_eq!(
            byte_to_position(&rope, 3),
            Position {
                line: 0,
                character: 2
            }
        );
        assert_eq!(
            byte_to_position(&rope, 4),
            Position {
                line: 1,
                character: 0
            }
        );
    }
}
