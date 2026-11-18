# F-Droid 上架构建指南

`cool.jiyue.jiyue_mobile` 已针对 F-Droid Inclusion Policy 完成合规改造，可从全开源源码构建纯净版 `foss` APK。

## 已落实的 F-Droid 合规要点

1. **移除闭源依赖**：移除了依赖闭源 `com.google.mlkit:barcode-scanning` 的 `mobile_scanner`，改用纯开源 `camera`（CameraX）+ `zxing2` + `image` 实现相机实时扫码与相册二维码图片识别。
2. **双发布风味隔离 (`github` vs `foss`)**：
   - `mobile/android/app/build.gradle.kts` 配置了 `flavorDimensions += "distribution"`，划分为 `github` 与 `foss` 两个风味；
   - `REQUEST_INSTALL_PACKAGES` 权限与 APK 自更新安装逻辑（`FlavorUpdater.kt`）已完全隔离在 `mobile/android/app/src/github/` 下，`src/foss/` 不包含任何自更新权限、Intent 或安装器代码；
   - 构建 `foss` 时传入 `--dart-define=FOSS_BUILD=true`，界面隐藏应用内“检查更新”下载入口，仅展示版本与开源发布页外链。
3. **关闭签名块依赖加密元数据**：`build.gradle.kts` 已显式禁用 `dependenciesInfo`（`includeInApk = false` / `includeInBundle = false`）。
4. **Rust FFI 源码交叉编译**：仓库内不提交任何预编译 `.so`（已写入 `.gitignore`），构建阶段通过 `scripts/build-android-ffi.py --locked` 从 `crates/jiyue-ffi` 现场交叉编译生成 `libjiyue_ffi.so`。

## Fastlane 元数据（已就绪）

应用标题、简介、完整介绍、512×512 图标、真机预览图与版本更新日志已放置于仓库标准路径：
- `fastlane/metadata/android/{zh-CN,en-US}/{title,short_description,full_description}.txt`
- `fastlane/metadata/android/{zh-CN,en-US}/images/icon.png`（512×512 PNG）
- `fastlane/metadata/android/{zh-CN,en-US}/images/phoneScreenshots/{1,2,3}.png`
- `fastlane/metadata/android/{zh-CN,en-US}/changelogs/<versionCode>.txt`

## 提交到官方 F-Droid (`fdroiddata`) 步骤

1. 确认新 Release Tag（如 `v0.4.0`）已推送到 GitHub，且 `mobile/pubspec.yaml` 中 `version: 0.4.0+10` 与 [cool.jiyue.jiyue_mobile.yml](cool.jiyue.jiyue_mobile.yml) 一致。
2. 在 GitLab Fork [`fdroid/fdroiddata`](https://gitlab.com/fdroid/fdroiddata)。
3. 将 [cool.jiyue.jiyue_mobile.yml](cool.jiyue.jiyue_mobile.yml) 复制为 `metadata/cool.jiyue.jiyue_mobile.yml`（需确保 `License: Apache-2.0 OR MIT` 与 `AutoUpdateMode: Version v%v` 符合最新 `fdroid lint` 规则）。
4. （可选，若在 Linux/WSL 环境有 `fdroidserver`）运行合规与可构建校验：
   ```bash
   fdroid readmeta
   fdroid lint cool.jiyue.jiyue_mobile
   fdroid build -v -l cool.jiyue.jiyue_mobile
   ```
5. 在 GitLab 向 `fdroid/fdroiddata` 提交 Merge Request（标题：`New App: cool.jiyue.jiyue_mobile`），等待 CI 跑通 `fdroid scanner` 与自动源码构建审核。

## 同步提交 IzzyOnDroid（1–2 天极速上架）

`Jiyue-<version>-android-foss.apk` 已去掉 Google 专有组件与 `REQUEST_INSTALL_PACKAGES` 并禁用 `dependenciesInfo`，直接满足 IzzyOnDroid 收录规范。可在 [IzzyOnDroid Repo Issues](https://gitlab.com/IzzyOnDroid/repo/-/issues) 提交收录请求（选择 *App Submission* 模板）：
- **Application ID**：`cool.jiyue.jiyue_mobile`
- **Source Repo**：`https://github.com/limitcool/jiyue`
- **APK 资产**：使用 GitHub Release 产物中的 `Jiyue-.*-android-foss\.apk$`
- **构建说明**：由 GitHub Actions CI 从纯开源源码 + `scripts/build-android-ffi.py --locked` 编译签名，可由 [cool.jiyue.jiyue_mobile.yml](cool.jiyue.jiyue_mobile.yml) 完全从源码复现。
