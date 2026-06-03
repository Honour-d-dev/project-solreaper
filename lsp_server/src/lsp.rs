use crate::editor::EditorHost;
use crate::loader::{self, LoadMsg};
use crate::salsa_query_plan::{AnalysisHost, SalsaFile};
use crate::utilities::{ format_symbol_hover, log_info, to_utf8path};
use crate::workspace::{discover_workspace};

use anyhow::{Context};
use lsp_server::{Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, Initialized,
    Notification as LspNotification,
};
use lsp_types::request::{HoverRequest, Request as LspRequest};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, Hover, HoverContents, HoverParams, InitializeParams, MarkupContent, MarkupKind,
};

pub(crate) struct SolidityLspServer {
    sender: crossbeam_channel::Sender<Message>,
    editor: EditorHost,
    db: AnalysisHost,
}

impl SolidityLspServer {
    pub(crate) fn new(client_capabilities: serde_json::Value , sender: crossbeam_channel::Sender<Message>, load_sender: crossbeam_channel::Sender<LoadMsg>) -> anyhow::Result<Self> {
        //TODO: might need to check for encoding in client capabilities when adding support for other editors. vscode only does utf16
        //The diagnostic typo error still exists here, RA has a work around, incase we ever need it
        let InitializeParams { root_uri, .. }: lsp_types::InitializeParams = serde_json::from_value(client_capabilities)
            .context("failed to deserialize InitializeParams from client")?;
        log_info("LSP initialize handshake completed");

        let root_path = to_utf8path(&root_uri.context("root_uri is missing")?)?;
        //@NOTE No vfs for now, we only use utf8Paths (& we get as_str for free) but pathing may not be compatible with windows filesystem

        let workspace = discover_workspace(&root_path);
        log_info(format!(
            "Discovered {} project(s) under {root_path}",
            workspace.projects.len()
        ));

        loader::load(&workspace, load_sender);

        let editor = EditorHost::new(workspace);
        let db = AnalysisHost::new();

        Ok(Self {
            sender,
            editor,
            db,
        })
    }

    pub(crate) fn run(mut self, receiver: crossbeam_channel::Receiver<Message>, mut load_rx: crossbeam_channel::Receiver<LoadMsg>) -> anyhow::Result<()> {
        loop {
        crossbeam_channel::select! {
            recv(receiver) -> msg => match msg {
                Ok(Message::Request(r))      => self.handle_request(r)?,
                Ok(Message::Notification(n)) => self.handle_notification(n)?,
                Ok(Message::Response(_))     => {}
                Err(_) => break, // client disconnected
            },
            recv(load_rx) -> loaded => match loaded {
                Ok(LoadMsg::File { lowered }) => {
                    if !self.editor.has_file(&lowered.path) {
                        log_info(format!("Loading file: {}", &lowered.path));
                        self.db.insert(lowered);
                    }
                }
                Ok(LoadMsg::Finished) | Err(_) => {
                    // Loader is done (or dropped). Swap in a channel that never
                    // fires so select! stops busy-looping on the closed receiver.
                    load_rx = crossbeam_channel::never();
                }
            },
        }
    }
    Ok(())
    }


    fn handle_request(
        &mut self,
        request: Request,
    ) -> anyhow::Result<()> {
        if request.method == HoverRequest::METHOD {
            let params: HoverParams = serde_json::from_value(request.params)?;
            let path = to_utf8path(&params.text_document_position_params.text_document.uri)?;
            log_info(format!("Hover request for {path}"));

            // If edits are staged, flush them before answering semantic queries.
            if let Some(file) = self.editor.apply_changes(&path) {
                self.db.insert(file);
            }

            let position = params.text_document_position_params.position;
            let node = self.editor.get_node_at_position(&path, position)?;
            let identifier = match self.editor.get_node_identifier(&path, &node) {
                Ok(id) => id,
                Err(err) => {
                    log_info(err.to_string());
                    return Ok(());
                }
            };
            let hover_content = if let Some(symbol) = self.db.resolve_symbol(&path, &identifier, node.range()) {
                format_symbol_hover( &symbol)
            } else {
                log_info(format!("No symbol found for identifier at position {:?} in {path} for {identifier}", position));
                return Ok(());
            };
    
            let result = Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: hover_content,
                }), 
                range: None,
            });
            let response = Response::new_ok(request.id, serde_json::to_value(result)?);
            self.sender.send(Message::Response(response))?;
            return Ok(());
        }
    
        log_info(format!("Ignoring unsupported request: {}", request.method));
        let response = Response::new_ok(request.id, serde_json::Value::Null);
        self.sender.send(Message::Response(response))?;
        Ok(())
    }

    fn resolve_recursive(&mut self,file: SalsaFile) {
        let missing_deps = self.db.collect_missing_deps(file);
        let resolved = self.editor.resolve_deps(&missing_deps);
        let resolved = self.db.insert_multiple(resolved);
        for file in resolved {
            self.resolve_recursive(file);
        }
    }
    
    fn handle_notification(
        &mut self,
        notification: Notification,
    ) -> anyhow::Result<()> {
        
        if notification.method == DidOpenTextDocument::METHOD {
            let params: DidOpenTextDocumentParams = serde_json::from_value(notification.params)?;
            let path = to_utf8path(&params.text_document.uri)?;

            log_info(format!("Opened {}", path));//.path returns absolute path

            let file = self.editor.insert_file(path, params.text_document.text)?;
            let salsa_file = self.db.insert(file);
            // collect missing dependency imports - dependencies are lazy-loaded
            // resolve in editor
            // update db
            // repeat, until all deps are resolved
            self.resolve_recursive(salsa_file);
          
            
            return Ok(());
        }
    
        if notification.method == DidChangeTextDocument::METHOD {
            let params: DidChangeTextDocumentParams = serde_json::from_value(notification.params)?;
            let path = to_utf8path(&params.text_document.uri)?;
            let mut should_apply = false;

            for change in params.content_changes.into_iter() {
                should_apply |= change.text.chars().any(char::is_whitespace);//is_whitespace also matches newline add for ';'
                self.editor.update(&path, change);
            }

            if should_apply {
                if let Some(file) = self.editor.apply_changes(&path) {
                    self.db.insert(file);
                }
                // self.log_info("applied changes")?;
            }

            // self.log_info(format!("Changed {path}"))?;
            return Ok(());
        }
    
        if notification.method == Initialized::METHOD {
            log_info("Client sent initialized");
        }
    
        Ok(())
    }
    
}