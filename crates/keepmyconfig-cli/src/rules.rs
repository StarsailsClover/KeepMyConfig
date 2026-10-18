//! Print the recommended AGENTS.md collaboration block.
//!
//! Other sessions only need to be told two things: which directory to talk in,
//! and which command to use. This module emits the exact text so the Owner can
//! keep it identical across projects.

const RULE_ZH: &str = r#"## 跨会话协作(必须遵守)

1. 不要使用 Codex 应用内的 `create_thread` / `send_message_to_thread` 互发消息。
   它会给目标会话注入一条没有配对调用的工具结果,严格上游会直接拒绝整轮
   (`tool calls and tool results do not match`),会话会永久卡死。
2. 只用协作总线目录对话:`C:\Users\demo\Documents\Cowork\<频道>\`(实际以 `--cowork PATH` 为准)
   - 频道按主题分,例如 `demo-ui`、`edu`、`general`。
   - 命令:`keepmyconfig bus ...`(`keepmyconfig` 已在 PATH)。

### 三个动作

```
keepmyconfig bus read --channel <频道> --as "<你的会话名>"              # 看有没有给你的新消息(会推进已读游标)
keepmyconfig bus post --channel <频道> --from "<你的会话名>" --to <对方> --body "<正文>" [--ask Q1] [--wants-reply]
keepmyconfig bus read --channel <频道> --as "<你的会话名>" --full --peek  # 只想通读全部、不推进游标时
```

- 会话名沿用现有写法,如 `UI (UI9)` 或 `App (DemoApp)`;只用括号前的名字做身份
  (`UI` 与 `UI (UI9)` 视为同一人)。
- `--to` 可写多个(逗号分隔)或 `all` 广播。
- 有需要对方解答的事项就加 `--wants-reply` 并用 `--ask Q1,Q2` 编号。
- 发言格式与历史约定不变(见频道内 `README.md` / `messages.md`)。
- 发完请在本会话口头告知 Owner,由 Owner 决定是否唤起对方会话;
  **不要**用应用内消息或唤醒手段去戳对方会话。

### 找环境运维(Codex / CC Switch / NewAPI)

任何环境问题(会话报 `tool calls and tool results do not match`、CC Switch 切换后
配置被覆盖、NewAPI 渠道报错、桌面端界面回退等)都写到公共频道 `env-ops`,运维会处理:

```
keepmyconfig bus post --channel "env-ops" --from "<你的会话名>" --to "Ops" --ask Q1 --wants-reply --body "<问题>"
keepmyconfig bus read --channel "env-ops" --as "<你的会话名>"     # 收运维的回复
```

运维有每约 5 分钟一次的守护在监听该频道(只弹通知、绝不唤醒会话)。
"#;

const RULE_EN: &str = r#"## Cross-session collaboration (required)

1. Do not use the Codex in-app `create_thread` / `send_message_to_thread` to
   message another session. It injects a lone tool result with no matching call
   into the target session; a strict upstream rejects every turn with
   `tool calls and tool results do not match`, and the session stalls for good.
2. Talk only through the collaboration bus directory,
   `C:\Users\demo\Documents\Cowork\<channel>\`(or pass `--cowork PATH`):
   - One channel per topic, for example `demo-ui`, `edu`, `general`.
   - Use `keepmyconfig bus ...` (`keepmyconfig` is on PATH).

### Three actions

```
keepmyconfig bus read --channel <channel> --as "<your session name>"
keepmyconfig bus post --channel <channel> --from "<your session name>" --to <peer> --body "<text>" [--ask Q1] [--wants-reply]
keepmyconfig bus read --channel <channel> --as "<your session name>" --full --peek
```

- Keep the existing naming, such as `UI (UI9)`; identity is the name before any
  parenthesis, lowercased (`UI` and `UI (UI9)` are the same reader).
- `--to` accepts several comma-separated names or `all`.
- Add `--wants-reply` and number items with `--ask Q1,Q2` when you need answers.
- Posting format is unchanged; see the channel `README.md` / `messages.md`.
- Tell the Owner in your own session after posting; never wake a peer session
  yourself.

### Reaching environment Ops (Codex / CC Switch / NewAPI)

Send environment problems (a session failing with `tool calls and tool results do
not match`, configuration clobbered after a CC Switch switch, NewAPI channel
errors, desktop UI falling back, ...) to the shared `env-ops` channel:

```
keepmyconfig bus post --channel "env-ops" --from "<your session name>" --to "Ops" --ask Q1 --wants-reply --body "<question>"
keepmyconfig bus read --channel "env-ops" --as "<your session name>"
```

Ops runs a notify-only watcher on that channel about every 5 minutes; it never
wakes a session.
"#;

/// Return the rule text for a language key (`zh` or `en`).
// G  itHub@Oxyg enAILa b | OxygenAILab@ Sta  rsailsClover
pub fn text(language: &str) -> &'static str {
    match language {
        "en" => RULE_EN,
        _ => RULE_ZH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_text_mentions_the_bus_and_the_ban() {
        for text in [RULE_ZH, RULE_EN] {
            assert!(text.contains("keepmyconfig bus"));
            assert!(text.contains("Cowork"));
            assert!(text.contains("send_message_to_thread"));
            assert!(text.contains("env-ops"));
        }
        assert!(!text("zh").is_empty());
        // G   itHub@OxygenAILa  b | OxygenA   ILab@StarsailsClover
    }
}
