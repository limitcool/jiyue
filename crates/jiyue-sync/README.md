# jiyue-sync 远端存储

基于 opendal 的 `RemoteStore`，支持 S3（优先）和 WebDAV（次要）。
- S3：填写 endpoint（AWS，或兼容服务如 RustFS / MinIO，如 `http://127.0.0.1:9000`）、region、bucket、access key、secret；`root_prefix` 设为 `vaults/my-notes/` 之类；兼容服务通常需要 `force_path_style = true`。
- S3 版本历史需要在桶上开启版本控制；未开启时 `supports_versions()` 为 false，且不会修改桶配置。
- WebDAV：endpoint 填 WebDAV 根地址，加上用户名、密码和 `root` 子目录；不提供版本历史，条件写入也不保证。
- 凭据只在使用时从 `credentials.toml`（或 `env:`、`cmd:` 形式）读取，`Debug` 和错误信息都会脱敏；密钥不会写入笔记库或同步。
- 单元测试（内存后端，无需网络）：`cargo test -p jiyue-sync store`。
- S3 集成测试（需要版本控制桶）：设置 `JIYUE_TEST_S3_ENDPOINT`、`_BUCKET`、`_KEY`、`_SECRET`、`_REGION`，运行 `cargo test -p jiyue-sync store -- --ignored s3_`。
- WebDAV 集成测试：设置 `JIYUE_TEST_DAV_ENDPOINT`、`_USER`、`_PASSWORD`，运行 `... -- --ignored webdav_`；未设置时跳过并输出提示。
- 集成测试只写入随机前缀 `jiyue-it-*` 下的对象，结束时清理（包括版本和删除标记）。
- S3 端到端测试（两台设备、冲突、删除、重命名、中断与恢复、版本历史、错误凭据、不可达地址）：`tests/s3_e2e.rs`。需要版本控制桶和同样的 `JIYUE_TEST_S3_*` 变量，运行 `cargo test -p jiyue-sync --test s3_e2e -- --ignored --test-threads=1 --nocapture`；或用 `scripts/run-s3-e2e.ps1`。每个测试使用随机前缀 `jiyue-e2e-*`，结束时删除其下所有版本。
- 无需服务器的回归测试（同一文件）：`cargo test -p jiyue-sync --test s3_e2e`。
- WebDAV 未做端到端验证（按用户决定跳过）。WebDAV 集成测试仍在，未设置变量时跳过。
- 真实 WebDAV 服务尚未验证；仅 S3 集成测试在 RustFS 上运行过。
