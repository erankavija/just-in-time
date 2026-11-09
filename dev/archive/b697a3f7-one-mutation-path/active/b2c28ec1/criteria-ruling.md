# Criteria ruling: REQ-06 rustdoc clause

**Issue:** b2c28ec1. **Decided by:** the steward session, on escalation from the b697a3f7 execution lead.

## Conflict

REQ-06 required the `validation/graph.rs` rustdoc to name `RootRelativePath::parse`. That clause pointed at a slash-splitter audit list in `graph.rs`'s module rustdoc, which named `validate_repo_relative_path`. Commit `deb6d3beb` (6ac41880, executor-side validation cleanup) deleted that list before this issue was implemented, so `graph.rs` holds no path-validation prose. The configured code review (run `e3ff064f-dd20-4877-aea5-f7034b157762`, finding F1) failed the clause as written.

## Ruling

REQ-06 is corrected, not dropped, so the protected property stays literally checkable: the `read_repo_file` contract rustdoc names `RootRelativePath::parse` as the one parser, and no rustdoc or comment in `crates/` names any other repository-relative path parser or splitter. No source change follows from the correction. Adding unrelated prose to `graph.rs` and ruling the clause vacuously satisfied were both declined.

If the review finds a remaining mention of another parser or splitter, a prose-only fix is authorized as a third rework round for that purpose alone.
