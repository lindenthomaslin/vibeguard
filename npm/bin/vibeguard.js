#!/usr/bin/env node
// npm 的 bin 只是一个转发器：真正的二进制在 vendor/ 里，
// 由 install.js 在安装时下载。stdio 必须 inherit，
// 否则 TUI 拿到的不是 TTY，会退化成一问一答的行模式。

const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const bin = path.join(__dirname, "..", "vendor", "vibeguard");

if (!fs.existsSync(bin)) {
  console.error(
    [
      "vibeguard 的二进制还没下载。",
      "",
      "常见原因：用 `npm install --ignore-scripts` 装的话，postinstall 没跑。",
      "补救：",
      "  npm rebuild vibeguard",
      "",
      "或者改用官方脚本：",
      "  curl -fsSL https://raw.githubusercontent.com/lindenthomaslin/vibeguard/master/install.sh | sh",
    ].join("\n")
  );
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });

// 信号要转发：TUI 里 Ctrl-C 是「取消当前请求」，直接吞掉会让人以为卡死
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP", "SIGQUIT"]) {
  process.on(sig, () => {
    if (!child.killed) child.kill(sig);
  });
}

child.on("error", (e) => {
  console.error(`vibeguard 启动失败：${e.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    // 被信号杀掉时照原样退出，让调用方（shell、CI）看到同样的结果
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code === null ? 1 : code);
});
