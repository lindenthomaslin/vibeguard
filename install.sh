#!/bin/sh
# 装 VibeGuard：一行命令，不需要 Rust。
#
#   curl -fsSL https://raw.githubusercontent.com/lindenthomaslin/vibeguard/master/install.sh | sh
#
# 想装到别处：VIBEGUARD_INSTALL_DIR=/usr/local/bin sh install.sh
# 想装指定版本：VIBEGUARD_VERSION=v0.1.0 sh install.sh
set -eu

REPO="lindenthomaslin/vibeguard"

say() { printf '%s\n' "$*"; }
die() { printf '错误：%s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "需要 $1，但它不在 PATH 里"
}

# ---- 平台 ----
os=$(uname -s)
arch=$(uname -m)

case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux)  os_part="unknown-linux-gnu" ;;
    *) die "暂时只支持 macOS 和 Linux，检测到 $os" ;;
esac

case "$arch" in
    x86_64|amd64)  arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) die "暂时没有 $arch 的构建" ;;
esac

target="${arch_part}-${os_part}"

# ---- 版本 ----
# 不指定就取 latest：让用户装到的是最新的修复，而不是一年前的 tag
version="${VIBEGUARD_VERSION:-latest}"
if [ "$version" = "latest" ]; then
    base="https://github.com/${REPO}/releases/latest/download"
else
    base="https://github.com/${REPO}/releases/download/${version}"
fi

# 包名要和 release.yml 里拼出来的一致
if [ "$version" = "latest" ]; then
    # latest 的下载地址不带版本号，只能先问一次实际 tag
    need curl
    tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
    [ -n "$tag" ] || die "查不到最新版本（仓库还没有 release？）"
else
    tag="$version"
fi

name="vibeguard-${tag}-${target}"
url="${base}/${name}.tar.gz"

# ---- 下载 ----
need curl
need tar

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t vibeguard)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "正在下载 ${name} …"
curl -fsSL "$url" -o "$tmp/pkg.tar.gz" || die "下载失败：$url"

# ---- 校验 ----
# 有校验和就验，没有就明确说出来，不假装验过
if curl -fsSL "${url}.sha256" -o "$tmp/pkg.sha256" 2>/dev/null; then
    expected=$(cut -d' ' -f1 "$tmp/pkg.sha256")
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$tmp/pkg.tar.gz" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$tmp/pkg.tar.gz" | cut -d' ' -f1)
    fi
    [ "$expected" = "$actual" ] || die "校验和不匹配，下载可能被篡改，已中止"
    say "校验和 OK"
else
    say "注意：这个 release 没有提供校验和，跳过校验"
fi

# ---- 安装 ----
tar -xzf "$tmp/pkg.tar.gz" -C "$tmp"
[ -f "$tmp/vibeguard" ] || die "压缩包里没有 vibeguard"

dir="${VIBEGUARD_INSTALL_DIR:-}"
if [ -z "$dir" ]; then
    # 能写 /usr/local/bin 就装那儿（在 PATH 里），否则退到 ~/.local/bin
    if [ -w /usr/local/bin ]; then
        dir=/usr/local/bin
    else
        dir="$HOME/.local/bin"
    fi
fi
mkdir -p "$dir"

if [ -w "$dir" ]; then
    mv "$tmp/vibeguard" "$dir/vibeguard"
else
    say "需要管理员权限写入 $dir"
    sudo mv "$tmp/vibeguard" "$dir/vibeguard"
fi
chmod 755 "$dir/vibeguard"

say ""
say "装好了：$dir/vibeguard（$tag · $target）"

case ":${PATH}:" in
    *":${dir}:"*) ;;
    *)
        say ""
        say "$dir 不在 PATH 里，加一行到你的 shell 配置："
        say "  export PATH=\"${dir}:\$PATH\""
        ;;
esac

say ""
say "下一步：vibeguard"
