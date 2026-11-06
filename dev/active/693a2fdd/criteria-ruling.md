# Criteria ruling: bulk Gated diversion reporting

**Issue:** 693a2fdd. **Decided by:** the steward session, on escalation from the b697a3f7 execution lead.

## Conflict

REQ-03 says a `Done` request diverted to `Gated` is reported as "modified and failed, as today". At the pre-change revision (`2252c76fd^`) bulk update reported a diversion in `errors` only, so the parenthetical and "as today" disagree. REQ-01 requires the remaining bulk-update tests to pass unchanged. The escalation assumed `test_apply_bulk_update_best_effort` asserted errors-only reporting for a diversion; it does not, because its failing issue requires a gate missing from the registry, which is a plain failure rather than a diversion.

The configured code review (run `3fe7575a-cef3-463d-a1d7-0193f31a7ae7`, finding F1) rejected errors-only reporting under REQ-03.

## Ruling

REQ-03's explicit "(modified and failed)" governs: the Gated change is published, so the diverted issue is reported in both `modified` and `errors`, with the single-issue update's typed error. Amending REQ-03 to keep errors-only reporting was declined.

## Outcome

The rework (`aef66178f`) needed no change to `test_apply_bulk_update_best_effort`, which passes unchanged under the new contract, so REQ-01 holds as written. The owning assertion that a diverted issue appears in both sets is in `test_apply_bulk_update_done_with_unpassed_gate_diverts_like_single_update`.
