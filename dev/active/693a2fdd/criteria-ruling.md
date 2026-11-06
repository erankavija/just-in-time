# Criteria ruling: bulk Gated diversion reporting

**Issue:** 693a2fdd. **Decided by:** the steward session, on escalation from the b697a3f7 execution lead.

## Conflict

REQ-03 says a `Done` request diverted to `Gated` is reported as "modified and failed, as today". At the pre-change revision (`2252c76fd^`) bulk update reported a diversion in `errors` only, so the parenthetical and "as today" disagree. REQ-01 requires the remaining bulk-update tests to pass unchanged, and `test_apply_bulk_update_best_effort` asserted the errors-only form.

The configured code review (run `3fe7575a-cef3-463d-a1d7-0193f31a7ae7`, finding F1) rejected errors-only reporting under REQ-03.

## Ruling

REQ-03's explicit "(modified and failed)" governs: the Gated change is published, so the diverted issue is reported in both `modified` and `errors`, with the single-issue update's typed error. REQ-01's "pass unchanged" yields for the modified-count and modified-membership assertions of `test_apply_bulk_update_best_effort` only; no other test changes under this ruling. Amending REQ-03 to keep errors-only reporting was declined.
