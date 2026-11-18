# Obtainium 上架与一键订阅

## 方式一：直接一键导入（已就绪，零门槛）

按照 Obtainium 官方 [Badging 规范](https://github.com/ImranR98/apps.obtainium.imranr.dev/blob/main/BADGING.md)，项目已自托管 [badge_obtainium.png](../../docs/images/badge_obtainium.png) 并内嵌一键跳转链接。

- **标准签名版 (`Jiyue-<ver>-android.apk`)** 一键订阅：
  https://apps.obtainium.imranr.dev/redirect?r=obtainium://app/%7B%22id%22%3A%22cool.jiyue.jiyue_mobile%22%2C%22url%22%3A%22https%3A%2F%2Fgithub.com%2Flimitcool%2Fjiyue%22%2C%22author%22%3A%22limitcool%22%2C%22name%22%3A%22Jiyue%20Notes%22%2C%22additionalSettings%22%3A%22%7B%5C%22apkFilterRegEx%5C%22%3A%5C%22Jiyue-.%2A-android%5C%5C%5C%5C.apk%24%5C%22%2C%5C%22autoApkFilterByArch%5C%22%3Afalse%7D%22%7D
- **FOSS 纯净版 (`Jiyue-<ver>-android-foss.apk`)** 一键订阅：
  https://apps.obtainium.imranr.dev/redirect?r=obtainium://app/%7B%22id%22%3A%22cool.jiyue.jiyue_mobile%22%2C%22url%22%3A%22https%3A%2F%2Fgithub.com%2Flimitcool%2Fjiyue%22%2C%22author%22%3A%22limitcool%22%2C%22name%22%3A%22Jiyue%20Notes%20%28FOSS%29%22%2C%22additionalSettings%22%3A%22%7B%5C%22apkFilterRegEx%5C%22%3A%5C%22Jiyue-.%2A-android-foss%5C%5C%5C%5C.apk%24%5C%22%2C%5C%22autoApkFilterByArch%5C%22%3Afalse%7D%22%7D

## 方式二：提交到众包应用目录 (`apps.obtainium.imranr.dev`)

[cool.jiyue.jiyue_mobile.json](cool.jiyue.jiyue_mobile.json) 已按官方 Complex App Schema（含标准版与 FOSS 版配置、`fastlane` 图标与 `categories` 字段）编写完成。

> 注：`ImranR98/apps.obtainium.imranr.dev` 目录仓库要求 GitHub 源仓库满足 **≥ 35 Stars 且项目创建满 4 个月**。条件满足后只需 Fork 该仓库，将 `cool.jiyue.jiyue_mobile.json` 复制至 `public/data/apps/complex/cool.jiyue.jiyue_mobile.json` 并发起 PR 即可收录进搜索目录。
