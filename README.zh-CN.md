<div align="center">
  <img src="icon.jpg" width="300" alt="VibeGuard">
</div>

<h1 align="center">VibeGuard</h1>

<p align="center">
  <b>AI 把代码写完了，却没人问一句「这玩意儿能被怎么利用」。</b><br>
  <sub>给 vibe coding 项目用的终端安全审阅器：讲清楚风险、给出修复，但你不点头，它一个字节都不写。</sub>
</p>

<p align="center">
  <img alt="rust" src="https://img.shields.io/badge/rust-2024-orange">
  <img alt="build" src="https://img.shields.io/badge/build-single%20binary-informational">
  <img alt="status" src="https://img.shields.io/badge/status-early-yellow">
  <img alt="tests" src="https://img.shields.io/badge/tests-259%20passing-brightgreen">
</p>

<p align="center">
  <a href="HANDOFF.zh-CN.md">交接文档</a> ·
  <a href="README.md">English</a>
</p>

---

## 要解决的问题

跟 AI 聊出来的代码是乐观的：happy path 写得很漂亮，unhappy path 默认是别人的问题。同样的毛病反复出现：

- 登录、注册、找回密码**没有速率限制**
- **SQL 用字符串拼**，不用参数化
- **密钥写死在源码里**，跟着一起提交
- 对请求参数用 **`shell=True` / `eval` / `exec`**
- AI 五分钟前刚加的接口，**忘了鉴权**

这些能活下来，是因为代码能编译、生成的测试能过、demo 跑得起来。整条流程里没有一环会问那句最关键的话：**这东西能被怎么利用？**

## 它怎么工作

```mermaid
flowchart LR
    A[("你的仓库")] --> B["只读模型点名要看的文件"]
    B --> C["用人话讲清楚风险"]
    C --> D["给出 diff"]
    D --> E{"你同意吗"}
    E -->|同意| F["写进文件"]
    E -->|不同意| G["什么都不变"]

    classDef read fill:#E8F6F8,stroke:#4FB3C4,color:#1B2A2E
    classDef write fill:#FDECEC,stroke:#D9534F,color:#5A1A18
    classDef ok fill:#EAF7EE,stroke:#5AA97A,color:#173A26
    class A,B,C,D read
    class F write
    class G ok
```

三种工作模式，`Shift-Tab` 循环切换，左下角常驻显示：

| 模式 | 行为 |
|---|---|
| `○ plan` | 只读。读代码、出一份漏洞报告，磁盘上什么都不动 |
| `◇ confirm` | 每处改动先显示 diff，然后停在 `oneshot` 上等你答复——问题挂在屏幕上时，文件确实一个字节都没动 |
| `◆ auto` | 不打断，直接改 |

权限模型是故意偏执的：默认 `readonly`，`plan` 连配置里给了全权限都能盖掉，`run_command` 要求 `full` 权限**并且**拒绝删除类命令。一个能改你文件的安全工具，本身就是一个攻击面。

## 安装

```bash
git clone https://github.com/lindenthomaslin/vibeguard.git
cd vibeguard
cargo build --release

export DEEPSEEK_API_KEY=sk-...   # 或者：vibeguard config init
./target/release/vibeguard
```

任何 OpenAI 兼容端点都能接，改 `~/.config/vibeguard/config.toml` 里的 `base_url`：

```bash
vibeguard config set base_url https://api.deepseek.com
vibeguard config set model deepseek-reasoner
vibeguard config set permission_mode editable
vibeguard ping
```

## 当前状态

早期阶段。对话内核是真的：流式对话、五个受权限约束的工具、三种模式、可搜索的历史对话、主题切换。**扫描器还是骨架**——漏洞模型在，但没接到界面上，那个模块里有两个测试是故意留红的记号。今天别指望它替你发现什么。

## 文档

- **[交接文档 HANDOFF.zh-CN.md](HANDOFF.zh-CN.md)** —— 为什么做这个、架构图、权限模型、配置、已知缺口，以及那些真金白银换来的依赖踩坑
- [English](HANDOFF.md)

## 许可

还没定。转载或分发之前请先问一声。
