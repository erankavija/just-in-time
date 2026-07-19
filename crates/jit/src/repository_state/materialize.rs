//! Pure materialization producers driven from a captured [`RepositoryImage`].
//!
//! Every expected materialization derives from declared authority: the configured
//! projections and the registries the producers read are taken from the captured
//! declaration bytes in the image, and existing target bytes are read from the same
//! image (never from the live filesystem). Shared targets are composed by the ONE
//! [`managed_document`](super::managed_document) primitive, so two projections into
//! one file compose deterministically rather than last-writer-wins. The result is a
//! set of exact [`RepositoryAction`]s for every target whose bytes differ from the
//! captured occupant; an unchanged target contributes no action.

use super::{
    compose_managed_documents, default_ruleset, parse_rule_identities, render_projection_body,
    render_rule_block, rewrite_header, rules_file_header, serialize_ruleset,
    splice_default_membership, ExpectedPreimage, FileMode, ManagedDocumentClaim, ProjectionInputs,
    RegionPlacement, RepositoryAction, RepositoryDeclarations, RepositoryEntry, RepositoryImage,
    RepositoryStateError, VirtualPath,
};
use crate::config::{JitConfig, ProjectionMode};
use crate::declarations::invariants::InvariantRegistry;
use crate::repository_state::default_rule_membership_diff_from_identities;

/// Map a repo-relative producer path onto its canonical [`VirtualPath`].
///
/// The projection producers address the engine registries and worktree targets by
/// their logical `.jit/...` / worktree-relative spellings; a `.jit/`-prefixed path
/// is a `Data(...)` target and every other repo-relative path is `Worktree(...)`.
fn image_path(repo_relative: &str) -> Result<VirtualPath, RepositoryStateError> {
    let vpath = match repo_relative.strip_prefix(".jit/") {
        Some(rest) => VirtualPath::data(rest),
        None => VirtualPath::worktree(repo_relative),
    }?;
    Ok(vpath)
}

/// Read a repo-relative path as UTF-8 text from the captured image.
///
/// `Ok(None)` when the captured entry is absent; a producer requesting a path the
/// image never captured fails with `UndiscoveredRepositoryPath`, never silent
/// absence.
fn read_text(image: &RepositoryImage, repo_relative: &str) -> anyhow::Result<Option<String>> {
    let vpath = image_path(repo_relative)?;
    match image.file_bytes(&vpath)? {
        Some(bytes) => Ok(Some(String::from_utf8(bytes.to_vec())?)),
        None => Ok(None),
    }
}

/// Assemble the repository configuration from the captured declaration bytes.
///
/// `config.toml` supplies the item-kind registry, projection registry, and
/// validation settings; the sibling `invariants.toml` populates the `#[serde(skip)]`
/// invariant registry the `full` invariant view renders. An absent `invariants.toml`
/// is an empty registry, matching the on-disk load boundary.
pub fn assemble_config(image: &RepositoryImage) -> anyhow::Result<JitConfig> {
    let config_text = read_text(image, ".jit/config.toml")?
        .ok_or_else(|| anyhow::anyhow!("captured image has no .jit/config.toml"))?;
    let mut config: JitConfig = toml::from_str(&config_text)?;
    // An invariants.toml outside the captured closure (or captured absent) is an
    // empty registry, matching the on-disk load boundary.
    let invariants_path = image_path(".jit/invariants.toml")?;
    config.invariants = if image.capture_spec().contains_path(&invariants_path) {
        match read_text(image, ".jit/invariants.toml")? {
            Some(text) => InvariantRegistry::from_toml_str(&text)?,
            None => InvariantRegistry::empty(),
        }
    } else {
        InvariantRegistry::empty()
    };
    Ok(config)
}

