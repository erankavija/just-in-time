use std::collections::BTreeSet;
use std::path::Path;
use syn::visit::Visit;
use syn::{ImplItem, ItemImpl, ItemStruct, ReturnType, Type, Visibility};

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|part| part.ident.to_string()),
        _ => None,
    }
}

#[derive(Default)]
struct Surface {
    facades: BTreeSet<String>,
    methods: Vec<(String, Option<String>)>,
}

impl<'ast> Visit<'ast> for Surface {
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if matches!(item.vis, Visibility::Public(_))
            && item.fields.iter().any(|field| {
                matches!(&field.ty, Type::Reference(reference)
                    if type_name(&reference.elem).as_deref() == Some("CommandExecutor"))
            })
        {
            self.facades.insert(item.ident.to_string());
        }
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        if type_name(&item.self_ty).as_deref() == Some("CommandExecutor") {
            self.methods.extend(item.items.iter().filter_map(|member| {
                let ImplItem::Fn(method) = member else {
                    return None;
                };
                matches!(method.vis, Visibility::Public(_)).then(|| {
                    let output = match &method.sig.output {
                        ReturnType::Type(_, ty) => type_name(ty),
                        ReturnType::Default => None,
                    };
                    (method.sig.ident.to_string(), output)
                })
            }));
        }
        syn::visit::visit_item_impl(self, item);
    }
}

impl Surface {
    fn scan(&mut self, directory: &Path) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                self.scan(&path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                self.visit_file(&syn::parse_file(&source).unwrap());
            }
        }
    }

    fn violations(&self) -> Vec<&str> {
        self.methods
            .iter()
            .filter(|(name, output)| {
                !matches!(name.as_str(), "new" | "with_layout" | "with_worktree_paths")
                    && !output.as_ref().is_some_and(|ty| self.facades.contains(ty))
            })
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

#[test]
fn test_command_executor_public_surface_is_constructors_and_accessors() {
    let mut surface = Surface::default();
    surface.scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands"));
    assert!(!surface.methods.is_empty(), "no executor methods found");
    assert!(!surface.facades.is_empty(), "no family facades found");
    assert!(
        surface.violations().is_empty(),
        "executor kernel methods must be crate-private: {:?}",
        surface.violations()
    );
}

#[test]
fn test_surface_scan_distinguishes_facades_from_kernel_and_unrelated_methods() {
    let mut surface = Surface::default();
    surface.visit_file(
        &syn::parse_file(
            r#"
        pub struct IssueOps<'a, S> { exec: &'a CommandExecutor<S> }
        pub struct PretendOps;
        impl<S> CommandExecutor<S> {
            pub fn new(storage: S) -> Self { todo!() }
            pub fn issues(&self) -> IssueOps<'_, S> { todo!() }
            pub(crate) fn rules(&self) -> RuleSet { todo!() }
            pub fn storage(&self) -> &S { todo!() }
            pub fn bypass(&self) -> PretendOps { todo!() }
        }
        impl Unrelated { pub fn allowed(&self) {} }
        mod nested {
            impl CommandExecutor<Concrete> {
                pub fn another_bypass(&self) {}
            }
        }
    "#,
        )
        .unwrap(),
    );
    assert_eq!(
        surface.violations(),
        ["storage", "bypass", "another_bypass"]
    );
}

#[test]
fn test_command_modules_do_not_depend_on_output() {
    fn scan(directory: &Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, offenders);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("crate::output")
            {
                offenders.push(path.display().to_string());
            }
        }
    }
    let mut offenders = Vec::new();
    scan(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands"),
        &mut offenders,
    );
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "commands must not name crate::output:\n{}",
        offenders.join("\n")
    );
}

#[derive(Default)]
struct StorageCalls {
    aliases: std::collections::BTreeMap<String, Vec<String>>,
    receivers: BTreeSet<String>,
    violations: Vec<String>,
}

