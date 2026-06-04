use std::fmt;
use std::mem;

use camino::Utf8PathBuf;
use tree_sitter::{Node, Range, Tree, TreeCursor};

use crate::cursor::{Scope, ScopeBuilder, ScopeId, ScopeNavigator, ScopeType};
use crate::utilities::log_info;
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
    pub interfaces: Vec<Interface>,
    pub libraries: Vec<Library>,
    pub free_functions: Vec<Function>,
    pub events: Vec<Event>,
    pub errors: Vec<Error>,
    pub structs: Vec<Struct>,
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
    Interface(Interface),
    Library(Library),
    Function(Function),
    IFunction(IFunction),
    Variable(Variable),
    Event(Event),
    Error(Error),
    Struct(Struct),
    Modifier(Modifier),
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
    pub events: Vec<Event>,
    pub errors: Vec<Error>,
    pub structs: Vec<Struct>,
    pub modifiers: Vec<Modifier>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub bases: Vec<String>,
    pub functions: Vec<IFunction>,
    pub events: Vec<Event>,
    pub errors: Vec<Error>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Library {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub functions: Vec<Function>,
    pub events: Vec<Event>,
    pub errors: Vec<Error>,
    pub structs: Vec<Struct>,
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
pub(crate) struct Event {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub parameters: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub parameters: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Modifier {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub parameters: Vec<Variable>,
    pub local_vars: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Struct {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub fields: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IFunction {
    pub name: String,
    pub docs: String,
    pub signature: String,
    pub range: Range,
    pub scope: ScopeId,
    pub parameters: Vec<Variable>,
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
    StructField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VariableType {
    Primitive(PrimitiveType),
    UserDefined(String),
    Array { typ: Box<VariableType>, size: Option<usize> },
    Mapping { key: Box<VariableType>, value: Box<VariableType> },
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

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrimitiveType::Int => write!(f, "int"),
            PrimitiveType::Uint => write!(f, "uint"),
            PrimitiveType::Bool => write!(f, "bool"),
            PrimitiveType::Address => write!(f, "address"),
            PrimitiveType::String => write!(f, "string"),
            PrimitiveType::Bytes => write!(f, "bytes"),
        }
    }
}

impl fmt::Display for VariableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableType::Primitive(p) => write!(f, "{p}"),
            VariableType::UserDefined(name) => write!(f, "{name}"),
            VariableType::Array { typ, size } => {
                if let Some(s) = size {
                    write!(f, "{typ}[{s}]")
                } else {
                    write!(f, "{typ}[]")
                }
            }
            VariableType::Mapping { key, value } => {
                write!(f, "mapping({key} => {value})")
            }
            VariableType::Unknown => write!(f, "Unknown"),
        }
    }
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
                interfaces: Vec::new(),
                libraries: Vec::new(),
                free_functions: Vec::new(),
                events: Vec::new(),
                errors: Vec::new(),
                structs: Vec::new(),
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
                "function_definition" => {//free functions
                    let function =
                        self.lower_function(cursor, mem::take(&mut pending_doc));
                    self.file.free_functions.push(function);
                }
                "interface_declaration" => {
                    let interface =
                        self.lower_interface(cursor, mem::take(&mut pending_doc));
                    self.file.interfaces.push(interface);
                }
                "library_declaration" => {
                    let library =
                        self.lower_library(cursor, mem::take(&mut pending_doc));
                    self.file.libraries.push(library);
                }
                "event_definition" => {
                    let event = self.lower_event(cursor, mem::take(&mut pending_doc));
                    self.file.events.push(event);
                }
                "struct_declaration" => {
                    let strukt = self.lower_struct(cursor, mem::take(&mut pending_doc));
                    self.file.structs.push(strukt);
                }
                "error_declaration" => {
                    let error = self.lower_error(cursor, mem::take(&mut pending_doc));
                    self.file.errors.push(error);
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
                _ => ImportType::Full,//never reached, match is logically exhaustive - just to satisfy exhaustive requrement else compiler complains
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
        let mut events = Vec::new();
        let mut errors = Vec::new();
        let mut structs = Vec::new();
        let mut modifiers = Vec::new();
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
                            "event_definition" => {
                                let event = self.lower_event(cursor, mem::take(&mut body_docs));
                                events.push(event);
                            }
                            "struct_declaration" => {
                                let strukt = self.lower_struct(cursor, mem::take(&mut body_docs));
                                structs.push(strukt);
                            }
                            "error_declaration" => {
                                let error = self.lower_error(cursor, mem::take(&mut body_docs));
                                errors.push(error);
                            }
                            "modifier_definition" => {
                                let modifier = self.lower_modifier(cursor, mem::take(&mut body_docs));
                                modifiers.push(modifier);
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
            events,
            errors,
            structs,
            modifiers,
        }
    }

    fn lower_interface(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Interface {
        self.scope_builder
            .to_next(cursor.node().range(), ScopeType::Contract);
        let contract_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut bases = Vec::new();
        let mut functions = Vec::new();
        let mut events = Vec::new();
        let mut errors = Vec::new();
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
                            "function_definition" => {
                                let function = self.lower_ifunction(
                                    cursor,
                                    mem::take(&mut body_docs),
                                );
                                functions.push(function);
                            }
                            "event_definition" => {
                                let event = self.lower_event(cursor, mem::take(&mut body_docs));
                                events.push(event);
                            }
                            "error_declaration" => {
                                let error = self.lower_error(cursor, mem::take(&mut body_docs));
                                errors.push(error);
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
        Interface {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: contract_scope,
            bases,
            functions,
            events,
            errors,
        }
    }

    fn lower_library(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Library {
        self.scope_builder
            .to_next(cursor.node().range(), ScopeType::Contract);
        let library_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut functions = Vec::new();
        let mut events = Vec::new();
        let mut errors = Vec::new();
        let mut structs = Vec::new();
        let mut body_docs = String::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "contract_body" => {
                    iterate_children!(cursor, {
                        match cursor.node().kind() {
                            "comment" => {
                                self.maybe_collect_docs(cursor.node(), &mut body_docs);
                            }
                            "function_definition" => {
                                let function = self.lower_function(
                                    cursor,
                                    mem::take(&mut body_docs),
                                );
                                functions.push(function);
                            }
                            "event_definition" => {
                                let event = self.lower_event(cursor, mem::take(&mut body_docs));
                                events.push(event);
                            }
                            "struct_declaration" => {
                                let strukt = self.lower_struct(cursor, mem::take(&mut body_docs));
                                structs.push(strukt);
                            }
                            "error_declaration" => {
                                let error = self.lower_error(cursor, mem::take(&mut body_docs));
                                errors.push(error);
                            }
                            _ => {
                                body_docs.clear();
                            }
                        }
                    });
                }
                _ => {}
            }
        });

        if name.is_empty() {
            self.push_diagnostic("library declaration without identifier", cursor.node().range());
        }

        self.scope_builder.to_parent();
        Library {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: library_scope,
            functions,
            events,
            errors,
            structs,
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
                //@TODO use visibility to shortcircuit on summarized
                // this fn would have to return an option though
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

    fn lower_ifunction(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> IFunction {
        let function_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut parameters = Vec::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                //@TODO use visibility to shortcircuit on summarized
                // this fn would have to return an option though
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
                _ => {}
            }
        });


        IFunction {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: function_scope,//@NOTE do i add a decl_scope and body_scope? this is body scope though
            parameters
        }
    }


    fn lower_event(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Event {
        let event_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut parameters = Vec::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "parameter" if !self.summarize => {
                    let parameter = self.lower_variable(
                        cursor,
                        VariableKind::Parameter,
                        event_scope,
                        String::new(),
                    );
                    parameters.push(parameter);
                }
                _ => {}
            }
        });

        Event {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: event_scope,
            parameters,
        }
    }


    fn lower_error(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Error {
        let error_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut parameters = Vec::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "parameter" if !self.summarize => {
                    let parameter = self.lower_variable(
                        cursor,
                        VariableKind::Parameter,
                        error_scope,
                        String::new(),
                    );
                    parameters.push(parameter);
                }
                _ => {}
            }
        });

        Error {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: error_scope,
            parameters,
        }
    }


    fn lower_modifier(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Modifier {
        let modifier_scope = self.scope_builder.current();

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
                        modifier_scope,
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

        Modifier {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: modifier_scope,
            parameters,
            local_vars,
        }
    }

    fn lower_struct(
        &mut self,
        cursor: &mut TreeCursor<'_>,
        docs: String,
    ) -> Struct {
        let struct_scope = self.scope_builder.current();

        let mut name = String::new();
        let mut fields = Vec::new();

        iterate_children!(cursor, {
            match cursor.node().kind() {
                "identifier" => {
                    name = self.node_text(&cursor.node()).to_string();
                }
                "struct_body" => {
                    iterate_children!(cursor, {
                        if cursor.node().kind() == "struct_member" {
                            let field = self.lower_variable(
                                cursor,
                                VariableKind::StructField,
                                struct_scope,
                                String::new(),
                            );
                            fields.push(field);
                        }
                    });
                }
                _ => {}
            }
        });

        Struct {
            name,
            docs,
            signature: self.signature(&cursor.node()),
            range: cursor.node().range(),
            scope: struct_scope,
            fields,
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
                //@TODO use visibility to shortcircuit on summarized
                "identifier" if name.is_none() => {
                    name = Some(self.node_text(&cursor.node()).to_string());
                }
                "type_name" => {
                    //@TODO array support
                    let typ_str = self.node_text(&cursor.node()).to_string();
                    let (is_array, size) = if typ_str.ends_with("]") {
                        let size = typ_str.find("[").map(|i| {
                            let s = &typ_str[i+1..typ_str.len()-1];
                            usize::from_str_radix(s, 10).ok()
                        }).flatten();
                        (true, size)
                    } else {
                        (false, None)
                    };
                    typ = if is_array {
                        VariableType::Array{typ: Box::new(self.extract_type_name(cursor)), size}
                    } else {
                        self.extract_type_name(cursor)
                    }
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

    fn extract_type_name(&self, cursor: &mut TreeCursor<'_>) -> VariableType {
        let mut typp = VariableType::Unknown;
        iterate_children!(cursor, {
            match cursor.node().kind() {
                "primitive_type" => {
                    match parse_primitive_type(self.node_text(&cursor.node())) {
                        Some(primitive) => {
                            typp = VariableType::Primitive(primitive);
                        }
                        None => {
                            log_info(&format!("Unkown type {}", self.node_text(&cursor.node())));
                            typp = VariableType::Unknown;
                        }
                    }
                }
                "user_defined_type" => {
                    typp = VariableType::UserDefined(self.node_text(&cursor.node()).to_string());
                }
                "type_name" => {//type_name again, so we recurse
                    let typ_str = self.node_text(&cursor.node()).to_string();
                    let (is_array, size) = if typ_str.ends_with("]") {
                        let size = typ_str.find("[").map(|i| {
                            let s = &typ_str[i+1..typ_str.len()-1];
                            usize::from_str_radix(s, 10).ok()
                        }).flatten();
                        (true, size)
                    } else {
                        (false, None)
                    };                    
                    typp = if is_array {
                        VariableType::Array{ typ: Box::new(self.extract_type_name(cursor)), size}
                    } else {
                        self.extract_type_name(cursor)
                    };
                }
                _ => {},
            }
        });
        typp
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
    if typ == "address" || typ == "address payable" {
        //@TODO combine for now , payable adds extra metadata/fns to the type so there should be a distinction later
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
