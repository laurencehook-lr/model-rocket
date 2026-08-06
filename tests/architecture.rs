use std::{collections::HashSet, fs, path::Path, path::PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::{
    Expr, Fields, File, FnArg, GenericParam, ImplItem, Item, ItemTrait, ItemUse, Lit, Macro, Pat,
    TraitItem, Type, UseTree, Visibility, visit, visit::Visit,
};

const PORT_FILE: &str = "src/ports.rs";
const PORT_DIRECTORY: &str = "src/ports";
const ADAPTER_DIRECTORY: &str = "src/adapters";
const RUST_EXTENSION: &str = "rs";
const REQUIRED_ARCHITECTURE_PATHS: [&str; 12] = [
    "src/domain/mod.rs",
    "src/ports.rs",
    "src/application/model_router.rs",
    "src/contracts/anthropic/mod.rs",
    "src/contracts/codex/mod.rs",
    "src/policies/mod.rs",
    "src/adapters/inbound/http.rs",
    "src/adapters/outbound/anthropic.rs",
    "src/adapters/outbound/codex/mod.rs",
    "src/adapters/outbound/codex/runtime.rs",
    "src/bootstrap.rs",
    "tests/architecture.rs",
];
const FORBIDDEN_LEGACY_PATHS: [&str; 7] = [
    "src/anthropic.rs",
    "src/anthropic_messages.rs",
    "src/app_server.rs",
    "src/bridge.rs",
    "src/http.rs",
    "src/infrastructure",
    "src/models.rs",
];
const FORBIDDEN_DOMAIN_TYPES: [&str; 16] = [
    "Body",
    "Bytes",
    "HeaderMap",
    "HeaderName",
    "HeaderValue",
    "Path",
    "PathBuf",
    "Receiver",
    "Request",
    "Response",
    "Sender",
    "SocketAddr",
    "StatusCode",
    "Uri",
    "Url",
    "Value",
];
const PORT_FUTURE_NAME: &str = "PortFuture";
const ALLOWED_PORT_TRAITS: [&str; 7] = [
    "AnthropicGateway",
    "AnthropicResponseSink",
    "ModelOutput",
    "ModelResponseSink",
    "ModelRouter",
    "ModelSession",
    "ModelSessionFactory",
];
const ALLOWED_PORT_IMPORTS: [&str; 18] = [
    "std::future::Future",
    "std::pin::Pin",
    "crate::domain::AnthropicRequest",
    "crate::domain::AnthropicResponseChunk",
    "crate::domain::AnthropicResponseHead",
    "crate::domain::AssistantOutcome",
    "crate::domain::AuthorizedRequest",
    "crate::domain::ModelRequest",
    "crate::domain::ModelResponseChunk",
    "crate::domain::ModelResponseEnd",
    "crate::domain::ModelResponseHead",
    "crate::domain::PresentedCredential",
    "crate::domain::PreflightReport",
    "crate::domain::StartModelTurn",
    "crate::domain::AssistantTextDelta",
    "crate::domain::ContinueModelTurn",
    "crate::domain::BridgeError",
    "crate::domain::CodexModelId",
];
const ALLOWED_PORT_TYPE_IDENTIFIERS: [&str; 31] = [
    "AnthropicGateway",
    "AnthropicRequest",
    "AnthropicResponseChunk",
    "AnthropicResponseHead",
    "AnthropicResponseSink",
    "AssistantOutcome",
    "AuthorizedRequest",
    "Box",
    "BridgeError",
    "CodexModelId",
    "Future",
    "ModelOutput",
    "ModelRequest",
    "ModelResponseChunk",
    "ModelResponseEnd",
    "ModelResponseHead",
    "ModelResponseSink",
    "ModelRouter",
    "ModelSession",
    "Pin",
    "PortFuture",
    "PreflightReport",
    "PresentedCredential",
    "Result",
    "Send",
    "Self",
    "StartModelTurn",
    "Sync",
    "T",
    "AssistantTextDelta",
    "ContinueModelTurn",
];
const IMPORT_PROVENANCE_TYPE_IDENTIFIERS: [&str; 16] = [
    "AnthropicRequest",
    "AnthropicResponseChunk",
    "AnthropicResponseHead",
    "AssistantOutcome",
    "AuthorizedRequest",
    "BridgeError",
    "CodexModelId",
    "ModelRequest",
    "ModelResponseChunk",
    "ModelResponseEnd",
    "ModelResponseHead",
    "PreflightReport",
    "PresentedCredential",
    "StartModelTurn",
    "AssistantTextDelta",
    "ContinueModelTurn",
];

#[derive(Default)]
struct LiteralVisitor {
    literals: Vec<String>,
    macros: Vec<String>,
}

#[derive(Default)]
struct LoggingVisitor {
    calls: Vec<(String, String)>,
}

impl<'ast> Visit<'ast> for LoggingVisitor {
    fn visit_macro(&mut self, item: &'ast Macro) {
        let name = path_name(&item.path);
        if ["trace", "debug", "info", "warn", "error"]
            .iter()
            .any(|level| name == *level || name.ends_with(&format!("::{level}")))
        {
            self.calls.push((name, item.tokens.to_string()));
        }
        visit::visit_macro(self, item);
    }
}

impl<'ast> Visit<'ast> for LiteralVisitor {
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if attribute.path().is_ident("doc") {
            return;
        }
        match &attribute.meta {
            syn::Meta::Path(_) => {}
            syn::Meta::List(list) => {
                collect_token_literals(list.tokens.clone(), &mut self.literals);
            }
            syn::Meta::NameValue(value) => visit::visit_expr(self, &value.value),
        }
    }

    fn visit_lit(&mut self, literal: &'ast Lit) {
        self.literals.push(literal_text(literal));
    }

    fn visit_macro(&mut self, item: &'ast Macro) {
        self.macros.push(path_name(&item.path));
    }
}

