## What and why / 改动内容与原因

<!-- One topic per pull request. Link the issue if there is one. / 每个 PR 只做一件事；如有 issue 请关联。 -->

## How it was checked / 验证方法

<!-- Commands you ran, and the platform. GUI checks: say which scenario and attach the screenshots if relevant. -->

## Checklist / 检查清单

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo test --workspace --locked` passes.
- [ ] Behaviour changes have tests (unit tests for pure logic).
- [ ] User documentation (README, `docs/`) is updated if behaviour changed.
- [ ] No secrets, personal paths, private hostnames or real notes are included.
- [ ] `Cargo.lock` is updated in this change if a dependency changed.
