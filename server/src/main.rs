//! Standalone LSP server that implements `textDocument/documentLink` for
//! Terraform/HCL files, linking resource types, data sources, and module
//! sources to their documentation on `registry.terraform.io`.

mod links;
mod providers;

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentLinkOptions, DocumentLinkParams, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("[terraform-link-docs] fatal: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_link_provider: Some(DocumentLinkOptions {
            resolve_provider: Some(false),
            work_done_progress_options: Default::default(),
        }),
        ..Default::default()
    };

    let server_capabilities = serde_json::to_value(capabilities)?;
    let _init_params = connection.initialize(server_capabilities)?;

    // `main_loop` takes ownership so the `Connection` (and its sender) is
    // dropped when it returns; otherwise `io_threads.join()` would block
    // forever on the writer thread.
    main_loop(connection)?;

    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    // In-memory store of open document text (Full sync → whole text each time).
    let mut documents: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                if let Err(err) = handle_request(&connection, &documents, req) {
                    eprintln!("[terraform-link-docs] request error: {err}");
                }
            }
            Message::Notification(not) => {
                if let Err(err) = handle_notification(&mut documents, not) {
                    eprintln!("[terraform-link-docs] notification error: {err}");
                }
            }
            Message::Response(_) => {}
        }
    }

    Ok(())
}

fn handle_request(
    connection: &Connection,
    documents: &HashMap<Url, String>,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        "textDocument/documentLink" => {
            let (id, params) = cast_request::<lsp_types::request::DocumentLinkRequest>(req)?;
            let response = document_link(documents, params);
            connection
                .sender
                .send(Message::Response(Response::new_ok(id, response)))?;
        }
        _ => {
            // Everything else: empty/no-op result.
            connection
                .sender
                .send(Message::Response(Response::new_ok(req.id, serde_json::Value::Null)))?;
        }
    }
    Ok(())
}

fn document_link(
    documents: &HashMap<Url, String>,
    params: DocumentLinkParams,
) -> Vec<lsp_types::DocumentLink> {
    let uri = params.text_document.uri;
    let text = match documents.get(&uri) {
        Some(t) => t.clone(),
        None => return Vec::new(),
    };
    let doc_path = uri.to_file_path().ok();
    let provider_map = providers::get_provider_map(doc_path.as_deref(), &text);
    links::compute_links(&text, &provider_map, doc_path.as_deref())
}

fn handle_notification(
    documents: &mut HashMap<Url, String>,
    not: Notification,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match not.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            documents.insert(params.text_document.uri, params.text_document.text);
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            // Full sync: the last content change carries the whole document.
            if let Some(change) = params.content_changes.into_iter().last() {
                documents.insert(params.text_document.uri, change.text);
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            documents.remove(&params.text_document.uri);
        }
        _ => {}
    }
    Ok(())
}

fn cast_request<R>(req: Request) -> Result<(RequestId, R::Params), ExtractError<Request>>
where
    R: lsp_types::request::Request,
    R::Params: serde::de::DeserializeOwned,
{
    req.extract(R::METHOD)
}