fn collect_token_literals(tokens: TokenStream, literals: &mut Vec<String>) {
    for token in tokens {
        match token {
            TokenTree::Group(group) => collect_token_literals(group.stream(), literals),
            TokenTree::Literal(literal) => literals.push(literal.to_string()),
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
}

#[derive(Default)]
struct PortTypeVisitor {
    forbidden: Vec<String>,
    used: Vec<String>,
}

impl<'ast> Visit<'ast> for PortTypeVisitor {
    fn visit_type_path(&mut self, type_path: &'ast syn::TypePath) {
        let identifier = type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        if type_path.qself.is_some()
            || type_path.path.segments.len() != 1
            || identifier
                .as_deref()
                .is_none_or(|name| !ALLOWED_PORT_TYPE_IDENTIFIERS.contains(&name))
        {
            self.forbidden.push(path_name(&type_path.path));
        }
        if let Some(identifier) = identifier {
            self.used.push(identifier);
        }
        visit::visit_type_path(self, type_path);
    }
}

fn literal_text(literal: &Lit) -> String {
    match literal {
        Lit::Str(value) => value.value(),
        Lit::ByteStr(value) => format!("{:?}", value.value()),
        Lit::Byte(value) => value.value().to_string(),
        Lit::Char(value) => value.value().to_string(),
        Lit::Int(value) => value.base10_digits().to_owned(),
        Lit::Float(value) => value.base10_digits().to_owned(),
        Lit::Bool(value) => value.value.to_string(),
        Lit::Verbatim(value) => value.to_string(),
        _ => String::from("unknown literal"),
    }
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn parse(path: &Path) -> Result<File, Box<dyn std::error::Error>> {
    Ok(syn::parse_file(&fs::read_to_string(path)?)?)
}

fn rust_files(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path)? {
            let entry_path = entry?.path();
            if entry_path.is_dir() {
                pending.push(entry_path);
            } else if entry_path.extension().and_then(|value| value.to_str())
                == Some(RUST_EXTENSION)
            {
                files.push(entry_path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn port_files() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    let flat_port = Path::new(PORT_FILE);
    if flat_port.is_file() {
        files.push(flat_port.to_path_buf());
    }
    let port_directory = Path::new(PORT_DIRECTORY);
    if port_directory.is_dir() {
        files.extend(rust_files(port_directory)?);
    }
    Ok(files)
}

fn validate_literal_free(syntax: &File) -> Result<(), String> {
    let mut visitor = LiteralVisitor::default();
    visitor.visit_file(syntax);
    if !visitor.literals.is_empty() {
        return Err(format!("inline literals: {:?}", visitor.literals));
    }
    if !visitor.macros.is_empty() {
        return Err(format!(
            "macros whose tokens could hide inline literals: {:?}",
            visitor.macros
        ));
    }
    Ok(())
}

fn collect_imports(
    prefix: &mut Vec<String>,
    tree: &UseTree,
    imports: &mut Vec<String>,
) -> Result<(), String> {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_imports(prefix, &path.tree, imports)?;
            prefix.pop();
        }
        UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            imports.push(prefix.join("::"));
            prefix.pop();
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_imports(prefix, item, imports)?;
            }
        }
        UseTree::Rename(_) => return Err(String::from("renamed imports are forbidden in ports")),
        UseTree::Glob(_) => return Err(String::from("glob imports are forbidden in ports")),
    }
    Ok(())
}

fn validate_import(item: &ItemUse) -> Result<(), String> {
    if item.leading_colon.is_some() {
        return Err(String::from("absolute-root imports are forbidden in ports"));
    }
    let mut imports = Vec::new();
    collect_imports(&mut Vec::new(), &item.tree, &mut imports)?;
    let forbidden = imports
        .into_iter()
        .filter(|path| !ALLOWED_PORT_IMPORTS.contains(&path.as_str()))
        .collect::<Vec<_>>();
    if forbidden.is_empty() {
        Ok(())
    } else {
        Err(format!("unapproved port imports: {forbidden:?}"))
    }
}

fn imports_in(items: &[Item]) -> Result<Vec<String>, String> {
    let mut imports = Vec::new();
    for item in items {
        if let Item::Use(item_use) = item {
            collect_imports(&mut Vec::new(), &item_use.tree, &mut imports)?;
        }
        if let Item::Mod(module) = item
            && let Some((_brace, nested)) = &module.content
        {
            imports.extend(imports_in(nested)?);
        }
    }
    Ok(imports)
}

fn validate_port_trait(item_trait: &ItemTrait) -> Result<(), String> {
    let name = item_trait.ident.to_string();
    if !ALLOWED_PORT_TRAITS.contains(&name.as_str()) {
        return Err(format!("unapproved port trait {name}"));
    }
    if !item_trait.generics.params.is_empty() || item_trait.generics.where_clause.is_some() {
        return Err(format!("port trait {name} may not be generic"));
    }
    let expected_methods: &[&str] = match name.as_str() {
        "AnthropicGateway" => &["exchange"],
        "AnthropicResponseSink" | "ModelResponseSink" => &["start", "emit"],
        "ModelOutput" => &["emit"],
        "ModelRouter" => &["authorize", "dispatch"],
        "ModelSession" => &["start_turn", "continue_tool"],
        "ModelSessionFactory" => &["launch"],
        _ => return Err(format!("unapproved port trait {name}")),
    };
    let actual_methods = item_trait
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if actual_methods != expected_methods {
        return Err(format!(
            "port trait {name} methods are {actual_methods:?}; expected {expected_methods:?}"
        ));
    }
    for item in &item_trait.items {
        let TraitItem::Fn(method) = item else {
            return Err(format!(
                "port trait {name} may contain method declarations only"
            ));
        };
        if method.default.is_some() {
            return Err(format!(
                "port trait {name} method {} may not have a default body",
                method.sig.ident
            ));
        }
        if method.sig.generics.where_clause.is_some()
            || method
                .sig
                .generics
                .params
                .iter()
                .any(|parameter| !matches!(parameter, GenericParam::Lifetime(_)))
        {
            return Err(format!(
                "port trait {name} method {} may have lifetime generics only",
                method.sig.ident
            ));
        }
    }
    Ok(())
}

fn validate_port_items(items: &[Item]) -> Result<(), String> {
    for item in items {
        match item {
            Item::Use(item_use) => validate_import(item_use)?,
            Item::Trait(item_trait) => validate_port_trait(item_trait)?,
            Item::Type(alias) if alias.ident == PORT_FUTURE_NAME => {}
            Item::Mod(module) => {
                if let Some((_brace, nested)) = &module.content {
                    validate_port_items(nested)?;
                }
            }
            other => {
                return Err(format!(
                    "ports may contain only imports, modules, traits, and PortFuture: {other:?}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_port_types(syntax: &File) -> Result<(), String> {
    let mut visitor = PortTypeVisitor::default();
    visitor.visit_file(syntax);
    visitor.forbidden.sort();
    visitor.forbidden.dedup();
    if !visitor.forbidden.is_empty() {
        return Err(format!(
            "unapproved port type identifiers: {:?}",
            visitor.forbidden
        ));
    }
    let imports = imports_in(&syntax.items)?;
    for identifier in visitor
        .used
        .iter()
        .filter(|identifier| IMPORT_PROVENANCE_TYPE_IDENTIFIERS.contains(&identifier.as_str()))
    {
        let expected_suffix = format!("::{identifier}");
        if !imports.iter().any(|path| {
            ALLOWED_PORT_IMPORTS.contains(&path.as_str()) && path.ends_with(&expected_suffix)
        }) {
            return Err(format!(
                "port type {identifier} does not have an approved explicit import"
            ));
        }
    }
    Ok(())
}

fn architecture_error(path: &Path, error: &str) -> std::io::Error {
    std::io::Error::other(format!("{}: {error}", path.display()))
}

#[derive(Default)]
struct CratePathVisitor {
    targets: HashSet<String>,
}

impl<'ast> Visit<'ast> for CratePathVisitor {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.first().is_some_and(|segment| segment == "crate")
            && let Some(target) = segments.get(1)
        {
            self.targets.insert(target.clone());
        }
        visit::visit_path(self, path);
    }
}

fn crate_targets(syntax: &File) -> HashSet<String> {
    let mut targets = HashSet::new();
    for item in &syntax.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        let mut imports = Vec::new();
        collect_dependency_imports(&mut Vec::new(), &item_use.tree, &mut imports);
        for import in imports {
            if let Some(path) = import.strip_prefix("crate::")
                && let Some(target) = path.split("::").next()
            {
                targets.insert(target.to_owned());
            }
        }
    }
    let mut visitor = CratePathVisitor::default();
    visitor.visit_file(syntax);
    targets.extend(visitor.targets);
    targets
}

fn collect_dependency_imports(prefix: &mut Vec<String>, tree: &UseTree, imports: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_dependency_imports(prefix, &path.tree, imports);
            prefix.pop();
        }
        UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            imports.push(prefix.join("::"));
            prefix.pop();
        }
        UseTree::Rename(rename) => {
            prefix.push(rename.ident.to_string());
            imports.push(prefix.join("::"));
            prefix.pop();
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_dependency_imports(prefix, item, imports);
            }
        }
        UseTree::Glob(_) => imports.push(prefix.join("::")),
    }
}

