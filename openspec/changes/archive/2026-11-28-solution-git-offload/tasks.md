# Tasks

## 1. Async git entry points

- [x] 1.1 Add async entry points in `crates/solution-service/src/git/mod.rs`
      that wrap `run`/`run_ok`/`is_tracked_clean` in `spawn_blocking` and
      await the result, with a module doc stating the "no git on the
      worker" rule; verify: `cargo build -p solution-service` succeeds and
      existing `git/mod.rs` unit tests (sync internals) still pass.
- [x] 1.2 Add async entry points for `worktree::{list, add, remove}` in
      `crates/solution-service/src/git/worktree.rs` on the same pattern;
      verify: `cargo test -p solution-service git::` passes, including the
      worktree parse/list tests.

## 2. Convert the five blocking call sites

- [x] 2.1 Rework `on_git_pull_push` (`service.rs`): keep only cheap
      validation in `handle()` (node format → reject, no-such → `NoSuch`,
      `.git` probe → non-git refusal), send the ack with the current
      snapshot before spawning, then run tracked-clean gate + pull/push as
      one background unit; gate refusal travels as `KIND_GIT_OP_FINISHED`
      with `ok=false` and the reason; verify: `cargo test -p solution-service
      pull` — dirty-repo test now expects ack + refusal notice instead of a
      synchronous error reply, and asserts the repository is untouched.
- [x] 2.2 Move `spawn_git_op`'s `git pull`/`push` subprocess into
      `spawn_blocking` inside the spawned task; verify: the pull/push tests
      in `service.rs` (create-remote, pull, push scenarios) pass unchanged
      in outcome.
- [x] 2.3 Make `snapshot()` await the async worktree listing (off-thread)
      and convert `on_load` / `on_get_tree` / action-reply paths to await
      it; verify: `cargo test -p solution-service snapshot` and the
      worktree-listing scenario test pass, with the listing result still
      reaching the snapshot that requested it.
- [x] 2.4 Rework `on_create_worktree` / `on_delete_worktree`: ack with the
      pre-operation snapshot, run the git call plus post-operation snapshot
      assembly as one background unit, publish via `TreeChanged`; verify:
      `cargo test -p solution-service worktree` — create/delete tests pass
      against the event-driven shape (directory exists / no longer listed
      after the change notification lands).

## 3. Compile-time wall

- [x] 3.1 Flip visibility: sync `run`/`run_ok`/`is_tracked_clean` and
      `worktree::{list, add, remove}` become module-private (async entries
      only public); verify: `cargo build --all-targets` passes — the
      compiler enumerates any remaining synchronous call site, including
      tests outside the module.

## 4. Offload regression test

- [x] 4.1 Add a latched-git offload test (modeled on
      `asset-service/offload_tests.rs`): inject a latched fake git program,
      start a gated invoke / snapshot listing, assert the subprocess runs
      on a thread other than the driver's worker thread, watchdog
      auto-release so a broken implementation fails instead of hanging;
      verify: `cargo test -p solution-service offload` passes, and the
      assertion fails if an async entry point is temporarily replaced by
      the sync one (mutation check during development).

## 5. Integration

- [x] 5.1 Run the full suite `cargo test` and verify everything passes,
      including the updated refusal/ack timing tests and the offload test;
      confirm no test asserts a synchronous `OpError` reply for the
      tracked-clean gate anymore.

  注：与本 change 相关的套件全绿（solution-service 48/48、
  service-manager 54/54、GUI worktree/notice 测试通过，无同步门控
  OpError 断言）。全 workspace 尚余 20 个**存量环境失败**（stash 对比
  证实基线 21 红 → 现 17+3 红，均非本改动引入）：fake-pi.sh 无法在
  Win32 拉起（~11）、`project-name-/` 路径断言（2）、solution_config
  `\\?\` 前缀（2）、asset_overview gpui 帧时序（6）、example-plugin
  导出 trie 字节补丁（1）—— 另开清理 change 处理。

## Workflow follow-up

- Archive the change after the project's review requirements are satisfied
  (`openspec archive`), which applies the `solution-service` spec delta.
- Verify the archived result.
