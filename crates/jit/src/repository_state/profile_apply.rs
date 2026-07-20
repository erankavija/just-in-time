//! ApplyProfile materialization: compose a profile package's canonical claims
//! (declaration-overlay registry edits, exact assets, and managed regions) plus the
//! configured projections those declarations imply into the exact set of
//! profile-owned targets.
//!
//! `repository_state` owns this composition; the profile package produces the
//! neutral [`ProfileClaims`] and the command captures the base image and applies the
//! resulting delta. This module imports no profile, storage, or command code — the
//! profile-package parsing (`profile::package`) and the delta finalization
//! ([`finalize_profile_application`](super::finalize_profile_application)) sit on
//! either side of it.

use std::collections::BTreeMap;

use crate::config::JitConfig;
use crate::config_manager::namespaces_from_config;
use crate::declarations::rules::RuleSet;
use crate::declarations::{
    parse_configuration, ConfigurationDeclarations, GateDefinition, GateRegistry,
};

use super::materialize::compose_configured_projections;
use super::{
    apply_overlay, assemble_config, compose_managed_documents, default_ruleset,
    reconcile_default_rules_with_config, FileMode, ManagedDocumentClaim, RepositoryAction,
    RepositoryDeclarations, RepositoryEntry, RepositoryImage, RepositoryStateError, VirtualPath,
};

/// A profile package's contribution to a repository, in canonical repository-state
/// vocabulary. Produced by `profile::package` from the immutable manifest and
/// consumed only by [`derive_profile_materializations`].
pub struct ProfileClaims {
    /// Declaration-overlay edits: the merged registry files
    /// (`config.toml`/`gates.toml`/`rules.toml`/`templates.toml`) keyed by canonical
    /// path, each with its final file mode. Absent from the map when the profile
    /// declares no contribution against that registry.
    pub registries: BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
    /// Exact one-to-one asset files keyed by canonical path.
    pub assets: BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
    /// Profile-owned managed regions composed over the captured base image.
    pub regions: Vec<(VirtualPath, ManagedDocumentClaim)>,
}