fn validate_layer_dependencies(syntax: &File, allowed: &[&str]) -> Result<(), String> {
    let mut forbidden = crate_targets(syntax)
        .into_iter()
        .filter(|target| !allowed.contains(&target.as_str()))
        .collect::<Vec<_>>();
    forbidden.sort();
    if forbidden.is_empty() {
        Ok(())
    } else {
        Err(format!("forbidden layer dependencies: {forbidden:?}"))
    }
}

#[derive(Default)]
struct DomainTypeVisitor {
    forbidden: HashSet<String>,
}

impl<'ast> Visit<'ast> for DomainTypeVisitor {
    fn visit_type_path(&mut self, type_path: &'ast syn::TypePath) {
        for segment in &type_path.path.segments {
            let identifier = segment.ident.to_string();
            if FORBIDDEN_DOMAIN_TYPES.contains(&identifier.as_str()) {
                self.forbidden.insert(identifier);
            }
        }
        visit::visit_type_path(self, type_path);
    }
}

fn validate_domain_boundary(syntax: &File) -> Result<(), String> {
    let imports = imports_in(&syntax.items)?;
    let external = imports
        .into_iter()
        .filter(|path| {
            !path.starts_with("std::")
                && !path.starts_with("core::")
                && !path.starts_with("super::")
                && !path.starts_with("self::")
                && !path.starts_with("crate::domain::")
                && ![
                    "anthropic",
                    "catalogue",
                    "error",
                    "executable",
                    "execution",
                    "identifiers",
                    "json",
                    "model",
                    "tokens",
                    "tool",
                ]
                .iter()
                .any(|module| path == module || path.starts_with(&format!("{module}::")))
        })
        .collect::<Vec<_>>();
    if !external.is_empty() {
        return Err(format!(
            "domain imports non-standard dependencies: {external:?}"
        ));
    }
    if syntax
        .items
        .iter()
        .any(|item| matches!(item, Item::Type(_)))
    {
        return Err(String::from(
            "transitional domain type aliases are forbidden",
        ));
    }
    let mut visitor = DomainTypeVisitor::default();
    visitor.visit_file(syntax);
    if !visitor.forbidden.is_empty() {
        let mut forbidden = visitor.forbidden.into_iter().collect::<Vec<_>>();
        forbidden.sort();
        return Err(format!(
            "domain exposes forbidden framework types: {forbidden:?}"
        ));
    }
    Ok(())
}

