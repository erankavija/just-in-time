use super::{HumanRender, OutputMode};
use crate::domain::identity::ProfileId;
use crate::profile::{
    ProfileAddResult, ProfileApplicationStatus, ProfileComposedApplyResult,
    ProfileContributionChange, ProfilePackResult, ProfileShowResult, ProfileTargetAction,
    ProfileTargetChange,
};
use crate::repository_state::RootRelativePath;

fn heading(
    id: &ProfileId,
    version: &semver::Version,
    origin: &RootRelativePath,
    status: &str,
) -> String {
    format!(
        "Profile {id} {version} (directory {}): {status}\n",
        origin.as_path().display()
    )
}

fn status_label(status: ProfileApplicationStatus, verb: &str, past: &str) -> String {
    match status {
        ProfileApplicationStatus::Unchanged => "unchanged".to_owned(),
        ProfileApplicationStatus::WouldApply => format!("would {verb}"),
        ProfileApplicationStatus::WouldConflict => "would conflict".to_owned(),
        ProfileApplicationStatus::Applied => past.to_owned(),
    }
}

fn decision(action: ProfileTargetAction, subject: &str, reason: Option<&str>) -> String {
    let action = match action {
        ProfileTargetAction::Unchanged => "unchanged",
        ProfileTargetAction::Create => "create",
        ProfileTargetAction::Update => "update",
        ProfileTargetAction::Remove => "remove",
        ProfileTargetAction::Conflict => "conflict",
    };
    format!(
        "  {action}: {subject}\n{}",
        reason.map_or(String::new(), |reason| format!("      {reason}\n"))
    )
}

fn decisions(
    targets: &[ProfileTargetChange],
    contributions: &[ProfileContributionChange],
) -> String {
    targets
        .iter()
        .map(|target| decision(target.action, &target.path, target.reason.as_deref()))
        .chain(
            contributions
                .iter()
                .map(|entry| decision(entry.action, &entry.identity, entry.reason.as_deref())),
        )
        .collect()
}

impl HumanRender for ProfileShowResult {
    fn render_human(&self, _: OutputMode) -> String {
        self.items
            .iter()
            .map(|entry| {
                format!(
                    "{}Compatible JIT: {}\nPackage hash: {}\nFiles: {} ({} bytes)\nTargets:\n{}",
                    heading(&entry.id, &entry.version, &entry.origin, "inspected"),
                    entry.manifest.compatible_jit,
                    entry.package_hash,
                    entry.file_count,
                    entry.byte_size,
                    entry
                        .targets
                        .iter()
                        .map(|target| format!("  {target}\n"))
                        .collect::<String>()
                )
            })
            .collect()
    }
}

impl HumanRender for ProfileComposedApplyResult {
    fn render_human(&self, _: OutputMode) -> String {
        self.items
            .iter()
            .map(|entry| {
                format!(
                    "{}{}{}",
                    heading(
                        &entry.id,
                        &entry.version,
                        &entry.origin,
                        &status_label(entry.status, "apply", "applied")
                    ),
                    decisions(&entry.targets, &entry.contributions),
                    entry
                        .refusal
                        .as_ref()
                        .map_or(String::new(), |refusal| format!("  refused: {refusal}\n"))
                )
            })
            .collect()
    }
}

impl HumanRender for ProfilePackResult {
    fn render_human(&self, _: OutputMode) -> String {
        self.items
            .iter()
            .map(|entry| {
                format!(
                    "{}Archive: {}\nPackage hash: {}\nFiles: {} ({} bytes); archive {} bytes\n{}",
                    heading(
                        &entry.id,
                        &entry.version,
                        &entry.origin,
                        &status_label(entry.status, "pack", "packed")
                    ),
                    entry.archive,
                    entry.package_hash,
                    entry.file_count,
                    entry.byte_size,
                    entry.archive_bytes,
                    decisions(&entry.targets, &[])
                )
            })
            .collect()
    }
}

impl HumanRender for ProfileAddResult {
    fn render_human(&self, _: OutputMode) -> String {
        self.items
            .iter()
            .map(|entry| {
                format!(
                    "{}Archive: {}\nDestination: {}\nPackage hash: {}\nFiles: {} ({} bytes)\n{}",
                    heading(
                        &entry.id,
                        &entry.version,
                        &entry.origin,
                        &status_label(entry.status, "add", "added")
                    ),
                    entry.archive,
                    entry.destination,
                    entry.package_hash,
                    entry.file_count,
                    entry.byte_size,
                    decisions(&entry.targets, &[])
                )
            })
            .collect()
    }
}
