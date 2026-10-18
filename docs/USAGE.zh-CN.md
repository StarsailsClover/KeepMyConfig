# KeepMyConfig 使用参考

> GitHub@OxygenAILab | OxygenAILab@StarsailsClover

## 1. 目录结构

```
<CODEX_HOME>/                      # 默认 ~/.codex（或 $CODEX_HOME）
├── config.toml
└── .keepmyconfig/
    ├── policy.toml                # 托管/忽略规则、合并模式、资产策略
    ├── state.json                 # 基线哈希、计数、最近事件
    ├── baseline/config.toml       # 最近一次可信的完整配置
    ├── overlay/config.toml        # 基线中受保护的投影
    ├── journal.jsonl              # 追加式审计日志
    └── backups/
        ├── <ts>-pre-repair.toml
        ├── assets-<ts>/
        └── cc-switch-<ts>.db
```

全局参数：`--codex-home PATH`、`--store PATH`、`--json`。
环境变量：`CODEX_HOME`、`KMC_CODEX_HOME`、`KMC_STORE`、`KMC_CCSWITCH_DB`。

## 2. 策略文件

```toml
schema = 1
merge_mode = "overlay_wins"   # 或 "live_wins"
detection = "balanced"        # "strict" / "balanced" / "off"

managed = [                   # 供应商标识：不保护，永远取 live
  "model", "model_provider", "model_catalog_json",
  "base_url", "wire_api", "model_providers",
  "experimental_bearer_token", "web_search",
]

ignored = [                   # 应用自管高频变化：不保护也不恢复
  "mcp_servers.node_repl",
  "notify",
]

pinned = [                    # 即使在托管子树内也强制保留这些路径
  # "model_providers.SailsAPI.name",
]

[assets]
dirs = ["skills", "plugins", "prompts", "rules"]
files = ["config.toml", "AGENTS.md", "requirements.toml"]
globs = ["config.toml.bak-*"]
backup_only = ["auth.json"]
exclude = ["**/node_modules/**", "**/.git/**", "**/target/**", "**/__pycache__/**", "**/*.zip", "plugins/cache/**"]
max_file_bytes = 52428800
```

- 规则是点分 TOML 路径；匹配路径**及其所有子路径**，如 `model_providers` 覆盖整表。
- `mcp_servers.node_repl` 与 `notify` 由 Codex 桌面应用写入，内含随版本变化的
  runtime 路径（`runtimes\cua_node\<hash>\...`）。App 更新后 overlay 里的副本即失效，
  而显式 `repair` 会无条件合并，因此保护它们等于把死路径写回去；两者默认忽略。
  若你手工维护并希望保护，请把它从 `ignored` 中移除。
- 含点或反斜杠的键在路径里带引号，例如
  `projects."c:\\work\\demo".trust_level`；可用 `projects.*` 匹配。
- `overlay_wins`（默认）在冲突时恢复你的值；`live_wins` 尊重新写入。
<!-- Gi tHub@OxygenAI   Lab |   Ox  ygenAILab@Starsail sClover -->
- `detection = "off"` 关闭 watch 自动修复；手工 `repair` 仍可用。
- `pinned` 中的路径优先于 `managed`，适合固定必须保持的供应商字段（例如第三方
  供应商的 `name`）。只有父表存在时才会写入 pinned 叶节点，删除 pinned 会被判定
  为覆写并自动修复。

## 3. 日常流程

### 首次使用

```powershell
keepmyconfig init
keepmyconfig status
keepmyconfig watch
```

开机自启（无需管理员）：`Win+R` 输入 `shell:startup`，创建
`KeepMyConfig.cmd`：

```bat
@echo off
"C:\path\to\keepmyconfig.exe" watch --quiet
```

需要崩溃自动重启时，改用任务计划程序“登录时触发 + 失败重启”。

### 有意修改配置后

watch 运行时，用户编辑与 Codex 自身更新会被自动采集。若 watch 未运行，执行：

```powershell
keepmyconfig capture
```

### CC Switch 切换供应商后

watch 运行时会自动修复。手动检查：

```powershell
keepmyconfig status --check   # 有漂移时退出码 2
keepmyconfig repair --dry-run
keepmyconfig repair
```

### 在重启与 Codex 更新后继续生效（Windows）

```powershell
keepmyconfig autostart install            # 登录守护 + 每 5 分钟检查
keepmyconfig autostart install --interval-minutes 10
keepmyconfig autostart install --method auto|task|startup
keepmyconfig autostart install --no-daemon
keepmyconfig autostart status
keepmyconfig autostart uninstall
```

`install` 会在 `keepmyconfig.exe` 旁写两个包装脚本，注册
`KeepMyConfig-Watch`（登录时运行 `watch`）与 `KeepMyConfig-Check`（按计划运行
`watch --once --quiet`）两个当前用户计划任务，并立即各启动一次。即使守护进程被应用
更新或重启杀掉，周期任务也会持续修复。若当前会话无权创建计划任务（非提权会话常见），
`auto` 会回退到启动文件夹循环脚本，每 N 分钟执行一次单次修复；也可用
`--method startup` 直接选择该方式。

### 已经丢失配置的抢救

```powershell
keepmyconfig init
keepmyconfig capture --from "$HOME\.codex\config.toml.bak-20261002-135358"
keepmyconfig repair
```

`--from` 会先经过同一套策略投影，只恢复用户键；供应商标识仍取 live。