fn top_level_impl_count(syntax: &File, self_type: &str, port_trait: &str) -> Result<usize, String> {
    let mut count = 0;
    for item_impl in syntax.items.iter().filter_map(|item| match item {
        Item::Impl(item_impl) => Some(item_impl),
        _ => None,
    }) {
        let Some((_bang, trait_path, _for_token)) = &item_impl.trait_ else {
            continue;
        };
        let syn::Type::Path(self_path) = item_impl.self_ty.as_ref() else {
            continue;
        };
        let matches = trait_path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == port_trait)
            && self_path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == self_type);
        if !matches {
            continue;
        }
        if item_impl.attrs.iter().any(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        }) {
            return Err(format!(
                "required implementation {self_type}: {port_trait} may not be cfg-gated"
            ));
        }
        count += 1;
    }
    Ok(count)
}

fn bootstrap_function<'a>(syntax: &'a File, function: &str) -> Result<&'a syn::ItemFn, String> {
    syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(item_fn) if item_fn.sig.ident == function => Some(item_fn),
            _ => None,
        })
        .ok_or_else(|| format!("bootstrap function {function} is missing"))
}

fn tail_expression(function: &syn::ItemFn) -> Result<&Expr, String> {
    match function.block.stmts.last() {
        Some(syn::Stmt::Expr(expression, None)) => Ok(expression),
        _ => Err(format!(
            "bootstrap function {} must return one explicit tail expression",
            function.sig.ident
        )),
    }
}

fn pattern_identifier(pattern: &Pat) -> Option<&syn::Ident> {
    match pattern {
        Pat::Ident(identifier) => Some(&identifier.ident),
        Pat::Type(typed) => pattern_identifier(&typed.pat),
        _ => None,
    }
}

fn pattern_identifiers<'a>(pattern: &'a Pat, identifiers: &mut Vec<&'a syn::Ident>) {
    match pattern {
        Pat::Ident(identifier) => identifiers.push(&identifier.ident),
        Pat::Type(typed) => pattern_identifiers(&typed.pat, identifiers),
        Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                pattern_identifiers(element, identifiers);
            }
        }
        Pat::TupleStruct(tuple) => {
            for element in &tuple.elems {
                pattern_identifiers(element, identifiers);
            }
        }
        Pat::Struct(structure) => {
            for field in &structure.fields {
                pattern_identifiers(&field.pat, identifiers);
            }
        }
        Pat::Slice(slice) => {
            for element in &slice.elems {
                pattern_identifiers(element, identifiers);
            }
        }
        Pat::Reference(reference) => pattern_identifiers(&reference.pat, identifiers),
        Pat::Or(or_pattern) => {
            for case in &or_pattern.cases {
                pattern_identifiers(case, identifiers);
            }
        }
        _ => {}
    }
}

fn pattern_is_mutable(pattern: &Pat, variable: &str) -> bool {
    match pattern {
        Pat::Ident(identifier) => identifier.ident == variable && identifier.mutability.is_some(),
        Pat::Type(typed) => pattern_is_mutable(&typed.pat, variable),
        _ => false,
    }
}

fn local_binding_count(function: &syn::ItemFn, variable: &str) -> usize {
    function
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Local(local) => Some(&local.pat),
            _ => None,
        })
        .map(|pattern| {
            let mut identifiers = Vec::new();
            pattern_identifiers(pattern, &mut identifiers);
            identifiers
                .into_iter()
                .filter(|identifier| **identifier == variable)
                .count()
        })
        .sum()
}

fn require_unshadowed_parameter(function: &syn::ItemFn, variable: &str) -> Result<(), String> {
    let matching_parameters = function
        .sig
        .inputs
        .iter()
        .filter_map(|argument| match argument {
            syn::FnArg::Typed(typed) => match typed.pat.as_ref() {
                Pat::Ident(identifier) if identifier.ident == variable => Some(identifier),
                _ => None,
            },
            syn::FnArg::Receiver(_) => None,
        })
        .collect::<Vec<_>>();
    if matching_parameters.len() != 1 {
        return Err(format!(
            "bootstrap function {} must declare parameter {variable} exactly once",
            function.sig.ident
        ));
    }
    let parameter = matching_parameters
        .into_iter()
        .next()
        .ok_or_else(|| format!("bootstrap parameter {variable} disappeared"))?;
    if parameter.mutability.is_some() || parameter.by_ref.is_some() || parameter.subpat.is_some() {
        return Err(format!(
            "bootstrap function {} parameter {variable} must be a simple immutable binding",
            function.sig.ident
        ));
    }
    let count = local_binding_count(function, variable);
    if count == 0 {
        Ok(())
    } else {
        Err(format!(
            "bootstrap function {} shadows parameter {variable} {count} times",
            function.sig.ident
        ))
    }
}

fn local_initializer<'a>(function: &'a syn::ItemFn, variable: &str) -> Result<&'a Expr, String> {
    let initializers = function
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Local(local)
                if pattern_identifier(&local.pat)
                    .is_some_and(|identifier| identifier == variable) =>
            {
                local
                    .init
                    .as_ref()
                    .map(|initializer| initializer.expr.as_ref())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if initializers.len() != 1 {
        return Err(format!(
            "bootstrap function {} must initialize {variable} exactly once; found {}",
            function.sig.ident,
            initializers.len()
        ));
    }
    if function
        .block
        .stmts
        .iter()
        .any(|statement| match statement {
            syn::Stmt::Local(local) => {
                pattern_identifier(&local.pat).is_some_and(|identifier| identifier == variable)
                    && pattern_is_mutable(&local.pat, variable)
            }
            _ => false,
        })
    {
        return Err(format!(
            "bootstrap function {} may not mutate dependency binding {variable}",
            function.sig.ident
        ));
    }
    if local_binding_count(function, variable) != 1 {
        return Err(format!(
            "bootstrap function {} must bind {variable} exactly once across all patterns",
            function.sig.ident
        ));
    }
    initializers.into_iter().next().ok_or_else(|| {
        format!(
            "bootstrap function {} does not initialize {variable}",
            function.sig.ident
        )
    })
}

