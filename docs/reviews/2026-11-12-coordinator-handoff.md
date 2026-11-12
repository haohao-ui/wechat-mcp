# Coordinator handoff review

Baseline: `ad3bef3`. The worker settled its task and reported no running background tasks; the worktree was clean before coordinator edits.

The coordinator reproduced a remaining startup defect: a legacy journal marked `Committed` was ignored without evidence that its installation pointer and authoritative record agreed. `test_legacy_committed_label_without_installation_proof_blocks` failed with exit 101 before the fix.

Both Rust and Swift now block on any legacy journal until explicit recovery or migration can establish that evidence. Detection remains read-only and preserves the old files. This may require administrator recovery for an otherwise valid old installation; it does not silently migrate or delete its state.

Validation after the change:

- `cargo test --locked --test trusted_paths_test`: 7 passed, exit 0.
- `cargo test --locked --test swift_host test_swift_core_supervisor_dual_check_guard_and_uuid_contract`: 1 passed, 75 filtered, exit 0; includes a legacy Committed-label regression and preservation assertion.
- `cargo fmt --all` completed.

Overall delivery remains partial. Production drain, worker exit proof, update execution/recovery integration, and independently authorized real-machine validation remain outstanding. This review is not production acceptance.
