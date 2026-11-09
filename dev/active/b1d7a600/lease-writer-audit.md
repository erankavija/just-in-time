# Lease audit of `with_mutation_session` callers

Source: `rg -n 'with_mutation_session' crates/jit/src`, test-only callers omitted. "Plan entry" is `lease-checked-writes` in `dev/active/b697a3f7-one-mutation-path/plan.md`.

| Caller | Writes an existing issue record | Check or exemption |
|---|---|---|
| `document.rs` document add | yes | `refuse_foreign_leases_held` in session |
| `document.rs` document removal | yes | `refuse_foreign_leases_held` in session |
| `document.rs` document rescan (`doc assets list --rescan`) | yes | `refuse_foreign_leases_held` in session, added by b1d7a600; `AssetCommands::List { rescan }` is claims-first |
| `mod.rs` manual gate status | yes | `refuse_foreign_leases_held` in session |
| `gate.rs` gate registry mutation | yes, gate removal and update touch no issue record; the lease check covers the issues requiring the gate | `refuse_foreign_leases_held` over those issues in session |
| `mod.rs` issue creation | no, new record | none needed |
| `batch_create.rs` batch issue creation | no, new records | none needed |
| `claim.rs` claim repository synchronization | yes, assignment plus `IssueClaimed` | runs inside `acquire_claim_synchronized` after the coordinator granted the lease to this agent; acquisition refuses another holder, so the write is under the writer's own lease. Not a plan-entry exemption (see report) |
| `archive.rs` archive execution | yes | plan entry: archive |
| `validate.rs` validation repair (`validate --fix`) | yes | plan entry: `validate --fix` |
| `validate.rs` derived-state repair (`recover`) | no issue decode, declaration-derived targets | plan entry: recovery |
| `gate.rs` gate preset creation | no, writes a preset file; source issue is read | none needed |
| `config.rs` config set | no | none needed |
| `project.rs` project render | no, projections | none needed |
| `profile.rs` profile package tree and profile run | no, declarations and package files | none needed |
| `init.rs` init | no, creates the data root | none needed |
| `mod.rs` repository export, `snapshot.rs` snapshot export | no | none needed |

Other issue writers do not call `with_mutation_session` directly and check in their own sessions: issue update and lifecycle (`issue.rs`, `mod.rs`), bulk update, template apply, `dep add`/`dep rm`, gate evaluation (`gate_check.rs`), gate add/remove/pass/fail (`gate.rs`).