fn call_arguments<'a>(
    expression: &'a Expr,
    expected: &str,
) -> Result<&'a syn::punctuated::Punctuated<Expr, syn::token::Comma>, String> {
    let Expr::Call(call) = expression else {
        return Err(format!("expected call to {expected}"));
    };
    let Expr::Path(path) = call.func.as_ref() else {
        return Err(format!("expected path call to {expected}"));
    };
    if path_name(&path.path) != expected {
        return Err(format!(
            "expected call to {expected}, found {}",
            path_name(&path.path)
        ));
    }
    Ok(&call.args)
}

fn argument<'a>(
    arguments: &'a syn::punctuated::Punctuated<Expr, syn::token::Comma>,
    index: usize,
    context: &str,
) -> Result<&'a Expr, String> {
    arguments
        .iter()
        .nth(index)
        .ok_or_else(|| format!("{context} is missing argument {index}"))
}

fn require_path(expression: &Expr, expected: &str) -> Result<(), String> {
    let Expr::Path(path) = expression else {
        return Err(format!("expected path {expected}"));
    };
    if path_name(&path.path) == expected {
        Ok(())
    } else {
        Err(format!(
            "expected path {expected}, found {}",
            path_name(&path.path)
        ))
    }
}

fn require_method<'a>(expression: &'a Expr, method: &str) -> Result<&'a Expr, String> {
    let Expr::MethodCall(call) = expression else {
        return Err(format!("expected method call {method}"));
    };
    if call.method == method && call.args.is_empty() {
        Ok(&call.receiver)
    } else {
        Err(format!("expected zero-argument method call {method}"))
    }
}

fn validate_bootstrap_wiring(syntax: &File) -> Result<(), String> {
    let model_router = bootstrap_function(syntax, "model_router")?;
    require_unshadowed_parameter(model_router, "config")?;
    let anthropic = call_arguments(local_initializer(model_router, "anthropic")?, "Arc::new")?;
    let anthropic_constructor = argument(anthropic, 0, "Anthropic Arc")?;
    let Expr::Try(anthropic_constructor) = anthropic_constructor else {
        return Err(String::from("Anthropic constructor must propagate failure"));
    };
    if !call_arguments(&anthropic_constructor.expr, "ReqwestAnthropicGateway::new")?.is_empty() {
        return Err(String::from(
            "Anthropic constructor must not receive hidden inputs",
        ));
    }
    let model_router_tail = call_arguments(
        tail_expression(model_router)?,
        "model_router_with_anthropic",
    )?;
    require_path(
        argument(model_router_tail, 0, "model_router tail")?,
        "config",
    )?;
    require_path(
        argument(model_router_tail, 1, "model_router tail")?,
        "anthropic",
    )?;

    let injected = bootstrap_function(syntax, "model_router_with_anthropic")?;
    require_unshadowed_parameter(injected, "config")?;
    require_unshadowed_parameter(injected, "anthropic")?;
    let _working_directory = local_initializer(injected, "working_directory")?;
    let session_arc = call_arguments(local_initializer(injected, "session_factory")?, "Arc::new")?;
    let session_constructor = call_arguments(
        argument(session_arc, 0, "session factory Arc")?,
        "AppServerFactory::new",
    )?;
    let executable = argument(session_constructor, 0, "App Server factory")?;
    let executable = require_method(executable, "clone")?;
    let config_receiver = require_method(executable, "codex_executable")?;
    require_path(config_receiver, "config")?;
    let catalogue_clone = call_arguments(
        argument(session_constructor, 1, "App Server factory")?,
        "Arc::clone",
    )?;
    let catalogue = require_method(
        argument(catalogue_clone, 0, "App Server catalogue")?,
        "catalogue",
    )?;
    require_path(catalogue, "config")?;
    if session_constructor.len() != 2 {
        return Err(String::from(
            "AppServerFactory must receive exactly the executable and catalogue snapshot",
        ));
    }

    let ok = call_arguments(tail_expression(injected)?, "Ok")?;
    let router_arc = call_arguments(argument(ok, 0, "router result")?, "Arc::new")?;
    let service = call_arguments(
        argument(router_arc, 0, "router Arc")?,
        "ModelRouterService::new",
    )?;
    if service.len() != 4 {
        return Err(String::from(
            "ModelRouterService must receive exactly four dependencies",
        ));
    }
    let credential = call_arguments(
        argument(service, 0, "ModelRouterService")?,
        "ExpectedCredential::new",
    )?;
    let bearer = require_method(argument(credential, 0, "ExpectedCredential")?, "bearer")?;
    require_path(bearer, "config")?;
    let working_directory = call_arguments(
        argument(service, 1, "ModelRouterService")?,
        "WorkingDirectory::new",
    )?;
    require_path(
        argument(working_directory, 0, "WorkingDirectory")?,
        "working_directory",
    )?;
    require_path(argument(service, 2, "ModelRouterService")?, "anthropic")?;
    require_path(
        argument(service, 3, "ModelRouterService")?,
        "session_factory",
    )?;

    let http_router = bootstrap_function(syntax, "http_router")?;
    require_unshadowed_parameter(http_router, "model_router")?;
    require_unshadowed_parameter(http_router, "catalogue")?;
    let http_ok = call_arguments(tail_expression(http_router)?, "Ok")?;
    let http = call_arguments(argument(http_ok, 0, "HTTP router result")?, "http::router")?;
    require_path(argument(http, 0, "HTTP router")?, "model_router")?;
    require_path(argument(http, 1, "HTTP router")?, "catalogue")?;
    if http.len() != 2 {
        return Err(String::from(
            "HTTP router must receive exactly the model router and catalogue snapshot",
        ));
    }
    Ok(())
}

fn require_anthropic_gateway<T: model_rocket::ports::AnthropicGateway>() {}

fn require_model_session<T: model_rocket::ports::ModelSession>() {}

fn require_model_session_factory<T: model_rocket::ports::ModelSessionFactory>() {}

