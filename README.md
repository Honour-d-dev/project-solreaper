## Project  Sol-Reaper
An Ambitious attempt at a Rust-Analyzer like language server for Solidity. Very early development stage.
Best of both worlds: Incremental reparsing + Incremental computation enabled by tree-sitter + salsa.


To run:
```
cd lsp_server
cargo build
cd ../extension
npm install
npm run compile
```

Then open the project(extension/src/extension.ts) in VS Code and press `F5` to start the extension in debug mode.

#### Implemented Capabilities:
Hover 👍

```

┌─────────────────────────────────────────────────────────────────────────────┐
│                              VS CODE EXTENSION                              │
│                         (extension/src/extension.ts)                        │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │  - Spawns lsp_server binary via stdio                                   ││
│  │  - JSON-RPC over stdin/stdout                                           ││
│  └─────────────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────┬───────────────────────────────────────┘
                                      │ stdio JSON-RPC
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           LSP SERVER (Rust)                                 │
│                         (lsp_server/src/main.rs)                            │
│  ┌─────────────────────────────────────────────────────────────────────────┐│
│  │  - tracing_subscriber setup (logs to stderr)                            ││
│  │  - lsp_server::Connection::stdio()                                      ││
│  │  - ServerCapabilities: incremental sync + hover                         ││
│  │  - Creates (loader_tx, loader_rx) loading channel                       ││
│  │  - SolidityLspServer::new(...) → .run()                                 ││
│  └─────────────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────┬───────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    SOLIDITYLSPSERVER  (lsp.rs)                              │
│                                                                             │
│   ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────────────┐  │
│   │   sender     │  │    editor    │  │        db (AnalysisHost)         │  │
│   │ (crossbeam)  │  │ (EditorHost) │  │          (salsa_db)              │  │
│   └──────┬───────┘  └──────┬───────┘  └──────────────┬───────────────────┘  │
│          │                 │                         │                      │
│          │                 │                         │                      │
│          ▼                 ▼                         ▼                      │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │                         Event Loop (.run())                         │   │
│   │  crossbeam_channel::select! {                                       │   │
│   │    recv(receiver) → LSP Requests / Notifications                    │   │
│   │    recv(load_rx)  → Background Loader Results                       │   │
│   │  }                                                                  │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘

                        ╔═══════════════════════════════════╗
                        ║           SUBSYSTEMS              ║
                        ╚═══════════════════════════════════╝

┌────────────────────────────────────────┐    ┌────────────────────────────────────────┐
│              EDITORHOST                │    │             ANALYSISHOST               │
│              (editor.rs)               │    │              (salsa_db)                │
│                                        │    │                                        │
│  ┌──────────────────────────────────┐  │    │  ┌──────────────────────────────────┐  │
│  │  FxHashMap<path, FileData>       │  │    │  │  FxHashMap<path, SalsaFile>      │  │
│  │                                  │  │    │  │                                  │  │
│  │  FileData {                      │  │    │  │  SalsaFile {                     │  │
│  │    rope: Rope,                   │  │    │  │    lowered: Arc<LoweredFile>     │  │
│  │    tree: Tree,                   │  │    │  │    path: Utf8PathBuf             │  │
│  │    has_changes: bool,            │  │    │  │  }                               │  │
│  │    project_root,                 │  │    │  │                                  │  │
│  │  }                               │  │    │  └──────────────────────────────────┘  │
│  └──────────────────────────────────┘  │    │                                        │
│                                        │    │     Responsibilities:                  │
│  Responsibilities:                     │    │       - symbol caching                 │
│    - Incremental text edits            │    │       - Incremental recomputation      │
│    - Incremental tree-sitter reparsing │    │         on invalidation                │
│    - Node lookup by position           │    │                                        │
│    - Identifier extraction             │    │                                        │
│    - Dependency resolution             │    │                                        │
│    - File lowering on demand           │    │                                        │
│      (summarized_lower)                │    │                                        │
│                                        │    │                                        │
│                                        │    │                                        │
└────────────────────────────────────────┘    └────────────────────────────────────────┘
                          

                        ╔═══════════════════════════════════╗
                        ║        WORKSPACE DISCOVERY        ║
                        ╚═══════════════════════════════════╝
        
                                      (workspace.rs)
                                            │
                            ┌───────────────┼───────────────┐
                            ▼               ▼               ▼
                      ┌─────────┐    ┌─────────┐    ┌─────────┐
                      │ Foundry │    │ Hardhat │    │  ...    │
                      │  (now)  │    │ (later) │    │         │
                      └────┬────┘    └────┬────┘    └────┬────┘
                           │              │              │
                      ┌────▼────┐    ┌────▼────┐    ┌────▼────┐
                      │foundry. │    │hardhat. │    │         │
                      │toml     │    │config   │    │         │
                      └────┬────┘    └─────────┘    └─────────┘
                           │
                      ┌────▼──────────────────────────────────┐
                      │  - source_dirs (src/, lib/, etc.)     │
                      │  - remappings (from foundry.toml)     │
                      │  - dependency_roots (lib/)            │
                      │  - collect all .sol files             │
                      └───────────────────────────────────────┘

```