/// Enumerate the phase-two capture closure for `jit project render`.
///
/// Given the phase-one configuration and the selected projection names (an empty
/// slice selects every declared projection), this returns every additional
/// canonical path the render must capture so it reads only image-projected
/// content: the four engine registries (`config.toml`, `invariants.toml`,
/// `rules.toml`, `gates.toml`), each selected projection's documentation target
/// (read for region-mode splicing), each projected kind's declared source, and —
/// when the phase-one `rules.toml` bytes are supplied — the schema files the
/// effective rule set references (needed to parse the effective rules the
/// full-style rule/gate view renders). The caller feeds these to
/// [`CaptureSpec::discover_paths`](super::CaptureSpec::discover_paths) after phase
/// one; the resulting closure is what `derive_project_render` reads from.
///
/// This planner enumerates exactly the paths
/// [`compose_configured_projections`] and the effective-rule assembly read, so a
/// selective `jit project render <name>` captures its own projection's sources and
/// target without pulling in a sibling projection's closure. An unknown selected
/// name or a projection referencing an unknown kind is an error; the command
/// validates names first, so this is a defensive guard rather than the primary
/// diagnostic.
pub fn render_capture_closure(
    config: &JitConfig,
    selected: &[String],
    rules_content: Option<&str>,
) -> anyhow::Result<Vec<VirtualPath>> {
    use crate::config::SourceOfTruth;
    use crate::domain::item::resolve_item_kinds;

    let mut paths = vec![
        image_path(".jit/config.toml")?,
        image_path(".jit/invariants.toml")?,
        image_path(".jit/rules.toml")?,
        image_path(".jit/gates.toml")?,
    ];
    let registry = config.projection.clone().unwrap_or_default();
    let all_kinds = resolve_item_kinds(config.item_kinds.as_ref())?;
    let names: Vec<String> = if selected.is_empty() {
        registry.keys().cloned().collect()
    } else {
        selected.to_vec()
    };
    for name in &names {
        let projection = registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown projection '{name}'"))?;
        paths.push(image_path(&super::require_target(projection, name)?)?);
        for kind_name in projection.kinds() {
            let kind = all_kinds
                .iter()
                .find(|k| k.name() == kind_name)
                .ok_or_else(|| {
                    anyhow::anyhow!("projection '{name}' references unknown kind '{kind_name}'")
                })?;
            match kind.source_of_truth() {
                SourceOfTruth::MarkdownFirst => {
                    if let Some(source) = kind.source() {
                        paths.push(image_path(source)?);
                    }
                }
                SourceOfTruth::RegistryFirst => {
                    if let Some(descriptor) = kind.toml_source() {
                        paths.push(image_path(&descriptor.toml)?);
                    }
                }
            }
        }
    }
    if let Some(content) = rules_content {
        // A rule schema reference is relative to the data root (`schemas/...`),
        // so it is a `Data(...)` path directly rather than a repo-relative one.
        for request in crate::declarations::rules::RuleSet::schema_requests(content)? {
            paths.push(VirtualPath::data(&request.reference)?);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Compose every configured projection into exact target actions.
///
/// Each projection's body is rendered by the single relocated
/// [`render_projection_body`] producer, and its target claim is composed through the
/// one [`managed_document`](super::managed_document) engine: a separate-file
/// projection is a whole-file base claim, a region projection a delimited region
/// claim, and several projections sharing a target collapse to one deterministic
/// composition. A composed target whose bytes match the captured occupant yields no
/// action.
pub(crate) fn compose_configured_projections(
    image: &RepositoryImage,
    config: &JitConfig,
    declarations: &RepositoryDeclarations<'_>,
) -> anyhow::Result<Vec<RepositoryAction>> {
    let Some(projections) = config.projection.as_ref() else {
        return Ok(Vec::new());
    };

    let inputs = ProjectionInputs {
        config,
        rules: declarations.rules,
        gates: declarations.gates,
    };

    // Phase one: render every projection body and build its managed-document claim.
    // Iterating the name-ordered projection registry keeps claim order deterministic.
    let mut claims: Vec<(VirtualPath, ManagedDocumentClaim)> = Vec::new();
    for (name, projection) in projections {
        let mut read = |path: &str| read_text(image, path);
        let (body, _count) = render_projection_body(projection, &inputs, &mut read)?;
        let target = super::require_target(projection, name)?;
        let vpath = image_path(&target)?;
        let owner = format!("projection:{name}");
        let claim = match projection.mode() {
            ProjectionMode::SeparateFile => ManagedDocumentClaim::Base {
                owner,
                bytes: body.into_bytes(),
            },
            ProjectionMode::Region => ManagedDocumentClaim::Region {
                owner,
                region_id: name.clone(),
                begin: projection.region_begin(name).into_bytes(),
                end: projection.region_end(name).into_bytes(),
                content: body.into_bytes(),
                placement: RegionPlacement::RequireExisting,
            },
        };
        claims.push((vpath, claim));
    }

    // Phase two: compose all claims per target once (deterministic, never
    // last-writer-wins), then emit an action only where the composed bytes differ
    // from the captured occupant.
    let composed = compose_managed_documents(image, claims)?;
    let mut actions = Vec::new();
    for (vpath, expected_bytes) in composed {
        let entry = image.entry(&vpath)?;
        if let RepositoryEntry::File { bytes, mode, .. } = entry {
            if bytes == &expected_bytes && *mode == FileMode::Regular {
                continue;
            }
        }
        actions.push(RepositoryAction::WriteFile {
            path: vpath.clone(),
            owner: "configured-projection".to_string(),
            expected: ExpectedPreimage::of(entry),
            bytes: expected_bytes,
            mode: FileMode::Regular,
        });
    }
    Ok(actions)
}

/// Materialize the default-rule family (`rules.toml`) and its baked schema files
/// from the captured registry, obeying the rules.toml ownership matrix.
///
/// `rules.toml` is the authored file: the default-family membership (the
/// `namespace-unique-*` rows the registry generates) and the generated header are
/// spliced in place — via the one span-level
/// [`splice_default_membership`]/[`rewrite_header`] primitive — while custom rows,
/// comments, block order, and the editable policy fields of default rules are
/// preserved unconditionally. Config owns the default schema CONTENT
/// (`schemas/default-*.json`), whose expected bytes are written to each captured
/// target. An absent `rules.toml` is the in-memory-defaults case: nothing is
/// materialized (the read path builds the defaults without writing), so this
/// contributes no action.
///
/// Ownership throughout this pass is keyed by rule NAME — the membership diff, the
/// span drop, and the schema-generation classification all identify a default rule
/// by its name. A `rules.toml` carrying two rules of the same name is invalid and
/// makes that key non-injective, so no name can be proven to belong to the default
/// family or to a custom rule. The entire rules/schema pass is therefore poisoned
/// (not merely the colliding boundary) and returns
/// [`AmbiguousOwnership`](RepositoryStateError::AmbiguousOwnership) before emitting
/// any action, so repair never rewrites or deletes an authored boundary it cannot
/// prove (ownership matrix: "if exact ownership/spans cannot be proven, return
/// non-repairable before publication").
pub(crate) fn compose_default_ruleset(
    image: &RepositoryImage,
    config: &JitConfig,
) -> Result<Vec<RepositoryAction>, RepositoryStateError> {
    // A ruleset outside the captured closure (or captured absent) is not an owned
    // materialization: defaults live only in memory, nothing on disk to keep
    // coherent, so no rules/schema targets are owned.
    let rules_path = image_path(".jit/rules.toml")?;
    if !image.capture_spec().contains_path(&rules_path) {
        return Ok(Vec::new());
    }
    let Some(current_rules) =
        read_text(image, ".jit/rules.toml").map_err(RepositoryStateError::producer)?
    else {
        return Ok(Vec::new());
    };
    let namespaces = crate::config_manager::namespaces_from_config(config);
    let mut actions = Vec::new();

    // Name-keyed ownership requires unique rule names; a duplicate makes ownership
    // unprovable across the whole pass, so refuse before any add/drop/delete.
    let identities =
        parse_rule_identities(&current_rules).map_err(RepositoryStateError::producer)?;
    if let Some(dup) = first_duplicate_rule_name(&identities) {
        return Err(RepositoryStateError::AmbiguousOwnership(format!(
            "rules.toml declares more than one rule named '{dup}'; \
             default-rule and schema ownership cannot be proven"
        )));
    }

    // rules.toml: splice only the generated default-family membership + header,
    // preserving every authored byte outside those proven-generated spans.
    let diff = default_rule_membership_diff_from_identities(&identities, &namespaces);
    let rendered_add: Vec<String> = diff.to_add.iter().map(render_rule_block).collect();
    let spliced = splice_default_membership(&current_rules, &rendered_add, &diff.to_drop)
        .map_err(RepositoryStateError::producer)?;
    let expected_rules =
        rewrite_header(&spliced, rules_file_header()).map_err(RepositoryStateError::producer)?;
    if expected_rules != current_rules {
        actions.push(RepositoryAction::WriteFile {
            path: rules_path.clone(),
            owner: "default-rule-membership".to_string(),
            expected: ExpectedPreimage::of(entry(image, &rules_path)?),
            bytes: expected_rules.into_bytes(),
            mode: FileMode::Regular,
        });
    }

    // schemas/default-*.json: config/default-rule generator owns the content.
    // Write each expected target the image actually captured (a target outside
    // the closure is not owned here and never fabricated).
    let serialized = serialize_ruleset(&default_ruleset(&namespaces));
    let expected_names: std::collections::BTreeSet<&str> = serialized
        .schema_files
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    for schema in &serialized.schema_files {
        let vpath = image_path(&format!(".jit/schemas/{}", schema.name))?;
        if !image.capture_spec().contains_path(&vpath) {
            continue;
        }
        let occupant = entry(image, &vpath)?;
        if let RepositoryEntry::File { bytes, mode, .. } = occupant {
            if bytes == schema.content.as_bytes() && *mode == FileMode::Regular {
                continue;
            }
        }
        actions.push(RepositoryAction::WriteFile {
            path: vpath.clone(),
            owner: "default-schema".to_string(),
            expected: ExpectedPreimage::of(occupant),
            bytes: schema.content.clone().into_bytes(),
            mode: FileMode::Regular,
        });
    }

    // Obsolete generated schemas: delete a captured schemas/*.json ONLY when a
    // persisted default-origin rule proves it was generated (its reference in the
    // current rules.toml) AND it is no longer an expected target. Filename
    // convention alone never authorizes deletion (ownership matrix, schema row).
    //
    // Rule names are unique here (checked above), so name->origin is injective. A
    // schema REFERENCE, however, is not a unique key: a default rule and a custom
    // rule may both reference one schema path. Such a shared reference is not
    // exclusively default-owned, so deleting it would break the custom rule — it is
    // therefore never deleted. Deletion requires a reference owned SOLELY by
    // default-origin rules.
    let origins: std::collections::HashMap<&str, Option<&str>> = identities
        .iter()
        .map(|(name, origin)| (name.as_str(), origin.as_deref()))
        .collect();
    let mut default_refs = std::collections::BTreeSet::new();
    let mut foreign_refs = std::collections::BTreeSet::new();
    for req in
        crate::declarations::rules::RuleSet::schema_requests(&current_rules).unwrap_or_default()
    {
        if origins.get(req.rule.as_str()).copied().flatten()
            == Some(crate::declarations::rules::DEFAULT_ORIGIN)
        {
            default_refs.insert(req.reference);
        } else {
            foreign_refs.insert(req.reference);
        }
    }
    for (vpath, occupant) in image.entries() {
        let RepositoryEntry::File { .. } = occupant else {
            continue;
        };
        let Some(name) = schema_file_name(vpath) else {
            continue;
        };
        let reference = format!("schemas/{name}");
        // Proven generated (referenced ONLY by default rules), not shared with any
        // custom rule, and no longer an expected target.
        if default_refs.contains(&reference)
            && !foreign_refs.contains(&reference)
            && !expected_names.contains(name)
        {
            actions.push(RepositoryAction::DeleteFile {
                path: vpath.clone(),
                owner: "default-schema".to_string(),
                expected: ExpectedPreimage::of(occupant),
            });
        }
    }
    Ok(actions)
}

/// The first rule name that appears more than once in `identities`, in encounter
/// order, or `None` when every rule name is unique. Duplicate names make the
/// name-keyed default-rule/schema ownership non-injective and are treated as
/// ambiguous ownership.
fn first_duplicate_rule_name(identities: &[(String, Option<String>)]) -> Option<&str> {
    let mut seen = std::collections::HashSet::new();
    identities
        .iter()
        .find(|(name, _)| !seen.insert(name.as_str()))
        .map(|(name, _)| name.as_str())
}

/// Look up a captured entry, mapping an uncaptured path to the typed producer error.
fn entry<'a>(
    image: &'a RepositoryImage,
    path: &VirtualPath,
) -> Result<&'a RepositoryEntry, RepositoryStateError> {
    image
        .entry(path)
        .map_err(|e| RepositoryStateError::producer(e.into()))
}

/// The file name of a `Data("schemas/<name>")` entry, or `None` for any other
/// path. Used to locate obsolete generated schema targets for repair.
fn schema_file_name(path: &VirtualPath) -> Option<&str> {
    if path.root_class() != crate::repository_state::RepositoryRootClass::Data {
        return None;
    }
    match path.relative() {
        crate::repository_state::RootRelativePath::Descendant(rel) => rel
            .strip_prefix("schemas/")
            .filter(|name| !name.contains('/')),
        crate::repository_state::RootRelativePath::Root => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::rules::RuleSet;
    use crate::declarations::GateRegistry;
    use crate::repository_state::{
        compare_materializations, derive_materializations, CaptureBudget, CaptureSpec,
        EntryIdentity, MaterializationDriftKind, MaterializationIntent, RepositoryImage,
        RepositoryLayout, RepositoryRootEvidence, RepositorySeed, RepositorySeedKind,
    };
    use std::collections::{BTreeMap, HashMap};

    const CONFIG: &str = r#"
[project]
name = "materialize-test"

[item_kinds.invariant]
scope = "project"
source = { toml = ".jit/invariants.toml", table = "invariants", id-field = "id", text-field = "statement" }
source-of-truth = "registry-first"

[projection.invariants]
kind = "invariant"
mode = "region"
target = "AGENTS.md"
style = "id-anchor"
"#;

    const INVARIANTS: &str = r#"
[[invariants]]
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"

[[invariants]]
id = "second-invariant"
statement = "Issues prefer functional style."
kind = "advisory"
"#;

    const EXPECTED_ROWS: &str = "- **sample-invariant** — Every dependency edge stays acyclic.\n\
         - **second-invariant** — Issues prefer functional style.\n";

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    /// Build a closed image from repo-relative path → bytes (`.jit/...` is Data,
    /// everything else Worktree). A `None` value captures the path as absent.
    fn image(files: &[(&str, Option<&str>)]) -> RepositoryImage {
        let layout = layout();
        let mut data_paths = Vec::new();
        let mut worktree_paths = Vec::new();
        let mut entries = BTreeMap::new();
        for (repo_rel, contents) in files {
            let vpath = image_path(repo_rel).unwrap();
            let entry = match contents {
                Some(text) => RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes(*repo_rel, text.as_bytes()).unwrap(),
                    bytes: text.as_bytes().to_vec(),
                    mode: FileMode::Regular,
                },
                None => RepositoryEntry::Absent,
            };
            match vpath.root_class() {
                crate::repository_state::RepositoryRootClass::Data => {
                    data_paths.push(vpath.clone())
                }
                crate::repository_state::RepositoryRootClass::Worktree => {
                    worktree_paths.push(vpath.clone())
                }
            }
            entries.insert(vpath, entry);
        }
        let budget = CaptureBudget {
            max_paths: 32,
            max_listings: 0,
            max_bytes: 1 << 20,
            max_depth: 8,
        };
        // Data declaration roots enter phase one; worktree targets are phase-two
        // discovered, matching the two-phase capture contract.
        let mut spec = CaptureSpec::phase_one(data_paths, budget).unwrap();
        spec.discover_paths(worktree_paths).unwrap();
        RepositoryImage::close(
            layout,
            spec,
            entries,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn rules() -> RuleSet {
        RuleSet::default()
    }
    fn gates() -> GateRegistry {
        GateRegistry::default()
    }

    #[test]
    fn test_render_capture_closure_enumerates_registries_sources_and_target() {
        let config: JitConfig = toml::from_str(CONFIG).unwrap();
        // Selecting the single declared projection captures the four engine
        // registries, its region target, and its registry-first kind source.
        let paths = render_capture_closure(&config, &["invariants".to_string()], None).unwrap();
        let expected: std::collections::BTreeSet<VirtualPath> = [
            VirtualPath::data("config.toml").unwrap(),
            VirtualPath::data("invariants.toml").unwrap(),
            VirtualPath::data("rules.toml").unwrap(),
            VirtualPath::data("gates.toml").unwrap(),
            VirtualPath::worktree("AGENTS.md").unwrap(),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            paths.into_iter().collect::<std::collections::BTreeSet<_>>(),
            expected
        );
        // An empty selection enumerates the same closure (one projection declared).
        let all = render_capture_closure(&config, &[], None).unwrap();
        assert!(all.contains(&VirtualPath::worktree("AGENTS.md").unwrap()));
        // An unknown selected name is a defensive error.
        assert!(render_capture_closure(&config, &["nope".to_string()], None).is_err());
    }

    #[test]
    fn test_render_capture_closure_adds_referenced_rule_schemas() {
        let config: JitConfig = toml::from_str(CONFIG).unwrap();
        // A rules.toml that references a json-schema pulls that schema into the
        // closure, so the effective-rule parse reads image-projected bytes.
        let rules = "[[rules]]\nname = \"shape\"\ntype = \"format\"\n\
             assert = { json-schema = \"schemas/custom.json\" }\n";
        let paths = render_capture_closure(&config, &[], Some(rules)).unwrap();
        assert!(paths.contains(&VirtualPath::data("schemas/custom.json").unwrap()));
    }

    fn declarations<'a>(
        config: &'a crate::declarations::ConfigurationDeclarations,
        gates: &'a GateRegistry,
        rules: &'a RuleSet,
    ) -> RepositoryDeclarations<'a> {
        RepositoryDeclarations {
            configuration: config,
            gates,
            rules,
        }
    }
    fn empty_config_decls() -> crate::declarations::ConfigurationDeclarations {
        crate::declarations::ConfigurationDeclarations {
            hierarchy: None,
            namespaces: HashMap::new(),
            item_kinds: HashMap::new(),
            projections: BTreeMap::new(),
            documentation: None,
        }
    }

    fn seed() -> RepositorySeed {
        RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "materialize-test".into(),
            },
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn agents_with_region(region: &str, inner: &str) -> String {
        format!("# Doc\n\nintro\n\n<!-- jit:{region}:begin -->\n{inner}\n<!-- jit:{region}:end -->\n\ntrailing\n")
    }

    #[test]
    fn test_derive_project_render_emits_write_for_stale_region() {
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let decls = declarations(&cfg, &g, &r);
        let image = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            (
                "AGENTS.md",
                Some(&agents_with_region("invariants", "STALE")),
            ),
        ]);
        let plan = derive_materializations(
            &image,
            decls,
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        let actions = plan.delta().actions();
        assert_eq!(actions.len(), 1, "one stale target write");
        let RepositoryAction::WriteFile { path, bytes, .. } = &actions[0] else {
            panic!("expected WriteFile, got {:?}", actions[0]);
        };
        assert_eq!(path, &VirtualPath::worktree("AGENTS.md").unwrap());
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(EXPECTED_ROWS.trim_end()));
        // Byte-for-byte: prose outside the region is preserved.
        assert!(text.starts_with("# Doc\n\nintro\n\n"));
        assert!(text.ends_with("\n\ntrailing\n"));
        assert!(!text.contains("STALE"));
    }

    #[test]
    fn test_derive_project_render_fresh_region_emits_no_action() {
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let decls = declarations(&cfg, &g, &r);
        // First derive against a stale doc to obtain the exact fresh bytes.
        let stale = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            (
                "AGENTS.md",
                Some(&agents_with_region("invariants", "STALE")),
            ),
        ]);
        let plan = derive_materializations(
            &stale,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = &plan.delta().actions()[0] else {
            panic!("expected write");
        };
        let fresh_text = String::from_utf8(bytes.clone()).unwrap();
        // Re-derive against a doc that already holds the fresh bytes → no drift.
        let fresh = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            ("AGENTS.md", Some(&fresh_text)),
        ]);
        let plan2 = derive_materializations(
            &fresh,
            decls,
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        assert!(
            plan2.delta().actions().is_empty(),
            "fresh target contributes no action"
        );
    }

    #[test]
    fn test_derive_shared_target_composes_both_regions_deterministically() {
        // Two region projections into ONE target compose deterministically.
        // CONFIG already declares the `invariants` projection; append a second.
        let config = format!(
            "{CONFIG}\n[projection.extra]\nkind = \"invariant\"\nmode = \"region\"\ntarget = \"AGENTS.md\"\nstyle = \"id-anchor\"\n"
        );
        let agents = "# Doc\n\n<!-- jit:invariants:begin -->\nOLD-A\n<!-- jit:invariants:end -->\n\nmiddle\n\n<!-- jit:extra:begin -->\nOLD-B\n<!-- jit:extra:end -->\n".to_string();
        let build = || {
            image(&[
                (".jit/config.toml", Some(&config)),
                (".jit/invariants.toml", Some(INVARIANTS)),
                ("AGENTS.md", Some(&agents)),
            ])
        };
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan_a = derive_materializations(
            &build(),
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        let plan_b = derive_materializations(
            &build(),
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        // Determinism: identical delta and plan hash across runs.
        assert_eq!(plan_a.delta(), plan_b.delta());
        assert_eq!(plan_a.hash(), plan_b.hash());
        let RepositoryAction::WriteFile { bytes, .. } = &plan_a.delta().actions()[0] else {
            panic!("expected single composed write");
        };
        assert_eq!(plan_a.delta().actions().len(), 1, "one composed target");
        let text = String::from_utf8(bytes.clone()).unwrap();
        // BOTH regions were replaced with the rendered rows; neither clobbered the
        // other, and the unmanaged "middle" prose is preserved.
        assert!(!text.contains("OLD-A") && !text.contains("OLD-B"));
        assert!(text.contains("middle"));
        assert_eq!(text.matches("sample-invariant").count(), 2);
    }

    #[test]
    fn test_derive_then_compare_reports_stale_drift() {
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let stale = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            (
                "AGENTS.md",
                Some(&agents_with_region("invariants", "STALE")),
            ),
        ]);
        let plan = derive_materializations(
            &stale,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RenderConfiguredProjections,
        )
        .unwrap();
        // Comparing the expected plan against the same stale image reports the
        // target as stale derived state.
        let drift = compare_materializations(&stale, &plan).unwrap();
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].kind, MaterializationDriftKind::Stale);
        assert_eq!(drift[0].path, VirtualPath::worktree("AGENTS.md").unwrap());
    }

    #[test]
    fn test_repair_preserves_authored_bytes_outside_the_region() {
        // Repair of a region projection splices ONLY the managed region; every
        // authored byte outside the markers is preserved unconditionally (ownership
        // matrix: configured region projection).
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let authored_prefix =
            "# Hand-authored heading\n\nAuthored intro the tool must never touch.\n\n";
        let authored_suffix = "\n\n## Authored trailing section\n\nMore authored prose.\n";
        let agents = format!(
            "{authored_prefix}<!-- jit:invariants:begin -->\nMANUALLY EDITED DERIVED CONTENT\n<!-- jit:invariants:end -->{authored_suffix}"
        );
        let image = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            ("AGENTS.md", Some(&agents)),
        ]);
        let plan = derive_materializations(
            &image,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        )
        .unwrap();
        let RepositoryAction::WriteFile { bytes, .. } = &plan.delta().actions()[0] else {
            panic!("expected a repair write");
        };
        let repaired = String::from_utf8(bytes.clone()).unwrap();
        // Authored prefix and suffix are byte-for-byte preserved; only the region
        // interior was replaced with the derived rows.
        assert!(repaired.starts_with(authored_prefix));
        assert!(repaired.ends_with(authored_suffix));
        assert!(!repaired.contains("MANUALLY EDITED"));
        assert!(repaired.contains(EXPECTED_ROWS.trim_end()));
    }

    fn ns_config(namespaces: &[&str]) -> String {
        let mut out = String::from("[project]\nname = \"ruleset-test\"\n");
        for ns in namespaces {
            out.push_str(&format!(
                "[namespaces.{ns}]\ndescription = \"{ns}\"\nunique = true\n"
            ));
        }
        out
    }

    /// A coherent scaffolded ruleset (config and rules.toml agree) derives no
    /// rules/schema action.
    #[test]
    fn test_derive_default_ruleset_coherent_scaffold_emits_no_action() {
        let config = ns_config(&["component", "team"]);
        let jc: JitConfig = toml::from_str(&config).unwrap();
        let ns = crate::config_manager::namespaces_from_config(&jc);
        let scaffold = serialize_ruleset(&default_ruleset(&ns));

        let schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config)),
            (".jit/rules.toml", Some(&scaffold.rules_toml)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::SemanticMutation,
        )
        .unwrap();
        assert!(
            plan.delta().actions().is_empty(),
            "a coherent scaffold has no drift: {:?}",
            plan.delta().actions()
        );
    }

    /// A newly-declared unique namespace drifts the on-disk ruleset: derive adds
    /// exactly the missing `namespace-unique-*` membership row (preserving authored
    /// custom rows and comments) and rewrites the namespace-registry schema.
    #[test]
    fn test_derive_default_ruleset_adds_membership_and_schema_preserving_authored() {
        let config_full = ns_config(&["component", "team", "squad"]);
        let config_partial = ns_config(&["component", "team"]);
        let jc_partial: JitConfig = toml::from_str(&config_partial).unwrap();
        let ns_partial = crate::config_manager::namespaces_from_config(&jc_partial);
        let scaffold = serialize_ruleset(&default_ruleset(&ns_partial));
        // Authored content the repair must preserve unconditionally.
        let authored_rules = format!(
            "{}\n[[rules]]\nname = \"custom-shape\"\n# hand-authored comment\nseverity = \"warn\"\nassert = {{ require-section = {{ heading = \"Goals\" }} }}\n",
            scaffold.rules_toml
        );
        let schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config_full)),
            (".jit/rules.toml", Some(&authored_rules)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::SemanticMutation,
        )
        .unwrap();

        let rules_write = plan
            .delta()
            .actions()
            .iter()
            .find_map(|a| match a {
                RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &VirtualPath::data("rules.toml").unwrap() =>
                {
                    Some(String::from_utf8(bytes.clone()).unwrap())
                }
                _ => None,
            })
            .expect("rules.toml membership add");
        assert!(
            rules_write.contains("namespace-unique-squad"),
            "squad added"
        );
        assert!(
            rules_write.contains("name = \"custom-shape\"")
                && rules_write.contains("# hand-authored comment"),
            "authored custom rule and comment preserved:\n{rules_write}"
        );
        // The namespace-registry schema (its enum of registered namespaces) drifts
        // with the new namespace, so its target is rewritten too.
        assert!(
            plan.delta().actions().iter().any(|a| matches!(
                a,
                RepositoryAction::WriteFile { path, .. }
                    if path == &VirtualPath::data("schemas/default-namespace-registry.json").unwrap()
            )),
            "namespace-registry schema rewritten: {:?}",
            plan.delta().actions()
        );
    }

    #[test]
    fn test_derive_default_schema_delete_requires_proven_ownership_not_filename() {
        // A captured schemas/default-orphan.json referenced by NO rule is NOT
        // deleted: filename convention alone never authorizes deletion.
        let config = ns_config(&["component", "team"]);
        let jc: JitConfig = toml::from_str(&config).unwrap();
        let ns = crate::config_manager::namespaces_from_config(&jc);
        let scaffold = serialize_ruleset(&default_ruleset(&ns));
        let mut schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        // An unreferenced, jit-named-looking stale schema.
        schema_paths.push((
            ".jit/schemas/default-orphan.json".to_string(),
            "{}".to_string(),
        ));
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config)),
            (".jit/rules.toml", Some(&scaffold.rules_toml)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        )
        .unwrap();
        assert!(
            !plan
                .delta()
                .actions()
                .iter()
                .any(|a| matches!(a, RepositoryAction::DeleteFile { .. })),
            "an unreferenced schema is not deleted on filename alone: {:?}",
            plan.delta().actions()
        );
    }

    #[test]
    fn test_derive_default_schema_deletes_obsolete_target_under_proven_ownership() {
        // A default-origin rule proves generation of schemas/default-obsolete.json,
        // which is no longer an expected target: repair deletes exactly it.
        let config = ns_config(&["component", "team"]);
        let jc: JitConfig = toml::from_str(&config).unwrap();
        let ns = crate::config_manager::namespaces_from_config(&jc);
        let scaffold = serialize_ruleset(&default_ruleset(&ns));
        let rules_toml = format!(
            "{}\n[[rules]]\nname = \"obsolete-default\"\norigin = \"default\"\nassert = {{ json-schema = \"schemas/default-obsolete.json\" }}\n",
            scaffold.rules_toml
        );
        let mut schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        schema_paths.push((
            ".jit/schemas/default-obsolete.json".to_string(),
            "{}".to_string(),
        ));
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config)),
            (".jit/rules.toml", Some(&rules_toml)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        )
        .unwrap();
        let deletes: Vec<&VirtualPath> = plan
            .delta()
            .actions()
            .iter()
            .filter_map(|a| match a {
                RepositoryAction::DeleteFile { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(
            deletes,
            vec![&VirtualPath::data("schemas/default-obsolete.json").unwrap()],
            "exactly the proven-obsolete schema is deleted"
        );
    }

    /// Build an image whose rules.toml has a custom `namespace-registry` rule
    /// (referencing schemas/custom-namespace.json) either before or after the
    /// scaffolded default `namespace-registry` rule, then repair over it. Both a
    /// custom-before-default and default-before-custom collision must be a typed
    /// non-repairable ambiguity — never a DeleteFile of the custom schema.
    fn assert_duplicate_name_is_non_repairable(custom_first: bool) {
        let config = ns_config(&["component", "team"]);
        let jc: JitConfig = toml::from_str(&config).unwrap();
        let ns = crate::config_manager::namespaces_from_config(&jc);
        let scaffold = serialize_ruleset(&default_ruleset(&ns));
        let custom = "[[rules]]\nname = \"namespace-registry\"\nseverity = \"warn\"\nassert = { json-schema = \"schemas/custom-namespace.json\" }\n";
        let rules_toml = if custom_first {
            format!("{custom}\n{}", scaffold.rules_toml)
        } else {
            format!("{}\n{custom}", scaffold.rules_toml)
        };
        let mut schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        // The authored custom schema the buggy classifier would have deleted.
        schema_paths.push((
            ".jit/schemas/custom-namespace.json".to_string(),
            "{\"type\":\"object\"}".to_string(),
        ));
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config)),
            (".jit/rules.toml", Some(&rules_toml)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let result = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        );
        let err = result.expect_err("duplicate rule names must be non-repairable");
        assert!(
            matches!(err, RepositoryStateError::AmbiguousOwnership(_)),
            "expected a typed ambiguity error (custom_first={custom_first}), got {err:?}"
        );
    }

    #[test]
    fn test_obsolete_schema_shared_with_custom_rule_is_not_deleted() {
        // schemas/default-shared.json is referenced by BOTH a default-origin rule
        // and a custom rule. Even though a default reference "proves generation",
        // the shared reference is not exclusively default-owned, so deleting it
        // would break the custom rule — it must be preserved.
        let config = ns_config(&["component", "team"]);
        let jc: JitConfig = toml::from_str(&config).unwrap();
        let ns = crate::config_manager::namespaces_from_config(&jc);
        let scaffold = serialize_ruleset(&default_ruleset(&ns));
        let rules_toml = format!(
            "{}\n[[rules]]\nname = \"obsolete-default\"\norigin = \"default\"\nassert = {{ json-schema = \"schemas/default-shared.json\" }}\n\
             [[rules]]\nname = \"custom-consumer\"\nseverity = \"warn\"\nassert = {{ json-schema = \"schemas/default-shared.json\" }}\n",
            scaffold.rules_toml
        );
        let mut schema_paths: Vec<(String, String)> = scaffold
            .schema_files
            .iter()
            .map(|f| (format!(".jit/schemas/{}", f.name), f.content.clone()))
            .collect();
        schema_paths.push((
            ".jit/schemas/default-shared.json".to_string(),
            "{}".to_string(),
        ));
        let mut files: Vec<(&str, Option<&str>)> = vec![
            (".jit/config.toml", Some(&config)),
            (".jit/rules.toml", Some(&rules_toml)),
        ];
        for (p, c) in &schema_paths {
            files.push((p, Some(c)));
        }
        let img = image(&files);
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let plan = derive_materializations(
            &img,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        )
        .unwrap();
        assert!(
            !plan
                .delta()
                .actions()
                .iter()
                .any(|a| matches!(a, RepositoryAction::DeleteFile { .. })),
            "a schema shared with a custom rule is not deleted: {:?}",
            plan.delta().actions()
        );
    }

    #[test]
    fn test_duplicate_rule_name_custom_before_default_is_non_repairable() {
        assert_duplicate_name_is_non_repairable(true);
    }

    #[test]
    fn test_duplicate_rule_name_default_before_custom_is_non_repairable() {
        assert_duplicate_name_is_non_repairable(false);
    }

    #[test]
    fn test_repair_ambiguous_markers_are_non_repairable_not_whole_file_rewrite() {
        // A region-projection target carrying DUPLICATE begin markers has ambiguous
        // ownership: repair fails before publication rather than serializing a
        // whole-file fallback (ownership matrix: ambiguous ownership is non-repairable).
        let cfg = empty_config_decls();
        let (g, r) = (gates(), rules());
        let ambiguous = "# Doc\n\n<!-- jit:invariants:begin -->\nA\n<!-- jit:invariants:begin -->\nB\n<!-- jit:invariants:end -->\n";
        let image = image(&[
            (".jit/config.toml", Some(CONFIG)),
            (".jit/invariants.toml", Some(INVARIANTS)),
            ("AGENTS.md", Some(ambiguous)),
        ]);
        let result = derive_materializations(
            &image,
            declarations(&cfg, &g, &r),
            &seed(),
            MaterializationIntent::RepairDerivedState,
        );
        let err = result.expect_err("ambiguous markers must be non-repairable");
        assert!(
            matches!(err, RepositoryStateError::Producer(_)),
            "expected a typed non-repairable producer error, got {err:?}"
        );
    }
}
