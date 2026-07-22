# 代办：Windows 发布与自动更新 v0.1

> 状态：`v0.1.0` GitHub Release 已发布，公网安装器、更新签名与 `latest.json` 已验证；首次安装和版本间升级闭环待实机验证
> 更新时间：2026-07-22
> 适用版本：`0.1.0`

## 1. 当前结论

| 项目 | 状态 | 说明 |
|---|---|---|
| Windows x64 NSIS 安装器 | 已实现并完成本地验证 | 本机已成功生成 `setup.exe` |
| Tauri 更新签名 | 已实现并完成本地验证 | 本机已成功生成对应 `.sig` |
| 应用内检查与安装 | 已实现 | normal 模式启动检查、24 小时间隔、菜单手动检查和用户点击安装 |
| smoke 更新网络隔离 | 已实现 | 前端不启动更新控制器，Rust 在访问网络前再次拒绝 |
| GitHub Actions 发布 | 已实现并完成首次验证 | 支持手动触发和 `v*` 标签触发；`v0.1.0` 流水线已成功 |
| 首个 GitHub Release | 已完成 | `v0.1.0` 已发布为 Latest，安装器、`.sig` 与 `latest.json` 均已上传 |
| `latest.json` 线上读取 | 已验证 | 已从公开 Latest 地址匿名读取，并按其中地址下载、验签安装器及核对资产摘要 |
| Windows Authenticode | 暂未配置 | 安装器可能触发 Microsoft Defender SmartScreen 提示 |

## 2. 目标与边界

第一版只提供 Windows x64 NSIS 安装与 GitHub Releases 自动更新，不增加账号、云同步或其他平台承诺。

- 首次安装必须由用户从 GitHub Releases 手动下载 `setup.exe`。
- 安装范围为当前 Windows 用户，不要求管理员级全局安装。
- 自动更新只在 normal 模式运行，不影响核心待办的离线使用。
- 发现新版本不会直接安装；用户点击“安装更新”才表示确认。
- 更新失败必须显式提示，不静默回退到旧更新实现。
- 当前不提供 MSI、macOS 或 Linux 安装包。

## 3. 用户流程

### 第 1 轮：首次安装

1. 发布者创建 GitHub Release，并上传 Windows x64 `setup.exe`、更新签名和 `latest.json`。
2. 用户从 Releases 页面手动下载 `setup.exe`。
3. 用户运行安装器并完成当前用户安装；未配置 Authenticode 时，Windows 可能显示 SmartScreen 提示。
4. 应用首次启动后进入 normal 模式，核心待办数据仍保存在本机 SQLite。

### 第 2 轮：自动或手动检查

1. normal 模式启动后，`UpdateController` 静默检查一次更新。
2. 应用保持运行时，每 24 小时再次检查。
3. 用户也可以从更多菜单选择“检查更新”。
4. 没有更新时，自动检查保持安静；手动检查会提示已是最新版。
5. 发现更新时，界面显示可用版本并把菜单动作改为“安装更新”。

### 第 3 轮：用户确认安装

1. 用户点击“安装更新”，这是当前交互中的明确确认，不额外虚构确认弹窗。
2. 前端先确认账本会话当前可安全进入更新流程，再临时锁住页面交互。
3. `install_update({ expectedVersion })` 只能安装 Rust 服务当前保存的同一版本；并发重复安装会被拒绝。
4. Tauri updater 下载并校验签名产物，退出前保存窗口位置，安装完成后重启应用。

## 4. 组件与依赖方向

```text
UpdateController
→ TauriGateway.checkForUpdate() / installUpdate(expectedVersion)
→ check_for_update / install_update 项目自有 Tauri 命令
→ AppUpdateService
→ tauri-plugin-updater
→ GitHub Releases / latest.json
```

| 组件 | 责任 | 不负责 |
|---|---|---|
| `UpdateController` | 启动检查、24 小时间隔、菜单状态、提示和用户安装动作 | 不直接访问网络，不持有 updater 对象 |
| `TauriGateway` | 隔离 IPC 命令名与 camelCase 参数 | 不决定是否允许更新 |
| 更新薄命令 | 将检查或精确版本安装意图交给服务 | 不复制模式和安装规则 |
| `AppUpdateService` | normal 校验、待安装版本、单次安装锁和 updater 适配 | 不处理待办领域状态 |
| Tauri updater | 读取更新元数据、下载、签名校验、安装与重启 | 不掌握页面调度和 smoke 入口显示 |

