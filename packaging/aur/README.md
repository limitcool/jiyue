# AUR (`jiyue-bin`) 发布指南

[PKGBUILD](jiyue-bin/PKGBUILD) 与 [.SRCINFO](jiyue-bin/.SRCINFO) 基于 GitHub Releases 中的 `.deb` 二进制产物构建，支持 `x86_64` 与 `aarch64`。

## 前置准备（仅首次）

1. 注册 [AUR 账号](https://aur.archlinux.org/register) 并在 Account Settings 中绑定你的 SSH 公钥。
2. 配置 `~/.ssh/config`（若使用非默认私钥）：
   ```sshconfig
   Host aur.archlinux.org
     IdentityFile ~/.ssh/id_ed25519
     User aur
   ```

## 推送 / 更新到 AUR

在任意装有 `git` 的环境（Windows / Linux / WSL 均可，因 `.SRCINFO` 已由本仓维护并随 `scripts/update-packaging.py` 自动同步）：

```bash
git clone ssh://aur@aur.archlinux.org/jiyue-bin.git
cp packaging/aur/jiyue-bin/PKGBUILD packaging/aur/jiyue-bin/.SRCINFO jiyue-bin/
cd jiyue-bin
git add PKGBUILD .SRCINFO
git commit -m "chore: release 0.4.0"
git push
```

若你在 Arch Linux 本机测试，也可在提交前验证：
```bash
makepkg --printsrcinfo > .SRCINFO
namcap PKGBUILD
```