/// Derive every profile-owned target's exact final bytes and mode from a captured
/// base image.
///
/// The declaration-overlay registries and exact assets land verbatim; the managed
/// regions compose over the captured base through the one managed-document engine;
/// and every configured projection the merged declarations imply is re-rendered
/// over the resulting proposed image and folded back into the target it already
/// produced. A projection whose target is not itself a profile-owned
/// asset/region/registry leaves that target untouched (matching the
/// profile-application contract: a projection only rewrites bytes the profile
/// otherwise publishes).
///
/// The result equals the byte-for-byte final image of every profile-owned target
/// regardless of whether it changed from the captured occupant; the caller
/// (`finalize_profile_application`) decides which targets to write.
pub fn derive_profile_materializations(
    base: &RepositoryImage,
    claims: ProfileClaims,
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, RepositoryStateError> {
    let mut targets: BTreeMap<VirtualPath, (Vec<u8>, FileMode)> = BTreeMap::new();
    for (path, entry) in claims.registries {
        targets.insert(path, entry);
    }
    for (path, entry) in claims.assets {
        targets.insert(path, entry);
    }
    // Profile-owned regions compose over the captured base; a region target keeps
    // its captured file mode (a fresh target is Regular), matching the profile's
    // region-target mode contract.
    for (path, bytes) in compose_managed_documents(base, claims.regions)? {
        let mode = existing_file_mode(base, &path)?;
        targets.insert(path, (bytes, mode));
    }

    // Build the proposed image (base overlaid with every target computed so far) and
    // re-render every configured projection from the PROPOSED declarations. The
    // occupant `compose_configured_projections` compares against IS the target
    // computed above, so it yields the rendered bytes whether or not it emits an
    // action: an emitted write updates the target to the rendered bytes, and a
    // no-op means the target already holds them. Either way the target ends at the
    // exact rendered projection bytes.
    let overlay = targets
        .iter()
        .map(|(path, (bytes, _))| (path.clone(), Some(bytes.clone())));
    let proposed = apply_overlay(base, overlay)
        .map_err(|error| RepositoryStateError::producer(error.into()))?;
    let config = assemble_config(&proposed).map_err(RepositoryStateError::producer)?;
    let configuration =
        proposed_configuration(&proposed).map_err(RepositoryStateError::producer)?;
    let rules = proposed_rules(&proposed, &config).map_err(RepositoryStateError::producer)?;
    let gates = proposed_gates(&proposed).map_err(RepositoryStateError::producer)?;
    let declarations = RepositoryDeclarations {
        configuration: &configuration,
        rules: &rules,
        gates: &gates,
    };
    let projection_actions =
        compose_configured_projections(&proposed, &config, &declarations, None)
            .map_err(RepositoryStateError::projection_producer)?;
    for action in projection_actions {
        if let RepositoryAction::WriteFile { path, bytes, .. } = action {
            if let Some(target) = targets.get_mut(&path) {
                target.0 = bytes;
            }
        }
    }
    Ok(targets)
}

/// The captured file mode at `path`, or `Regular` for an absent or non-file entry.
fn existing_file_mode(
    base: &RepositoryImage,
    path: &VirtualPath,
) -> Result<FileMode, RepositoryStateError> {
    match base
        .entry(path)
        .map_err(|error| RepositoryStateError::producer(error.into()))?
    {
        RepositoryEntry::File { mode, .. } => Ok(*mode),
        _ => Ok(FileMode::Regular),
    }
}

/// Read a repo-relative path's UTF-8 bytes from a proposed image (`.jit/...` is a
/// `Data` entry, every other repo-relative path a `Worktree` entry).
fn image_bytes(image: &RepositoryImage, repo_rel: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let vpath = match repo_rel.strip_prefix(".jit/") {
        Some(rest) => VirtualPath::data(rest),
        None => VirtualPath::worktree(repo_rel),
    }?;
    Ok(image.file_bytes(&vpath)?.map(<[u8]>::to_vec))
}

/// Parse the proposed configuration declarations from the proposed image.
fn proposed_configuration(image: &RepositoryImage) -> anyhow::Result<ConfigurationDeclarations> {
    let bytes = image_bytes(image, ".jit/config.toml")?
        .ok_or_else(|| anyhow::anyhow!("proposed image has no .jit/config.toml"))?;
    Ok(parse_configuration(&bytes)?)
}

/// Parse the proposed EFFECTIVE rule set from the proposed image, reconciling the
/// default family against the proposed configuration — the exact derivation the
/// whole-repository validation and `jit project render` paths use.
fn proposed_rules(image: &RepositoryImage, config: &JitConfig) -> anyhow::Result<RuleSet> {
    let namespaces = namespaces_from_config(config);
    let Some(bytes) = image_bytes(image, ".jit/rules.toml")? else {
        return Ok(default_ruleset(&namespaces));
    };
    let content = String::from_utf8(bytes)?;
    let schemas = RuleSet::schema_requests(&content)?
        .into_iter()
        .filter_map(|request| {
            let vpath = VirtualPath::data(&request.reference).ok()?;
            let bytes = image.file_bytes(&vpath).ok().flatten()?.to_vec();
            Some((request.reference, bytes))
        })
        .collect::<Vec<_>>();
    let parsed = RuleSet::parse(&content, Some(config), schemas)?;
    Ok(reconcile_default_rules_with_config(parsed, &namespaces))
}

/// Parse the proposed gate registry from the proposed image.
fn proposed_gates(image: &RepositoryImage) -> anyhow::Result<GateRegistry> {
    #[derive(serde::Deserialize)]
    struct GatesFile {
        #[serde(default)]
        gates: Vec<GateDefinition>,
    }
    let content = image_bytes(image, ".jit/gates.toml")?
        .map(String::from_utf8)
        .transpose()?
        .unwrap_or_default();
    let file: GatesFile = toml::from_str(&content)?;
    let mut gates = std::collections::HashMap::new();
    for gate in file.gates {
        let key = gate.key.clone();
        if gates.insert(key.clone(), gate).is_some() {
            anyhow::bail!("duplicate gate key '{key}' in .jit/gates.toml");
        }
    }
    Ok(GateRegistry { gates })
}