前端没有直接使用 updater 插件权限，更新能力统一通过项目自有 Rust 命令暴露，避免插件 API 散落到页面代码。

## 5. normal 与 smoke 双层禁网

更新网络只允许 normal 模式：

1. 前端层：smoke 不启动 `UpdateController`，并隐藏更多菜单中的更新入口。
2. Rust 层：`AppUpdateService` 在构造 updater、读取 `latest.json` 或下载产物前检查运行配置；smoke 返回稳定错误 `UPDATE_DISABLED_IN_SMOKE`。

这两层是相互独立的安全边界。即使测试或错误代码绕过前端，Rust 仍不会访问正式更新源。

## 6. 发布工作流

`.github/workflows/release.yml` 支持两种触发方式：

- GitHub Actions 页面从 `main` 手动执行 `workflow_dispatch`。
- 推送符合 `v*` 的版本标签。

工作流按以下顺序执行：

1. 安装锁定的 Node 依赖与 Rust stable 工具链。
2. 校验 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml` 版本一致；标签触发时还要求标签等于 `v<版本号>`。
3. 运行统一门禁、Rust 格式检查和 Clippy。
4. 通过 `tauri-apps/tauri-action@v1` 构建 Windows x64 NSIS，并把安装器、`.sig` 与 `latest.json` 上传到草稿 Release。
5. 使用客户端内嵌公钥对实际安装器和 `.sig` 做密码学验签，同时核对 `latest.json` 的版本、平台、签名和下载地址；再按元数据地址重新下载安装器并与本次构建产物比较 SHA-256。
6. 只有验证通过，才把草稿自动转为正式 GitHub Release；失败时保留草稿，不进入 `releases/latest`。

`v0.1.0` 已通过上述流程发布：[GitHub Actions 运行记录](https://github.com/mhdfy1988/todo/actions/runs/29899650153)。发布后又通过公开 Latest 地址独立下载并复验了元数据、安装器、签名与 GitHub 资产摘要。该结果证明首发产物链路可用，但不等于已经完成后续版本的应用内升级。

## 7. 发布密钥

GitHub Actions 只约定以下 Secrets 名称：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

私钥、密码和它们的真实内容绝不能写入仓库、文档、日志或聊天记录。建议在访问受控、加密的离线介质中保留恢复副本，并由项目维护者记录恢复与轮换责任；备份可用性应在不暴露密钥内容的前提下定期确认。

应用配置中只保存用于验证更新签名的公钥。私钥丢失后无法继续沿用同一信任链签发更新；若直接改用新密钥，既有安装版本默认不会信任新签名，因此首发前必须确认备份可恢复。

## 8. updater 签名与 Authenticode

两者用途不同：

- Tauri updater 的 `.sig`：由应用用于验证下载的更新产物是否由项目发布密钥签出。
- Windows Authenticode：由 Windows 用于验证安装器发布者身份，并影响 SmartScreen 信任积累。

当前只完成 updater 签名，尚未配置 Authenticode。文案和发布说明不能把 `.sig` 描述成“安装器已经获得 Windows 代码签名”。

## 9. 首次发布检查单

- [x] 确认三个版本号一致，并确认标签为 `v0.1.0`。
- [x] 确认两个 GitHub Actions Secrets 已配置，私钥未进入仓库。
- [x] 运行统一门禁、格式检查、Clippy 和 Windows NSIS 构建。
- [x] 创建首个 GitHub Release，核对 `setup.exe`、`.sig` 与 `latest.json`。
- [ ] 在干净的 Windows x64 当前用户环境完成手动首次安装。
- [ ] 确认 normal 启动检查和菜单手动检查能读取线上元数据。
- [x] 确认 smoke 前端无入口，Rust 在访问网络前拒绝更新。
- [ ] 用后续版本完成一次“发现更新 → 用户点击安装 → 下载校验 → 安装重启”实机闭环。
- [ ] 记录 SmartScreen 实际表现；若进入公开分发，再单独评估 Authenticode 证书。

当前准确口径是“`v0.1.0` 首次线上发布与公网产物验证已完成；手动首次安装和后续版本的应用内升级闭环仍待实机验证”。
