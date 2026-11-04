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
