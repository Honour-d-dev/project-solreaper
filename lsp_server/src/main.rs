mod utilities;
mod lsp;
mod cursor;
mod salsa_query_plan;
mod lowering;
mod editor;
mod workspace;
mod loader;


use lsp::SolidityLspServer;

use anyhow::{Context};
use lsp_server::Connection;
use lsp_types::{
    HoverProviderCapability, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind
};





fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("lsp_server=info"))
        .add_directive("lsp_server::msg=off".parse().expect("valid tracing directive"))
        .add_directive("lsp_server::stdio=off".parse().expect("valid tracing directive"));
        // .add_directive("ignore=off".parse().expect("valid tracing directive"))
        // .add_directive("globset=off".parse().expect("valid tracing directive"))
        // .add_directive("salsa=off".parse().expect("valid tracing directive"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .without_time()
        .with_level(false)
        .with_target(false)
        .init();
    tracing::info!("solidity lsp server starting");

    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..ServerCapabilities::default()
    };

    let client_capabilities = connection
        .initialize(serde_json::to_value(server_capabilities)?)
        .context("failed to complete LSP initialize handshake")?;

    let (loader_tx, loader_rx) = loader::create_loader();

    SolidityLspServer::new( client_capabilities, connection.sender, loader_tx)?.run(connection.receiver, loader_rx)?;
    //realized we only need receiver for the event loop so no need to attach it to entire object. prevents borrow issues

    io_threads.join().context("failed to join LSP IO threads")?;
    Ok(())
}