impl StorageCalls {
    fn imports(&mut self, tree: &syn::UseTree, prefix: &[String]) {
        match tree {
            syn::UseTree::Path(path) => {
                let mut prefix = prefix.to_vec();
                prefix.push(path.ident.to_string());
                self.imports(&path.tree, &prefix);
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.imports(tree, prefix);
                }
            }
            syn::UseTree::Name(name) => {
                let mut path = prefix.to_vec();
                path.push(name.ident.to_string());
                self.aliases.insert(name.ident.to_string(), path);
            }
            syn::UseTree::Rename(rename) => {
                let mut path = prefix.to_vec();
                path.push(rename.ident.to_string());
                self.aliases.insert(rename.rename.to_string(), path);
            }
            syn::UseTree::Glob(_) => {}
        }
    }

    fn storage_path(&self, path: &syn::Path) -> bool {
        let mut parts = path
            .segments
            .iter()
            .map(|part| part.ident.to_string())
            .collect::<Vec<_>>();
        if let Some(alias) = parts.first().and_then(|first| self.aliases.get(first)) {
            parts.splice(..1, alias.iter().cloned());
        }
        parts.iter().any(|part| part == "storage")
            && !parts
                .iter()
                .any(|part| part.ends_with("Error") || part == "is_lock_timeout")
    }

    fn scan(&mut self, path: &Path) {
        let source = std::fs::read_to_string(path).unwrap();
        let file = syn::parse_file(&source).unwrap();
        self.visit_file(&file);
        for item in file.items {
            if let syn::Item::Mod(module) = item {
                if module.content.is_none() && !test_only(&module.attrs) {
                    self.scan(&path.parent().unwrap().join(format!("{}.rs", module.ident)));
                }
            }
        }
    }
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Path>()
                    .is_ok_and(|path| path.is_ident("test")))
    })
}

impl<'ast> Visit<'ast> for StorageCalls {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if !test_only(&item.attrs) {
            self.imports(&item.tree, &[]);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if test_only(&item.attrs) {
            return;
        }
        let saved = self.receivers.clone();
        let store_types = item.sig.generics.type_params().filter(|parameter| {
            parameter.bounds.iter().any(|bound| matches!(bound, syn::TypeParamBound::Trait(bound)
                if bound.path.segments.last().is_some_and(|part|
                    matches!(part.ident.to_string().as_str(), "IssueStore" | "RepositoryStateStore" | "HandlerStore"))))
        }).map(|parameter| parameter.ident.to_string()).collect::<BTreeSet<_>>();
        for argument in &item.sig.inputs {
            if let syn::FnArg::Typed(argument) = argument {
                let ty = match argument.ty.as_ref() {
                    Type::Reference(reference) => reference.elem.as_ref(),
                    ty => ty,
                };
                if type_name(ty)
                    .is_some_and(|name| store_types.contains(&name) || name == "JsonFileStorage")
                {
                    if let syn::Pat::Ident(name) = argument.pat.as_ref() {
                        self.receivers.insert(name.ident.to_string());
                    }
                }
            }
        }
        syn::visit::visit_item_fn(self, item);
        self.receivers = saved;
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "storage"
            || matches!(call.receiver.as_ref(), syn::Expr::Path(path)
            if path.path.get_ident().is_some_and(|name| self.receivers.contains(&name.to_string())))
        {
            self.violations.push(call.method.to_string());
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if self.storage_path(&path.path) {
                self.violations
                    .push(path.path.segments.last().unwrap().ident.to_string());
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

#[test]
fn test_cli_command_path_has_no_storage_calls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = StorageCalls::default();
    for file in ["main.rs", "cli.rs", "dispatch/mod.rs"] {
        calls.scan(&root.join(file));
    }
    assert!(
        calls.violations.is_empty(),
        "CLI storage calls: {:?}",
        calls.violations
    );
}

#[test]
fn test_storage_call_scan_distinguishes_calls_from_bounds_errors_and_fixtures() {
    let mut calls = StorageCalls::default();
    calls.visit_file(
        &syn::parse_file(
            r#"
        use crate::storage::{JsonFileStorage as Files, IssueStore, IssueNotFoundError};
        fn forbidden<S: IssueStore>(backend: &S) {
            let files = Files::new("root");
            backend.resolve_issue_id(&prefix);
            executor.storage().list_issues();
            crate::storage::resolve_dependency_id(&prefix, &[]);
        }
        fn allowed<S: IssueStore>(context: &CommandContext<S>) {
            context.executor.issues().resolve_issue_id(&prefix);
            error.is::<IssueNotFoundError>();
            crate::storage::is_lock_timeout(&error);
        }
        #[cfg(test)] mod fixtures {
            fn setup() { Files::new("fixture"); executor.storage().list_issues(); }
        }
    "#,
        )
        .unwrap(),
    );
    assert_eq!(
        calls.violations,
        [
            "new",
            "resolve_issue_id",
            "storage",
            "resolve_dependency_id"
        ]
    );
}
