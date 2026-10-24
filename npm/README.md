# vibeguard

A terminal security reviewer for vibe-coded projects. It reads the repository, explains the risk,
proposes a fix, and writes nothing until you say so.

```bash
npm install -g vibeguard
vibeguard
```

The npm package is a thin wrapper: `postinstall` downloads the prebuilt binary for your platform
(macOS or Linux, Intel or ARM) from the GitHub release and verifies its SHA-256. No Rust toolchain,
no build step.

Set your key once and it is remembered:

```bash
vibeguard config init        # asks for the API key, base URL and model
```

or export `DEEPSEEK_API_KEY` if you prefer the environment.

## What you get

- `/scan` — 20 local rules over a 410-item checklist, seconds, no tokens
- `/scan --ai` — the model judges the rest, each verdict backed by `file:line`
- `/fix` — a diff per finding, applied one at a time after you press `y`
- `/report` — `.md` + `.json`, both stating how much of the checklist was actually covered

## Not supported here

- Windows: no build is published. The permission model assumes a Unix layout and has not been
  tested there.
- `npm install --ignore-scripts` skips the download; run `npm rebuild vibeguard` afterwards.

Full documentation, including the permission model and the measured accuracy numbers:
https://github.com/lindenthomaslin/vibeguard