## 4. 资产备份

```powershell
keepmyconfig backup                          # config.toml、config.toml.bak-*、auth.json
keepmyconfig backup --assets config,skills
keepmyconfig backup --assets all
keepmyconfig backup --assets all --link      # 硬链接，省空间
keepmyconfig backup --assets plugins --include_cache
```

恢复默认只补缺失文件：

```powershell
keepmyconfig restore-assets
keepmyconfig restore-assets --dry-run
keepmyconfig restore-assets --overwrite
keepmyconfig restore-assets --include-backup-only   # 连 auth.json 一起恢复
```

默认排除 `node_modules`、`.git`、`target`、`__pycache__`、zip 与
<!-- GitHub@Oxyge nAILab | Oxyg   enAILab@StarsailsClo  ver -->
`plugins/cache/**`；超过 50 MiB 的文件跳过并计入报告。

`status` 会显示最近快照，`doctor` 在没有快照或快照超过 30 天时告警。同一卷上使用
硬链接的快照几乎不占额外空间：

```powershell
keepmyconfig backup --assets all --link
```

## 5. CC Switch 集成

```powershell
keepmyconfig ccswitch inspect
keepmyconfig ccswitch adopt            # 预览
keepmyconfig ccswitch adopt --apply    # 需先退出 CC Switch
keepmyconfig ccswitch adopt --apply --force   # 运行中也强制写入（不推荐）
keepmyconfig ccswitch adopt --no-mcp   # 不改 mcp_servers 表
keepmyconfig ccswitch backups
keepmyconfig ccswitch restore --backup "...\cc-switch-20261002-190000.db"
```

adopt 按表结构（`settings`、`providers`、`mcp_servers`）校验数据库而非死认版本号，
记录 `PRAGMA user_version`，并在一致备份之后用一个事务写完全部改动。

当本地基线与 Codex 自带备份都已丢失时，可从 CC Switch 保存的供应商配置里恢复：

```powershell
keepmyconfig ccswitch recover                 # 预览，自动选择最完整的配置
keepmyconfig ccswitch recover --provider "Sails API"
keepmyconfig ccswitch recover --apply
```

该命令会把全部可用来源按保护策略投影：所有 Codex 供应商配置（最完整的优先，并列时
优先当前供应商），再加上从 `mcp_servers` 表中 `enabled_codex = 1` 行生成的合成
MCP registry 文档；以 `overlay_wins` 合并并集，并在写入前生成 `pre-recover` 备份。
传 `--provider NAME` 时只使用该供应商配置（仍会附加 MCP registry）。

## 6. 检测评分

| 信号 | 分值 |
|---|---|
| 至少 1 个受保护路径消失 | +1 |
| 至少 3 个 | +1 |
| 至少 8 个 | +1 |
| 供应商托管键发生变化 | +2 |
| 无新增用户路径 | +1（1–5 个：0；更多：−1） |
| 文件缩小 ≥30% | +1 |
| 一次删除 3 个及以上完整的 MCP/Plugin/Marketplace/Project 条目 | +2 |
| `BROWSER_USE_CODEX_APP_VERSION` 变化且受保护路径消失 | 必定判定为覆写 |

`balanced` 阈值 4，`strict` 阈值 3，`off` 从不自动修复。

## 7. 退出码与 JSON

| 退出码 | 含义 |
|---|---|
| 0 | 成功 / 健康 |
<!-- GitHub@Oxyg  enAILab | OxygenAILab@Sta rsailsC   lover -->
| 1 | 错误（含 doctor 结构性问题） |
| 2 | `--check` 发现漂移或覆写特征 |

所有报告类命令支持 `--json`；`journal.jsonl` 为 JSON Lines。

## 8. 常见问题

| 现象 | 处理 |
|---|---|
| `store is not initialized` | `keepmyconfig init` |
| `another process holds .../.lock` | 关闭另一个 watch/单次进程 |
| `cannot parse TOML` | 文件正在被写入；watch 会重试，或稍后 status |
| 修复把你不再想要的值带回来了 | 用 `capture` 重建基线，或改 `policy.toml` |
| CC Switch 仍然反复覆写 | `keepmyconfig ccswitch adopt --apply` |
| `auth.json` 不是最新 | 它被刻意排除在自动恢复之外；重新登录，或谨慎使用 `restore-assets --include-backup-only` |
| Codex 桌面端回到旧界面、中文消失 | 壳的界面资源是启动时联网获取的，一旦抓取失败就会回退到内置经典界面。恢复可用出口后完全退出并重开即可，**无需重新登录**。用 `curl https://chatgpt.com` 返回 403 是 Cloudflare 拦脚本，不能作为判据；判断出口请用 `https://github.com`。 |
| 换代理或节点后界面又退回旧版 | 把 Clash 分组（`GLOBAL` 或你的订阅分组）切到可达地区即可。Clash for Windows 会把选择持久化到 `profiles/list.yml` 与 `cache.db`，重启不丢。**目前没有支持的方式能锁定壳版本。** |

## 9. 安全说明

- 无网络请求、无遥测。
- `auth.json` 默认不参与合并/恢复，因为 CC Switch 会按供应商合法轮换它。
- CC Switch 数据库写入必须显式 `--apply`，先备份，运行中除非 `--force` 否则拒绝。

GitHub@OxygenAILab | OxygenAILab@StarsailsClover