#[test]
fn ports_contain_only_approved_declarations_and_types() -> Result<(), Box<dyn std::error::Error>> {
    let files = port_files()?;
    assert!(
        !files.is_empty(),
        "the required port declarations are missing"
    );
    for path in files {
        let syntax = parse(&path)?;
        validate_literal_free(&syntax).map_err(|error| architecture_error(&path, &error))?;
        validate_port_items(&syntax.items).map_err(|error| architecture_error(&path, &error))?;
        validate_port_types(&syntax).map_err(|error| architecture_error(&path, &error))?;
    }
    Ok(())
}

#[test]
fn adapters_contain_no_inline_literals() -> Result<(), Box<dyn std::error::Error>> {
    for path in rust_files(Path::new(ADAPTER_DIRECTORY))? {
        let syntax = parse(&path)?;
        validate_literal_free(&syntax).map_err(|error| architecture_error(&path, &error))?;
    }
    Ok(())
}

#[test]
fn production_logging_sites_are_a_closed_secret_free_set() -> Result<(), Box<dyn std::error::Error>>
{
    let mut actual = HashSet::new();
    for path in rust_files(Path::new("src"))? {
        let mut visitor = LoggingVisitor::default();
        visitor.visit_file(&parse(&path)?);
        for (name, tokens) in visitor.calls {
            actual.insert((path.display().to_string(), name, tokens));
        }
    }
    let expected = HashSet::from([
        (
            String::from("src/main.rs"),
            String::from("info"),
            String::from("address = % local_addr , \"bridge listening\""),
        ),
        (
            String::from("src/contracts/codex/diagnostics.rs"),
            String::from("tracing::warn"),
            String::from(
                "path = % path . display () , % error , \"cannot remove isolated Codex home\"",
            ),
        ),
        (
            String::from("src/contracts/codex/diagnostics.rs"),
            String::from("tracing::warn"),
            String::from("% error , \"cannot terminate failed Codex App Server process\""),
        ),
    ]);
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn final_module_tree_exists_without_legacy_facades() {
    for required in REQUIRED_ARCHITECTURE_PATHS {
        assert!(
            Path::new(required).is_file(),
            "required boundary is missing: {required}"
        );
    }
    for forbidden in FORBIDDEN_LEGACY_PATHS {
        assert!(
            !Path::new(forbidden).exists(),
            "legacy or parallel implementation remains: {forbidden}"
        );
    }
}

#[test]
fn domain_is_std_only_and_contains_no_framework_escape_types()
-> Result<(), Box<dyn std::error::Error>> {
    for path in rust_files(Path::new("src/domain"))? {
        let syntax = parse(&path)?;
        validate_domain_boundary(&syntax).map_err(|error| architecture_error(&path, &error))?;
    }
    Ok(())
}

#[test]
fn layer_dependencies_point_inward() -> Result<(), Box<dyn std::error::Error>> {
    for (directory, allowed) in [
        ("src/domain", &["domain"][..]),
        ("src/application", &["domain", "policies", "ports"][..]),
        (
            "src/contracts",
            &["contracts", "domain", "policies", "product"][..],
        ),
        ("src/policies", &["domain", "policies", "product"][..]),
        (
            "src/adapters/inbound",
            &["contracts", "domain", "policies", "ports"][..],
        ),
        (
            "src/adapters/outbound",
            &["contracts", "domain", "policies", "ports", "product"][..],
        ),
    ] {
        for path in rust_files(Path::new(directory))? {
            let syntax = parse(&path)?;
            validate_layer_dependencies(&syntax, allowed)
                .map_err(|error| architecture_error(&path, &error))?;
        }
    }
    for path in port_files()? {
        let syntax = parse(&path)?;
        validate_layer_dependencies(&syntax, &["domain"])
            .map_err(|error| architecture_error(&path, &error))?;
    }
    Ok(())
}

#[test]
fn exact_boundary_types_implement_every_port() -> Result<(), Box<dyn std::error::Error>> {
    for (path, self_type, port_trait) in [
        (
            "src/adapters/outbound/anthropic.rs",
            "ReqwestAnthropicGateway",
            "AnthropicGateway",
        ),
        (
            "src/adapters/outbound/codex/mod.rs",
            "AppServer",
            "ModelSession",
        ),
        (
            "src/adapters/outbound/codex/mod.rs",
            "AppServerFactory",
            "ModelSessionFactory",
        ),
        (
            "src/adapters/inbound/http.rs",
            "ChannelModelResponseSink",
            "ModelResponseSink",
        ),
        (
            "src/application/model_router.rs",
            "RoutedModelOutput",
            "ModelOutput",
        ),
        (
            "src/application/model_router.rs",
            "RoutedAnthropicOutput",
            "AnthropicResponseSink",
        ),
        (
            "src/application/model_router.rs",
            "ModelRouterService",
            "ModelRouter",
        ),
    ] {
        let count = top_level_impl_count(&parse(Path::new(path))?, self_type, port_trait)
            .map_err(std::io::Error::other)?;
        assert!(
            count == 1,
            "{path} must contain exactly one unconditional top-level implementation: {self_type} implements {port_trait}; found {count}"
        );
    }

    require_anthropic_gateway::<model_rocket::adapters::outbound::anthropic::ReqwestAnthropicGateway>(
    );
    require_model_session::<model_rocket::adapters::outbound::codex::AppServer>();
    require_model_session_factory::<model_rocket::adapters::outbound::codex::AppServerFactory>();
    Ok(())
}

#[test]
fn bootstrap_wires_every_port_without_a_concrete_application_dependency()
-> Result<(), Box<dyn std::error::Error>> {
    type RouterResult = Result<
        std::sync::Arc<dyn model_rocket::ports::ModelRouter>,
        model_rocket::domain::BridgeError,
    >;
    type RouterWithAnthropic = fn(
        &model_rocket::config::Config,
        std::sync::Arc<dyn model_rocket::ports::AnthropicGateway>,
    ) -> RouterResult;
    type HttpRouter = fn(
        std::sync::Arc<dyn model_rocket::ports::ModelRouter>,
        std::sync::Arc<model_rocket::domain::ModelCatalogue>,
    ) -> Result<axum::Router, model_rocket::domain::BridgeError>;

    let syntax = parse(Path::new("src/bootstrap.rs"))?;
    validate_bootstrap_wiring(&syntax).map_err(std::io::Error::other)?;

    let _: fn(&model_rocket::config::Config) -> RouterResult =
        model_rocket::bootstrap::model_router;
    let _: RouterWithAnthropic = model_rocket::bootstrap::model_router_with_anthropic;
    let _: HttpRouter = model_rocket::bootstrap::http_router;
    Ok(())
}

#[test]
fn architecture_gate_rejects_nested_impl_and_discarded_constructor_decoys()
-> Result<(), Box<dyn std::error::Error>> {
    let nested = syn::parse_file(
        "mod decoy { impl ModelRouter for ModelRouterService { fn dispatch(&self) {} } }",
    )?;
    assert_eq!(
        top_level_impl_count(&nested, "ModelRouterService", "ModelRouter")?,
        0
    );
    for gated in [
        "#[cfg(any())] impl ModelRouter for ModelRouterService {}",
        "#[cfg_attr(target_os = \"linux\", cfg(any()))] impl ModelRouter for ModelRouterService {}",
    ] {
        let gated = syn::parse_file(gated)?;
        assert!(top_level_impl_count(&gated, "ModelRouterService", "ModelRouter").is_err());
    }

    let discarded = syn::parse_file(
        r"
        fn model_router(config: &Config) {
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic);
            unrelated_router()
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) {
            let session_factory = Arc::new(AppServerFactory::new(config.codex_executable().clone()));
            ModelRouterService::new(a(), b(), anthropic, session_factory);
            unrelated_router()
        }
        fn http_router(model_router: Router) {
            http::router(model_router);
            unrelated_router()
        }
        ",
    )?;
    assert!(validate_bootstrap_wiring(&discarded).is_err());

    let shadowed = syn::parse_file(
        r"
        fn model_router(config: &Config) {
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            let anthropic = unrelated_gateway();
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) {
            let working_directory = working_directory();
            let session_factory = Arc::new(AppServerFactory::new(config.codex_executable().clone()));
            let session_factory = unrelated_factory();
            Ok(Arc::new(ModelRouterService::new(a(), b(), anthropic, session_factory)))
        }
        fn http_router(model_router: Router) {
            Ok(http::router(model_router))
        }
        ",
    )?;
    assert!(validate_bootstrap_wiring(&shadowed).is_err());

    let parameter_shadow = syn::parse_file(
        r"
        fn model_router(config: &Config) {
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) {
            let anthropic = unrelated_gateway();
            unrelated_router()
        }
        fn http_router(model_router: Router) {
            Ok(http::router(model_router))
        }
        ",
    )?;
    assert!(validate_bootstrap_wiring(&parameter_shadow).is_err());

    let mutable_dependency = syn::parse_file(
        r"
        fn model_router(config: &Config) {
            let mut anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) {
            unrelated_router()
        }
        fn http_router(model_router: Router) {
            Ok(http::router(model_router))
        }
        ",
    )?;
    assert!(validate_bootstrap_wiring(&mutable_dependency).is_err());
    Ok(())
}

