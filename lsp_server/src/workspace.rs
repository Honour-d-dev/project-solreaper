use std::fs;
use std::sync::mpsc;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::{WalkBuilder, WalkState};
use rustc_hash::FxHashMap;

use crate::utilities::normalize_path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PackageId(pub usize);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Remapping {
    pub prefix: String,
    pub target: Utf8PathBuf,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PackageConfig {
    pub remappings: Vec<Remapping>,
}

/// The package.json data needed to expand the package graph.
///
/// We intentionally keep this small. JavaScript build configuration and plugin
/// execution are outside the LSP's workspace-discovery responsibilities.
#[derive(Debug, Clone, Default)]
struct PackageManifest {
    workspace_patterns: Vec<String>,
    dependency_names: Vec<String>,
}

/// Configuration files found at one package root.
///
/// A package can have Foundry and Hardhat configuration at the same time. The
/// descriptor therefore records capabilities instead of assigning one package
/// kind to the root.
#[derive(Debug, Clone, Default)]
struct PackageDescriptor {
    root: Utf8PathBuf,
    foundry_config: Option<Utf8PathBuf>,
    hardhat_config: Option<Utf8PathBuf>,
    manifest: Option<PackageManifest>,
    remapping: Option<Utf8PathBuf>,
    is_dependency: bool,
}

#[derive(Debug, Clone)]
enum PackageReference {
    Workspace(Utf8PathBuf),
    Dependency(String),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Package {
    pub root: Utf8PathBuf,
    pub config: PackageConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredSourceRoot {
    pub root: Utf8PathBuf,
    pub package_id: PackageId,
    pub files: Vec<Utf8PathBuf>,
    pub is_dependency: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredWorkspace {
    pub workspace: Workspace,
    pub source_roots: Vec<DiscoveredSourceRoot>,
}

#[derive(Debug, Clone)]
pub(crate) struct Workspace {
    pub packages: Vec<Package>,
    pub package_id: FxHashMap<Utf8PathBuf, PackageId>,
}

impl Workspace {
    pub(crate) fn empty() -> Self {
        Self {
            packages: Vec::new(),
            package_id: FxHashMap::default(),
        }
    }
}

const PRUNED_DIRS: [&str; 5] = [".git", "node_modules", "out", "artifacts", "cache"];

/// Discover all packages before the Salsa database is built.
///
/// The traversal follows package manifests and known Foundry dependency roots,
/// but never crawls the workspace's `node_modules` directory blindly. Each
/// resolved package is converted into source roots before DefMaps can query it.
pub(crate) fn discover_workspace(root: &Utf8Path) -> DiscoveredWorkspace {
    let mut workspace = Workspace::empty();
    let mut source_roots = Vec::<DiscoveredSourceRoot>::new();

    for descriptor in discover_package_graph(root) {
        let is_dependency = descriptor.is_dependency;
        let (source_root_dirs, package_config) = package_layout(&descriptor, !is_dependency, is_dependency);
        if workspace.package_id.contains_key(&descriptor.root) {
            continue;
        }
        add_package(
            &mut workspace,
            &mut source_roots,
            descriptor.root,
            source_root_dirs,
            package_config,
            is_dependency,
        );
    }

    DiscoveredWorkspace { workspace, source_roots }
}

fn add_package(
    workspace: &mut Workspace,
    discovered_source_roots: &mut Vec<DiscoveredSourceRoot>,
    package_root: Utf8PathBuf,
    source_root_dirs: Vec<Utf8PathBuf>,
    config: PackageConfig,
    is_dependency: bool,
) {
    let package_root = normalize_path(&package_root);
    let source_root_dirs = source_root_dirs.into_iter().map(|path| normalize_path(&path)).collect::<Vec<_>>();
    let collected_roots = source_root_dirs.into_iter()
        .map(|root| (root.clone(), collect_sol_files(std::slice::from_ref(&root))))
        .filter(|(_, files)| !files.is_empty())
        .collect::<Vec<_>>();
    if collected_roots.is_empty() {
        return;
    }

    let package_id = PackageId(workspace.packages.len());
    workspace.package_id.insert(package_root.clone(), package_id);

    for (root, root_files) in collected_roots {
        discovered_source_roots.push(DiscoveredSourceRoot {
            root,
            package_id,
            files: root_files,
            is_dependency,
        });
    }

    workspace.packages.push(Package {
        root: package_root,
        config,
    });
}

fn detect_package(dir: &Utf8Path) -> Option<PackageDescriptor> {
    let foundry_config = dir.join("foundry.toml").is_file().then(|| dir.join("foundry.toml"));
    let hardhat_config = ["hardhat.config.ts", "hardhat.config.cts", "hardhat.config.mts", "hardhat.config.js", "hardhat.config.cjs", "hardhat.config.mjs"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file());
    let package_json = dir.join("package.json");
    let manifest = package_json.is_file().then(|| parse_package_manifest(&package_json)).flatten();
    let remapping_file = dir.join("remappings.txt").is_file()
        .then(|| dir.join("remappings.txt"));

    (foundry_config.is_some() || hardhat_config.is_some() || package_json.is_file() || remapping_file.is_some())
        .then(|| PackageDescriptor {
            root: normalize_path(dir),
            foundry_config,
            hardhat_config,
            manifest,
            remapping: remapping_file,
            is_dependency: false,
        })
}

fn parse_package_manifest(path: &Utf8Path) -> Option<PackageManifest> {
    let text = fs::read_to_string(path).ok()?;
    parse_package_manifest_text(&text)
}

fn parse_package_manifest_text(text: &str) -> Option<PackageManifest> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let mut workspace_patterns = match value.get("workspaces") {
        Some(serde_json::Value::Array(workspaces)) => workspaces.iter()
            .filter_map(|pattern| pattern.as_str().map(String::from))
            .collect(),
        Some(serde_json::Value::Object(workspaces)) => workspaces.get("packages")
            .and_then(serde_json::Value::as_array)
            .map(|packages| packages.iter().filter_map(|pattern| pattern.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut dependency_names = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]
        .into_iter()
        .filter_map(|key| value.get(key)?.as_object())
        .flat_map(|dependencies| dependencies.keys().cloned())
        .collect::<Vec<_>>();
    workspace_patterns.sort();
    workspace_patterns.dedup();
    dependency_names.sort();
    dependency_names.dedup();
    Some(PackageManifest { workspace_patterns, dependency_names })
}

fn find_package_roots(root: &Utf8Path) -> Vec<PackageDescriptor> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];

    while let Some(dir) = stack.pop() {
        if let Some(descriptor) = detect_package(&dir) {
            out.push(descriptor);
            continue;
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() && ft.is_dir() {
                if let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) 
                && let Some(name) = path.file_name() 
                && !PRUNED_DIRS.contains(&name) {
                    stack.push(path);
                }
            }
        }
    }

    out
}

/// Expand local packages, workspace packages, Foundry libraries, and npm
/// dependencies into one deduplicated package graph.
fn discover_package_graph(root: &Utf8Path) -> Vec<PackageDescriptor> {
    let mut queue = find_package_roots(root);
    let mut discovered = Vec::new();
    let mut seen = FxHashMap::default();

    while let Some(descriptor) = queue.pop() {
        let package_root = normalize_path(&descriptor.root);//no need
        if seen.insert(package_root.clone(), ()).is_some() {
            continue;
        }

        let descriptor = PackageDescriptor { root: package_root.clone(), ..descriptor };
        if let Some(config) = &descriptor.foundry_config {
            let (_, dependency_roots, _) = foundry_layout(&descriptor.root, config, false);
            for dependency_root in dependency_roots {
                queue.extend(find_package_roots(&dependency_root).into_iter().map(|mut descriptor| {
                    descriptor.is_dependency = true;
                    descriptor
                }));
            }
        }
        for reference in package_references(&descriptor) {
            let (package_root, is_dependency) = match reference {
                PackageReference::Workspace(root) => (root, false),
                PackageReference::Dependency(name) => {
                    let Some(root) = resolve_node_package(&descriptor.root, &name) else { continue; };
                    (root, true)
                }
            };
            if !seen.contains_key(&package_root) {
                let mut package = package_descriptor(&package_root);
                package.is_dependency = is_dependency;
                queue.push(package);
            }
        }
        discovered.push(descriptor);
    }

    discovered
}

fn package_descriptor(root: &Utf8Path) -> PackageDescriptor {
    detect_package(root).unwrap_or_else(|| PackageDescriptor {
        root: normalize_path(root),
        ..PackageDescriptor::default()
    })
}

/// Read one package.json and return both local workspace references and npm
/// dependency references. Keeping this in one function ensures the manifest is
/// parsed once and both kinds of package edges enter the same graph traversal.
fn package_references(descriptor: &PackageDescriptor) -> Vec<PackageReference> {
    let Some(manifest) = &descriptor.manifest else {
        return Vec::new();
    };
    let mut references = Vec::new();

    for pattern in &manifest.workspace_patterns {
        if let Some(parent) = pattern.strip_suffix("/*") {
            let directory = descriptor.root.join(parent);
            if let Ok(entries) = fs::read_dir(directory) {
                references.extend(entries.flatten()
                    .filter_map(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
                    .filter(|path| path.is_dir())
                    .map(|path| PackageReference::Workspace(normalize_path(&path))));
            }
        } else {
            let path = descriptor.root.join(pattern);
            if path.is_dir() {
                references.push(PackageReference::Workspace(normalize_path(&path)));
            }
        }
    }
    references.extend(manifest.dependency_names.iter().cloned().map(PackageReference::Dependency));
    references
}

fn resolve_node_package(owner: &Utf8Path, name: &str) -> Option<Utf8PathBuf> {
    let mut current = Some(owner);
    while let Some(directory) = current {
        let candidate = directory.join("node_modules").join(name);
        if candidate.join("package.json").is_file() {
            return Some(normalize_path(&candidate));
        }
        current = directory.parent();
    }
    None
}

/// Merge every configuration source present at a package root. A package can
/// combine Foundry, Hardhat, npm, and remapping configuration.
fn package_layout(
    descriptor: &PackageDescriptor,
    include_dev_source_roots: bool,
    is_dependency: bool,
) -> (Vec<Utf8PathBuf>, PackageConfig) {
    let mut source_root_dirs = Vec::new();
    let mut remappings = Vec::new();

    if let Some(config) = &descriptor.foundry_config {
        let (dirs, _, config) = foundry_layout(&descriptor.root, config, include_dev_source_roots);
        source_root_dirs.extend(dirs);
        remappings.extend(config.remappings);
    }
    if let Some(config) = &descriptor.hardhat_config {
        let (dirs, config) = hardhat_layout(&descriptor.root, config, include_dev_source_roots);
        source_root_dirs.extend(dirs);
        remappings.extend(config.remappings);
    }
    if let Some(remapping_file) = &descriptor.remapping {
        remappings.extend(parse_remappings_txt(remapping_file).unwrap_or_default());
    }
    if let Some(manifest) = &descriptor.manifest {
        for dependency in &manifest.dependency_names {
            if let Some(dependency_root) = resolve_node_package(&descriptor.root, dependency) {
                remappings.push(Remapping {
                    prefix: format!("{dependency}/"),
                    target: dependency_root,
                });
            }
        }
    }
    remappings.sort_by(|left, right| left.prefix.cmp(&right.prefix));
    remappings.dedup();

    if source_root_dirs.is_empty() {
        let default_root = if is_dependency {
            descriptor.root.clone()
        } else {
            descriptor.root.join("contracts")
        };
        if default_root.is_dir() {
            source_root_dirs.push(default_root);
        }
    }

    source_root_dirs.sort();
    source_root_dirs.dedup();
    (source_root_dirs, PackageConfig { remappings })
}

fn foundry_layout(
    root: &Utf8Path,
    config: &Utf8Path,
    include_dev_source_roots: bool,
) -> (
    Vec<Utf8PathBuf>,
    Vec<Utf8PathBuf>,
    PackageConfig,
) {
    let remappings_txt = root.join("remappings.txt");
    let mut remappings = Vec::new();

    if remappings_txt.is_file() {
        // remappings.txt overrides foundry.toml entirely
        remappings = parse_remappings_txt(&remappings_txt).unwrap_or_default();
    }

    let (src, toml_remappings) = fs::read_to_string(config)
        .ok()
        .and_then(|text| parse_foundry_toml(&text, remappings.is_empty()))
        .unwrap_or_else(|| ("src".to_string(), Vec::new()));

    let mut source_root_dirs = Vec::new();
    let src_dir = root.join(src.as_str());
    if src_dir.is_dir() {//FIXME: src directory could be contracts 
        source_root_dirs.push(src_dir);
    }

    let contracts_dir = root.join("contracts");
    if contracts_dir.is_dir() {
        source_root_dirs.push(contracts_dir);
    }

    if include_dev_source_roots {
        let test_dir = root.join("test");
        if test_dir.is_dir() {
            source_root_dirs.push(test_dir);
        }

        let script_dir = root.join("script");
        if script_dir.is_dir() {
            source_root_dirs.push(script_dir);
        }
    }

    let dependency_roots = ["lib"/* , "node_modules"*/]//we don't want to search node_modules, but i think there're other aliases to lib in foudry
        .iter()
        .map(|d| root.join(d))
        .filter(|d| d.is_dir())
        .collect();
    
    
    if remappings.is_empty() {
        remappings = toml_remappings;
    }
    (
        source_root_dirs,
        dependency_roots,
        PackageConfig { remappings },
    )
}

fn hardhat_layout(
    root: &Utf8Path,
    config: &Utf8Path,
    include_dev_source_roots: bool,
) -> (
    Vec<Utf8PathBuf>,
    PackageConfig,
) {
    let base = config.parent().unwrap_or(root);
    let source_dir = base.join(parse_hardhat_path(config, "sources")
        .unwrap_or_else(|| "contracts".to_string()));
    let mut source_root_dirs = source_dir.is_dir()
        .then_some(source_dir)
        .into_iter()
        .collect::<Vec<_>>();

    if include_dev_source_roots {
        let tests_dir = base.join(parse_hardhat_path(config, "tests")
            .unwrap_or_else(|| "test".to_string()));
        if tests_dir.is_dir() {
            source_root_dirs.push(tests_dir);
        }
    }

    (source_root_dirs, PackageConfig::default())
}

fn parse_hardhat_path(config: &Utf8Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(config).ok()?;
    parse_hardhat_path_text(&text, key)
}

fn parse_hardhat_path_text(text: &str, key: &str) -> Option<String> {
    let key_start = text.find(key)?;
    let remainder = &text[key_start + key.len()..];
    let separator = remainder.find([':', '='])?;
    let value = remainder[separator + 1..].trim_start();
    let quote = value.chars().next().filter(|quote| matches!(quote, '\'' | '"'))?;
    let value = &value[quote.len_utf8()..];
    let end = value.find(quote)?;
    let path = value[..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Extracts (src_dir, remappings) from foundry.toml.
/// Handles [profile.default], [default] (backwards-compat), and top-level keys.
fn parse_foundry_toml(text: &str, parse_remappings: bool) -> Option<(String, Vec<Remapping>)> {//@TODO: write tests for these
    let doc = toml::from_str::<toml::Value>(text).ok()?;

    // Try profile sections ([profile.default] or [default]) first
    if let Some((src, remaps)) = extract_from_profile(&doc, "default", parse_remappings) {
        return Some((src.unwrap_or_else(|| "src".to_string()), remaps));
    }

    // Fall back to top-level keys (no profile wrapper)
    let src = doc
        .get("src")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let remaps = if parse_remappings {
        doc.get("remappings")
            .and_then(|v| v.as_array())
            .map(parse_remap_array)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Some((src.unwrap_or_else(|| "src".to_string()), remaps))
}

/// Look inside `[profile.<name>]` or `[<<name>]` for src + remappings.
fn extract_from_profile(doc: &toml::Value, name: &str, parse_remappings: bool) -> Option<(Option<String>, Vec<Remapping>)> {
    let profile = doc
        .get("profile")
        .and_then(|v| v.as_table())
        .and_then(|table| table.get(name))
        .or_else(|| doc.as_table().and_then(|table| table.get(name)))?;

    let src = profile
        .get("src")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let remaps = if parse_remappings {
        profile
            .get("remappings")
            .and_then(|v| v.as_array())
            .map(parse_remap_array)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Some((src, remaps))
}

fn parse_remap_array(arr: &Vec<toml::Value>) -> Vec<Remapping> {
    let mut out = Vec::new();
    for v in arr {
        let s = v.as_str().unwrap_or("");
        if let Some((prefix, target)) = s.split_once('=') {
            out.push(Remapping {
                prefix: prefix.trim().to_string(),
                target: Utf8PathBuf::from(target.trim()),
            });
        }
    }
    out
}

fn parse_remappings_txt(path: &Utf8Path) -> Option<Vec<Remapping>> {
    let text = fs::read_to_string(path).ok()?;
    let mut remappings = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((prefix, target)) = line.split_once('=') {
            remappings.push(Remapping {
                prefix: prefix.trim().to_string(),
                target: Utf8PathBuf::from(target.trim()),
            });
        }
    }
    Some(remappings)
}

fn collect_sol_files(dirs: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
    let mut iter = dirs.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };

    let mut builder = WalkBuilder::new(first);
    for dir in iter {
        builder.add(dir);
    }
    builder.standard_filters(true);

    let (tx, rx) = mpsc::channel::<Utf8PathBuf>();
    builder.build_parallel().run(|| {
        let tx = tx.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.into_path();
            if path.extension().and_then(|e| e.to_str()) == Some("sol") {
                if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                    let _ = tx.send(path);
                }
            }
            WalkState::Continue
        })
    });
    drop(tx);

    rx.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_hardhat_path_text, parse_package_manifest_text};

    #[test]
    fn parses_static_hardhat_paths() {
        let config = r#"
            export default {
                paths: {
                    sources: "./src",
                    tests: './test'
                }
            };
        "#;

        assert_eq!(parse_hardhat_path_text(config, "sources"), Some("./src".into()));
        assert_eq!(parse_hardhat_path_text(config, "tests"), Some("./test".into()));
        assert_eq!(parse_hardhat_path_text(config, "cache"), None);
    }

    #[test]
    fn parses_workspace_and_dependency_references_from_one_manifest() {
        let manifest = parse_package_manifest_text(r#"
            {
                "workspaces": { "packages": ["packages/*"] },
                "dependencies": { "ethers": "^6.0.0", "solmate": "^6.0.0" },
                "devDependencies": { "hardhat": "^3.0.0" },
                "peerDependencies": { "viem": "^2.0.0" }
            }
        "#).unwrap();

        assert_eq!(manifest.workspace_patterns, vec!["packages/*"]);
        assert_eq!(manifest.dependency_names, vec!["ethers", "hardhat", "solmate", "viem"]);
    }

}
