// 安装时把对应平台的二进制下载到 vendor/。
//
// 为什么不把二进制打进 npm 包：四个平台加起来 32MB，所有人都会下载全部四个。
// 也不做成 optionalDependencies 的四个子包：那样版本要同步五处，发包也要发五次。
// 一个 postinstall 换一次网络请求，是这个体量下的合理取舍。

const { execFileSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const REPO = "lindenthomaslin/vibeguard";
const { version } = require("./package.json");

function target() {
  const platform = process.platform;
  const arch = process.arch;

  const os = { darwin: "apple-darwin", linux: "unknown-linux-gnu" }[platform];
  if (!os) {
    throw new Error(
      `暂时只支持 macOS 和 Linux，检测到 ${platform}。` +
        `\nWindows 还没发布构建（权限模型假设 Unix 布局，也没在 Windows 上测过）。`
    );
  }
  const cpu = { x64: "x86_64", arm64: "aarch64" }[arch];
  if (!cpu) throw new Error(`暂时没有 ${arch} 的构建`);
  return `${cpu}-${os}`;
}

function download(url) {
  // 用 curl 而不是自己实现 https：系统自带的更省事，而且 macOS/Linux 都有
  const out = path.join(os.tmpdir(), `vibeguard-${process.pid}-${Date.now()}`);
  execFileSync("curl", ["-fsSL", url, "-o", out], { stdio: ["ignore", "ignore", "pipe"] });
  return out;
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function main() {
  const vendor = path.join(__dirname, "vendor");
  const bin = path.join(vendor, "vibeguard");

  if (fs.existsSync(bin)) {
    // 重装、`npm rebuild` 都会再跑一次 postinstall，已经装过就别再下一遍
    console.log("vibeguard: 已经装好了，跳过下载");
    return;
  }

  if (process.env.VIBEGUARD_SKIP_DOWNLOAD === "1") {
    console.log("vibeguard: VIBEGUARD_SKIP_DOWNLOAD=1，跳过下载");
    return;
  }

  const t = target();
  const base = `https://github.com/${REPO}/releases/download/v${version}`;
  const name = `vibeguard-${t}`;
  console.log(`vibeguard: 正在下载 ${name} …`);

  let pkg, sum;
  try {
    pkg = download(`${base}/${name}.tar.gz`);
    sum = download(`${base}/${name}.tar.gz.sha256`);
  } catch (e) {
    throw new Error(
      `下载失败（${e.message}）。\n` +
        `可以改用官方脚本安装：\n` +
        `  curl -fsSL https://raw.githubusercontent.com/${REPO}/master/install.sh | sh`
    );
  }

  const want = fs.readFileSync(sum, "utf8").trim().split(/\s+/)[0];
  const got = sha256(pkg);
  if (want !== got) {
    throw new Error(`校验和不匹配，已中止（期望 ${want}，实际 ${got}）`);
  }

  fs.mkdirSync(vendor, { recursive: true });
  // tar 在 macOS 和 Linux 上都是自带的；Windows 已经在上面挡掉了
  execFileSync("tar", ["-xzf", pkg, "-C", vendor], { stdio: "inherit" });
  fs.chmodSync(bin, 0o755);

  for (const f of [pkg, sum]) fs.rmSync(f, { force: true });
  console.log(`vibeguard: 装好了（${t}）`);
}

try {
  main();
} catch (e) {
  console.error(`\nvibeguard 安装失败：${e.message}\n`);
  process.exit(1);
}
