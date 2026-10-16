# Publishing Strata Hub

## Prepare the files

Run from the maintained source folder with stable Rust MSVC, Microsoft C++ Build Tools, Windows SDK and PowerShell 7 installed:

```powershell
pwsh -NoProfile -File .\Build-Package.ps1
```

The generated `dist/<timestamp>/` contains:

| Item | Destination |
| --- | --- |
| `strata-hub-source/` | Git repository root |
| `strata-hub-source.zip` | Optional source archive |
| `strata-hub-windows-x64.zip` | Windows release attachment |
| `checksums.json` | SHA-256 for the packaged executable |

Use the clean source folder for the initial Git import. Do not upload a local development workspace or its entire `dist` folder. A development workspace may hold private backups, deployment records and client credentials even when `.gitignore` excludes them.

The source package includes Rust and helper sources, focused tests, public logo assets, relative-path settings examples, build instructions and Apache-2.0 license files. Every source path is listed explicitly in `Build-Package.ps1`; update that list when adding a publishable source file. `THIRD_PARTY_NOTICES.txt` is generated from locked Cargo dependencies and is included in both packages. Strata and agent software are downloaded separately, not bundled.

The packaging checks reject recognizable provider credentials, private keys and personal build paths in either package folder, including the Windows executable. They report filenames without exposing matching values. Review examples and changes as well: arbitrary secrets cannot be recognized reliably by patterns alone. Synthetic test values (`fixture`, `fixture-key`, `fixture-only`) and localhost test URLs are intentional.

Never include real `strata-*.json` profiles, hub settings/drafts, `model-prepare-template.json`, agent configuration/backups, environment files, logs, usage databases, model weights, personal launchers or debug symbols. Keep configuration, history and API keys in the private data locations selected by each user. Release ZIPs contain neither your settings nor your model files.

## Review the first Git import

Open a terminal in the clean `strata-hub-source` folder:

```powershell
git init
git add .
git status --short
git diff --cached --stat
git diff --cached
```

Review the staged filenames and text before committing. No remote, account name or personal email is supplied by this project. Choose your Git identity and repository visibility, then create the remote repository on GitHub when ready. Add your own remote URL and push only after reviewing this staged import.

## Release behavior

The Windows app uses native Win32/GDI and SQLite. It requires no Rust installation, browser or WebView at runtime. Strata installation/update and model operations remain explicit. The installer generates a local API key on the destination PC; existing users retain their own credentials. Agent synchronization modifies only existing generic local Strata connections and keeps backups in private runtime data.

The packages are not code-signed. The source archive contains no Git history. Keep both `LICENSE` and generated dependency notices when distributing the packages.
