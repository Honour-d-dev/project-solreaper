use crate::capabilities::definition::definition;
use crate::capabilities::hover::hover;
use crate::loader;
use crate::salsa::{SalsaDb};
use crate::utilities::{log_info, to_utf8path};

use anyhow::Context;
use lsp_server::{Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, Initialized,
    Notification as LspNotification,
};
use lsp_types::request::{GotoDefinition, HoverRequest, Request as LspRequest};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, InitializeParams,
};

pub(crate) struct SolidityLspServer {
    sender: crossbeam_channel::Sender<Message>,
    db: SalsaDb,
}

impl SolidityLspServer {
    pub(crate) fn new(client_capabilities: serde_json::Value, sender: crossbeam_channel::Sender<Message>) -> anyhow::Result<Self> {
        //TODO: might need to check for encoding in client capabilities when adding support for other editors. vscode only does utf16
        //The diagnostic typo error still exists here, RA has a work around, incase we ever need it
        let InitializeParams { root_uri, .. }: lsp_types::InitializeParams = serde_json::from_value(client_capabilities)
            .context("failed to deserialize InitializeParams from client")?;
        log_info("LSP initialize handshake completed");

        let root_path = to_utf8path(&root_uri.context("root_uri is missing")?)?;
        //@NOTE No vfs for now, we only use utf8Paths, pathing may not be compatible with windows filesystem

        let (workspace, source_bundle) = loader::load_workspace(root_path);// Block on workspace loading
        log_info("Workspace fully loaded");

        let db = SalsaDb::new(workspace, source_bundle);
        log_info("DB Initialized");

        Ok(Self {
            sender,
            db,
        })
    }

    pub(crate) fn run(mut self, receiver: crossbeam_channel::Receiver<Message>) -> anyhow::Result<()> {
        for msg in receiver {
            match msg {
                Message::Request(r)      => self.handle_request(r)?,
                Message::Notification(n) => self.handle_notification(n)?,
                Message::Response(_)     => {}
            }
        }
        Ok(())
    }


    fn handle_request(
        &mut self,
        request: Request,
    ) -> anyhow::Result<()> {
        if request.method == HoverRequest::METHOD {
            match hover(&self.db, request) {
                Ok(response) => self.sender.send(Message::Response(response))?,
                Err(err) => log_info(format!("No hover Content: Error - {}", err)),
            };
            return Ok(());
        }

        if request.method == GotoDefinition::METHOD {
            match definition(&self.db, request) {
                Ok(response) => self.sender.send(Message::Response(response))?,
                Err(err) => log_info(format!("No definition location: Error - {}", err)),
            };
            return Ok(());
        }

        log_info(format!("Ignoring unsupported request: {}", request.method));
        let response = Response::new_ok(request.id, serde_json::Value::Null);
        self.sender.send(Message::Response(response))?;
        Ok(())
    }

    fn handle_notification(
        &mut self,
        notification: Notification,
    ) -> anyhow::Result<()> {
        
        if notification.method == DidOpenTextDocument::METHOD {
            let params: DidOpenTextDocumentParams = serde_json::from_value(notification.params)?;
            let path = to_utf8path(&params.text_document.uri)?;

            log_info(format!("Opened {}", path));//.path returns absolute path

            self.db.open(path, params.text_document.text);
          
            
            return Ok(());
        }
    
        if notification.method == DidChangeTextDocument::METHOD {
            let params: DidChangeTextDocumentParams = serde_json::from_value(notification.params)?;
            let path = to_utf8path(&params.text_document.uri)?;

            self.db.apply_changes(path, params.content_changes);
            return Ok(());
        }
    
        if notification.method == Initialized::METHOD {
            log_info("Client sent initialized");
        }
    
        Ok(())
    }
    
}