#[test]
fn architecture_gate_rejects_mutable_parameter_reassignment()
-> Result<(), Box<dyn std::error::Error>> {
    for mutable_parameter in [
        r"
        fn model_router(mut config: &Config) {
            config = unrelated_config();
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) { unrelated_router() }
        fn http_router(model_router: Router) { Ok(http::router(model_router)) }
        ",
        r"
        fn model_router(config: &Config) {
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, mut anthropic: Gateway) {
            anthropic = unrelated_gateway();
            let working_directory = working_directory();
            let session_factory = Arc::new(AppServerFactory::new(config.codex_executable().clone()));
            Ok(Arc::new(ModelRouterService::new(ExpectedCredential::new(config.bearer()), WorkingDirectory::new(working_directory), anthropic, session_factory)))
        }
        fn http_router(model_router: Router) { Ok(http::router(model_router)) }
        ",
        r"
        fn model_router(config: &Config) {
            let anthropic = Arc::new(ReqwestAnthropicGateway::new()?);
            model_router_with_anthropic(config, anthropic)
        }
        fn model_router_with_anthropic(config: &Config, anthropic: Gateway) { unrelated_router() }
        fn http_router(mut model_router: Router) {
            model_router = unrelated_router();
            Ok(http::router(model_router))
        }
        ",
    ] {
        let mutable_parameter = syn::parse_file(mutable_parameter)?;
        assert!(validate_bootstrap_wiring(&mutable_parameter).is_err());
    }
    Ok(())
}

