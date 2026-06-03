use std::mem;

use camino::Utf8PathBuf;
use tree_sitter::{Node, Range, Tree, TreeCursor};

use crate::cursor::{Scope, ScopeBuilder, ScopeId, ScopeNavigator, ScopeType};
use crate::utilities::{iterate_children, resolve_import};
use crate::workspace::Remapping;

/// High-level IR for one source unit.
///
/// This module is intentionally standalone and not yet wired into the request
/// handling pipeline.
#[derive(Debug,Clone,PartialEq, Eq)]
pub(crate) struct File {//rename to prsedFile
    pub path: Utf8PathBuf,
    pub imports: Vec<Import>,
    pub contracts: Vec<Contract>,
    pub free_functions: Vec<Function>,
    pub diagnostics: Vec<LoweringDiagnostic>,
    pub scope: Scope,
}

/// Technically Solidity supports 4 import types
/// 1. Global Import (Imports everything into the global scope)
/// import "./MyContract.sol";
/// 
/// 2. Named/Specific Import (Recommended Best Practice)
/// import {ERC20, Ownable as MyOwnable} from "./MyContract.sol";
/// 
/// 3. Namespace/Alias Import (Alternative A)
/// import * as MySymbols from "./MyContract.sol";
/// 
/// 4. Module-Level Alias Import (Alternative B)
/// import "./MyContract.sol" as MySymbols;
/// 
/// But (3) & (4) are basically the same thing

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Import {
    pub path: Utf8PathBuf,
    pub info: ImportType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportType {
    Full,//can i use full for namespace? namespace is just full with an alias
    Named {
        symbols: Vec<ImportItem>,
    },
    Namespace {
        alias: String
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Symbol {//i think symbols should have parents eg a block/func/contract/None(file)
    Contract(Contract),
    Function(Function),
    Variable(Variable),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Contract {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub bases: Vec<String>,
    pub state_vars: Vec<Variable>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Function {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub parameters: Vec<Variable>,
    pub local_vars: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Variable {
    pub name: Option<String>,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub kind: VariableKind,
    pub typ: VariableType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VariableKind {
    State,
    Local,
    Parameter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VariableType {
    Primitive(PrimitiveType),
    UserDefined(String),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimitiveType {
    Int,
    Uint,
    Bool,
    Address,
    String,
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoweringDiagnostic {
    pub message: String,
    pub range: Range,
}

///@TODO Implement Incremental Lowering - this is still one of the most costly operations
pub(crate) fn lower(
    file_path: &Utf8PathBuf,
    source: &str,
    tree: &Tree,
    project_root: &camino::Utf8Path,
    remappings: &[Remapping],
) -> File {
    let mut lowerer = Lowerer::new(file_path, source, project_root, remappings, false);
    lowerer.lower_source_unit(&mut tree.root_node().walk());
    lowerer.finish()
}

/// Only lower globals - no local variable or function body
pub(crate) fn summarized_lower(file_path: &Utf8PathBuf, source: &str, tree: &Tree, project_root: &camino::Utf8Path, remappings: &[Remapping]) -> File {
    let mut lowerer = Lowerer::new(file_path, source, project_root, remappings, true);
    lowerer.lower_source_unit(&mut tree.root_node().walk());
    lowerer.finish()
}

struct Lowerer<'src> {
    file: File,
    source: &'src str,
    file_path: &'src Utf8PathBuf,
    project_root: &'src camino::Utf8Path,
    remappings: &'src [Remapping],
    scope_builder: ScopeBuilder,
    summarize: bool,
}

impl<'src> Lowerer<'src> {
    fn new(
        file_path: &'src Utf8PathBuf,
        source: &'src str,
        project_root: &'src camino::Utf8Path,
        remappings: &'src [Remapping],
        summarize: bool,
    ) -> Self {
        Self {
            source,
            file_path,
            project_root,
            remappings,
            file: File {
                path: file_path.clone(),
                imports: Vec::new(),
                contracts: Vec::new(),
                free_functions: Vec::new(),
                diagnostics: Vec::new(),
                scope: Scope::new(),
            },
            scope_builder: Scope::new().build(),
            summarize,
        }
    }

    fn finish(mut self) -> File {
        self.file.scope = self.scope_builder.finish();
        self.file
    }

    fn lower_source_unit(&mut self, cursor: &mut TreeCursor<'_>) {
        let mut pending_doc = String::new();

        iterate_children!(cursor, {
            let child = cursor.node();
            match child.kind() {
                "comment" => {
                    self.maybe_collect_docs(child, &mut pending_doc);
                }
                "import_directive" => {
                    pending_doc.clear();
                    self.lower_import(cursor);
                }
                "contract_declaration" => {
                    let contract =
                        self.lower_contract(cursor, mem::take(&mut pending_doc));
                    self.file.contracts.push(contract);
                }
                "function_definition" => {
                    let function =
                        self.lower_function(cursor, mem::take(&mut pending_doc));
                    self.file.free_functions.push(function);
                }
                _ => {//@TODO add error, struct , enum
                    pending_doc.clear();
                }
            }
        });
    }

    fn lower_import(&mut self, cursor: &mut TreeCursor<'_>) {
        let mut path = Utf8PathBuf::new();
        let mut name = String::new();
        let mut alias = String::new();
        let mut symbols: Vec<ImportItem> = vec![];
        let mut is_alias = false;
        iterate_children!(cursor, {
            if cursor.node().kind() == "identifier" {

                //can't determine import shape at this point so we optimistically push import items
                if is_alias {
                    alias = self.node_text(&cursor.node()).to_string();
                    if !symbols.is_empty() {
                        symbols.last_mut().unwrap().alias = Some(alias.clone());
                    }
                    //consume alias flag
                    is_alias = false;
                } else {
                    name = self.node_text(&cursor.node()).to_string();
                    symbols.push(ImportItem { name: name.clone(), alias: None });
                }

            }

            if cursor.node().kind() == "as" {
                //prepare for incoming alias identifier
                is_alias = true;
            }

            if cursor.node().kind() == "string" {
                let relative = self.node_text(&cursor.node()).trim_matches(['"', '\'']);
                if relative.is_empty() {
                    self.push_diagnostic("import directive without path", cursor.node().range());
                } else {
                    match resolve_import(self.file_path, relative, self.project_root, self.remappings) {
                        Ok(p) => path = p,
                        Err(err) => self.push_diagnostic(
                            format!("failed to resolve import `{relative}`: {err:#}"),
                            cursor.node().range(),
                        ),
                    }
                }
            }
        });

        // import {x} from "./x.sol"; =         name & !alias
        // import {x as y} from "./x.sol"; =    name & alias
        // import {x as y, z} from "./x.sol"; = name & alias + name & !alias +....
        // import * as x from "./x.sol"; =     !name & alias
        // import "./x.sol" as x; =            !name & alias
        // import "./x.sol"; =                 !name & !alias
        if !path.as_str().is_empty() {
            let info = match (name, alias) {
                (name, alias) if !name.is_empty()  => ImportType::Named { symbols },
                (name, alias) if name.is_empty() && alias.is_empty() => ImportType::Full,
                (name, alias) if name.is_empty() && !alias.is_empty() => ImportType::Namespace { alias },
                _ => ImportType::Full,//never reached, match is exhaustive - just to satisfy exhaustive requrement else compiler complains
            };

            self.file.imports.push(Import {
                path,
                info,
            });
        }
        

    }

    fn lower_contract(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Contract {
        self.scope_builder
            .to_next(cursor.node().range(), ScopeType::Contract);
        let contract_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut bases = Vec::new();
        let mut state_vars = Vec::new();
        let mut functions = Vec::new();
        let mut body_docs = String::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "inheritance_specifier" => {
                    iterate_children!(cursor, {
                        if cursor.node().kind() == "user_defined_type" {
                            bases.push(self.node_text(&cursor.node()).to_string());
                        }
                    });
                }
                "contract_body" => {
                    iterate_children!(cursor, {
                        match cursor.node().kind() {
                            "comment" => {
                                self.maybe_collect_docs(cursor.node(), &mut body_docs);
                            }
                            //ideally if summarize we should lower private fns and vars
                            //but current impl doesnt trivially support that
                            "state_variable_declaration" => {
                                let state_var = self.lower_variable(
                                    cursor,
                                    VariableKind::State,
                                    contract_scope,
                                    mem::take(&mut body_docs),
                                );
                                state_vars.push(state_var);
                            }
                            "function_definition" => {
                                let function = self.lower_function(
                                    cursor,
                                    mem::take(&mut body_docs),
                                );
                                functions.push(function);
                            }
                            _ => {//@TODO add error, struct, enum
                                body_docs.clear();
                            }
                        }
                    });
                }
                _ => {}
            }
        });

        //@TODO might be unnecessary - will contract be in tree if no identifier??
        if name.is_empty() {
            self.push_diagnostic("contract declaration without identifier", cursor.node().range());
        }

        
        self.scope_builder.to_parent();
        Contract {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: contract_scope,
            bases,
            state_vars,
            functions,
        }
    }

    fn lower_function(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Function {
        self.scope_builder
            .to_next(cursor.node().range(), ScopeType::Function);
        let function_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut parameters = Vec::new();
        let mut local_vars = Vec::new();
        let mut body_docs = String::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "parameter" if !self.summarize => {
                    let parameter = self.lower_variable(
                        cursor,
                        VariableKind::Parameter,
                        function_scope,
                        String::new(),
                    );
                    parameters.push(parameter);
                }
                "function_body" if !self.summarize => {
                    self.collect_local_vars(
                        cursor,
                        &mut local_vars,
                        &mut body_docs,
                    );
                }
                _ => {}
            }
        });

        let _ = self.scope_builder.to_parent();

        Function {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: function_scope,//@NOTE do i add a decl_scope and body_scope? this is body scope though
            parameters,
            local_vars,
        }
    }

    fn collect_local_vars(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        local_vars: &mut Vec<Variable>,
        pending_docs: &mut String,
    ) {
        iterate_children!(cursor, {
            match cursor.node().kind() {
                "comment" => {
                    self.maybe_collect_docs(cursor.node(), pending_docs);
                }
                "variable_declaration_statement" => {
                    self.lower_variable_declaration_statement(
                        cursor,
                        VariableKind::Local,
                        local_vars,
                        self.scope_builder.current(),
                        pending_docs,
                    );
                }
                "statement" => {
                    self.collect_local_vars(cursor, local_vars, pending_docs);
                }
                "for_statement"
                | "block_statement"
                | "while_statement"
                | "do_while_statement"
                | "if_statement"
                | "unchecked_statement"
                | "try_statement"
                | "catch_clause" => {
                    self.collect_local_vars_in_block_scope(
                        cursor,
                        local_vars,
                        pending_docs,
                    );
                }
                _ => {}
            }
        });
    }

    fn collect_local_vars_in_block_scope(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        local_vars: &mut Vec<Variable>,
        pending_docs: &mut String,
    ) {
        self.scope_builder
            .to_next(cursor.node().range(), ScopeType::Block);
        self.collect_local_vars(cursor, local_vars, pending_docs);
        let _ = self.scope_builder.to_parent();
    }

    fn lower_variable_declaration_statement(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        kind: VariableKind,
        out: &mut Vec<Variable>,
        scope: ScopeId,
        pending_docs: &mut String,
    ) {
        iterate_children!(cursor, {
            if cursor.node().kind() == "variable_declaration" {
                let var = self.lower_variable(cursor, kind, scope, mem::take(pending_docs));
                out.push(var);
            }
        });
    }

    /// Should be called on variable_declaration | state_variable_declaration | parameter.
    fn lower_variable(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        kind: VariableKind,
        scope: ScopeId,
        docs: String,
    ) -> Variable {
        let mut name = None;
        let mut typ = VariableType::Unknown;

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" if name.is_none() => {
                    name = Some(self.node_text(&cursor.node()).to_string());
                }
                "type_name" => {
                    let full_type = self.node_text(&cursor.node()).to_string();
                    typ = self
                        .extract_type_ref(cursor)
                        .unwrap_or(VariableType::UserDefined(full_type));
                }
                _ => {}
            }
        });

        Variable {
            name,
            docs,
            signature: self.node_text(&cursor.node()).to_string(),
            range: cursor.node().range(),
            scope,
            kind,
            typ,
        }
    }

    fn extract_type_ref(&self, cursor: &mut TreeCursor<'_>) -> Option<VariableType> {
        match cursor.node().kind() {
            "primitive_type" => {
                let primitive = parse_primitive_type(self.node_text(&cursor.node()))?;
                Some(VariableType::Primitive(primitive))
            }
            "user_defined_type" => {
                Some(VariableType::UserDefined(self.node_text(&cursor.node()).to_string()))
            }
            _ => {
                let mut resolved = None;
                iterate_children!(cursor, {
                    if resolved.is_none() {
                        resolved = self.extract_type_ref(cursor);
                    }
                });
                resolved
            }
        }
    }

    fn maybe_collect_docs(&self, node: Node<'_>, buffer: &mut String) {
        if node.kind() != "comment" {
            return;
        }

        let byte_range = node.byte_range();
        if byte_range.end - byte_range.start < 3 {
            return;
        }

        let prefix = &self.source[byte_range.start..byte_range.start + 3];
        if prefix == "///" || prefix == "/**" {
            buffer.push_str(self.node_text(&node));
            buffer.push_str("\n\n");
        }
    }

    #[inline]
    fn signature(&self, node: &Node<'_>) -> String {
        self.node_text(node)
            .split('{')
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    }

    #[inline]
    fn node_text(&self, node: &Node<'_>) -> &str {
        &self.source[node.byte_range()]
    }

    #[inline]
    fn push_diagnostic(&mut self, message: impl Into<String>, range: Range) {
        self.file.diagnostics.push(LoweringDiagnostic {
            message: message.into(),
            range,
        });
    }
}

fn parse_primitive_type(typ: &str) -> Option<PrimitiveType> {
    if typ == "bool" {
        return Some(PrimitiveType::Bool);
    }
    if typ == "address" {
        return Some(PrimitiveType::Address);
    }
    if typ == "string" {
        return Some(PrimitiveType::String);
    }
    if is_sized_type(typ, "uint") {
        return Some(PrimitiveType::Uint);
    }
    if is_sized_type(typ, "int") {
        return Some(PrimitiveType::Int);
    }
    if is_sized_type(typ, "bytes") {
        return Some(PrimitiveType::Bytes);
    }

    None
}

fn is_sized_type(typ: &str, prefix: &str) -> bool {
    let Some(rest) = typ.strip_prefix(prefix) else {
        return false;
    };

    if rest.is_empty() {
        return true;
    }

    let Ok(mut n) = rest.parse::<u16>() else {
        return false;
    };

    if prefix != "bytes" {
        n /= 8;
    }

    (1..=32).contains(&n)
}
