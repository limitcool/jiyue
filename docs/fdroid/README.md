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

## Fastlane 元数据

应用标题、简介、完整介绍位于仓库标准路径：
- `fastlane/metadata/android/zh-CN/`
- `fastlane/metadata/android/en-US/`

建议在提交 `fdroiddata` MR 前补充真机截屏到：
- `fastlane/metadata/android/zh-CN/images/phoneScreenshots/`
- `fastlane/metadata/android/en-US/images/phoneScreenshots/`

## 提交到 F-Droid (`fdroiddata`) 步骤

1. 推送含本改造的新 Release Tag（例如 `v0.3.7`，且确认 `mobile/pubspec.yaml` 中 `version: 0.3.7+8` 与 Tag 一致）。
2. Fork GitLab 上的 [`fdroid/fdroiddata`](https://gitlab.com/fdroid/fdroiddata)。
3. 将 [cool.jiyue.jiyue_mobile.yml](cool.jiyue.jiyue_mobile.yml) 复制到 `fdroiddata` 仓库的 `metadata/cool.jiyue.jiyue_mobile.yml`，并将文件中的 `versionName`、`versionCode`、`commit`、`CurrentVersion`、`CurrentVersionCode` 替换为新发布的实际 Tag。
4. 本地可使用 `fdroidserver` 检查配方合规性与完整源码构建：
   ```bash
   fdroid readmeta
   fdroid lint cool.jiyue.jiyue_mobile
   fdroid build -v -l cool.jiyue.jiyue_mobile
   ```
5. 在 GitLab 向 `fdroid/fdroiddata` 提交 Merge Request（标题如 `New App: cool.jiyue.jiyue_mobile`），等待 CI 跑通 `fdroid scanner` 与可复现性构建。