#[test]
fn inbound_adapter_cannot_import_outbound_adapter() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "use crate::adapters::outbound::codex::AppServerFactory;",
        "fn forbidden() { crate::adapters::outbound::anthropic::ReqwestAnthropicGateway::new(); }",
    ] {
        let syntax = syn::parse_file(source)?;
        assert!(
            validate_layer_dependencies(&syntax, &["contracts", "domain", "policies", "ports"])
                .is_err(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn port_gate_rejects_unapproved_primitive_and_framework_types()
-> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "trait Bad { fn primitive(&self, value: &str, count: usize); }",
        "trait Bad { fn framework(&self, body: axum::Body); }",
    ] {
        let syntax = syn::parse_file(source)?;
        assert!(validate_port_types(&syntax).is_err());
    }
    Ok(())
}

#[test]
fn literal_gate_checks_non_documentation_attributes() -> Result<(), Box<dyn std::error::Error>> {
    let syntax = syn::parse_file("#[serde(rename = \"wire-name\")] struct Adapter;")?;
    assert!(validate_literal_free(&syntax).is_err());
    let documentation = syn::parse_file("/// permitted documentation\nstruct Adapter;")?;
    assert!(validate_literal_free(&documentation).is_ok());
    Ok(())
}

#[test]
fn port_gate_rejects_data_hidden_in_nested_modules() -> Result<(), Box<dyn std::error::Error>> {
    let syntax = syn::parse_file("mod hidden { struct PortDto; }")?;
    assert!(validate_port_items(&syntax.items).is_err());
    Ok(())
}

#[test]
fn port_gate_rejects_trait_level_type_escape_hatches() -> Result<(), Box<dyn std::error::Error>> {
    let invalid_sources = [
        "trait ModelOutput { fn passthrough<T>(&self, value: T); }",
        "trait ModelOutput { type TextDelta; fn emit(&self, value: Self::TextDelta); }",
        "trait ModelOutput { fn emit(&self, value: alien::TextDelta); }",
        "trait ModelOutput { fn emit(&self) {} }",
    ];
    for source in invalid_sources {
        let syntax = syn::parse_file(source)?;
        let item_result = validate_port_items(&syntax.items);
        let type_result = validate_port_types(&syntax);
        assert!(item_result.is_err() || type_result.is_err(), "{source}");
    }
    Ok(())
}

#[test]
fn domain_gate_rejects_renamed_framework_wrappers() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "struct Workspace(std::path::PathBuf);",
        "struct Document(serde_json::Value);",
        "struct Stream(tokio::sync::mpsc::Sender<String>);",
        "struct Payload(axum::body::Bytes);",
    ] {
        let syntax = syn::parse_file(source)?;
        assert!(validate_domain_boundary(&syntax).is_err(), "{source}");
    }
    Ok(())
}

#[test]
fn dependency_gate_rejects_outward_layer_imports() -> Result<(), Box<dyn std::error::Error>> {
    for (source, allowed) in [
        (
            "use crate::adapters::outbound::codex::AppServerFactory;",
            &["domain", "policies", "ports"][..],
        ),
        (
            "use crate::application::ModelRouterService;",
            &["adapters", "contracts", "domain", "policies", "ports"][..],
        ),
        (
            "use crate::config::Config;",
            &["contracts", "domain", "policies", "product"][..],
        ),
    ] {
        let syntax = syn::parse_file(source)?;
        assert!(
            validate_layer_dependencies(&syntax, allowed).is_err(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn exact_port_gate_rejects_missing_or_extra_methods() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "trait ModelOutput {}",
        "trait ModelOutput { fn emit(&self); fn flush(&self); }",
        "trait AnthropicGateway { fn request(&self); }",
    ] {
        let syntax = syn::parse_file(source)?;
        let Some(Item::Trait(item_trait)) = syntax.items.first() else {
            return Err("trait fixture was not parsed as a trait".into());
        };
        assert!(validate_port_trait(item_trait).is_err(), "{source}");
    }
    Ok(())
}

fn validate_app_server_launch_capability(adapter: &File) -> Result<(), Box<dyn std::error::Error>> {
    let launch_methods = adapter
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item_impl) => Some(item_impl),
            _ => None,
        })
        .filter(|item_impl| match item_impl.self_ty.as_ref() {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "AppServer"),
            _ => false,
        })
        .flat_map(|item_impl| &item_impl.items)
        .filter_map(|item| match item {
            ImplItem::Fn(method) if method.sig.ident == "launch" => Some(method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        launch_methods.len(),
        1,
        "AppServer must expose exactly one inherent launch method"
    );
    let launch = launch_methods
        .first()
        .ok_or("AppServer::launch is missing")?;
    assert!(matches!(launch.vis, Visibility::Public(_)));
    assert!(launch.sig.asyncness.is_some());
    let executable = launch
        .sig
        .inputs
        .first()
        .ok_or("AppServer::launch has no executable argument")?;
    let FnArg::Typed(executable) = executable else {
        return Err("AppServer::launch must not take self".into());
    };
    let Type::Reference(reference) = executable.ty.as_ref() else {
        return Err("AppServer::launch executable must be borrowed".into());
    };
    let Type::Path(executable_type) = reference.elem.as_ref() else {
        return Err("AppServer::launch executable must use a named capability".into());
    };
    assert_eq!(
        executable_type
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .as_deref(),
        Some("ValidatedCodexExecutable")
    );
    Ok(())
}

#[test]
fn production_executable_capability_cannot_be_forged_or_bypassed()
-> Result<(), Box<dyn std::error::Error>> {
    let domain = parse(Path::new("src/domain/executable.rs"))?;
    let capability = domain
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item_struct) if item_struct.ident == "ValidatedCodexExecutable" => {
                Some(item_struct)
            }
            _ => None,
        })
        .ok_or("validated executable capability is missing")?;
    let Fields::Named(fields) = &capability.fields else {
        return Err("validated executable capability must use named private fields".into());
    };
    assert!(
        fields
            .named
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited)),
        "validated executable capability fields must remain private"
    );

    let public_capability_methods = domain
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item_impl) => Some(item_impl),
            _ => None,
        })
        .flat_map(|item_impl| &item_impl.items)
        .filter_map(|item| match item {
            ImplItem::Fn(method) if matches!(method.vis, Visibility::Public(_)) => {
                Some(method.sig.ident.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(public_capability_methods, ["as_str"]);

    let config = parse(Path::new("src/config.rs"))?;
    let config_struct = config
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item_struct) if item_struct.ident == "Config" => Some(item_struct),
            _ => None,
        })
        .ok_or("production Config is missing")?;
    let Fields::Named(fields) = &config_struct.fields else {
        return Err("production Config must use named private fields".into());
    };
    assert!(
        fields
            .named
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited)),
        "production Config fields must remain private"
    );

    let adapter = parse(Path::new("src/adapters/outbound/codex/mod.rs"))?;
    validate_app_server_launch_capability(&adapter)?;

    let config_source = fs::read_to_string("src/config.rs")?;
    assert!(config_source.contains("#[cfg(feature = \"test-support\")]\n    pub fn test_fixture("));
    Ok(())
}
