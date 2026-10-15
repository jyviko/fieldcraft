# Releasing FieldCraft

Releases are deliberate. Development lands on `develop`, the trunk; `main` holds released code
only. Fast-forwarding `main` to the chosen, verified `develop` commit builds every package and
creates a **draft** GitHub Release; a maintainer verifies the draft and publishes it. Pushes to
`develop` and tags never start a release, and nothing bumps the version automatically. The workflow is `.github/workflows/release.yml`; the packaging scripts
are in `packaging/` and run the same way on a developer machine.

## What a release contains

| Platform | Files | Script |
|---|---|---|
| macOS 11+, universal (arm64 + x86_64) | `fieldcraft-<v>-macos-universal.dmg` (FieldCraft.app on a drag-to-Applications image), `fieldcraft-cli-<v>-macos-universal.zip` | `packaging/macos/package.sh` |
| Linux x86_64, glibc 2.35+ | `fieldcraft-<v>-linux-x86_64.tar.gz` (`bin/`, `share/`), `fieldcraft-<v>-linux-x86_64.AppImage`, `fieldcraft_<v>_amd64.deb` (Debian, Ubuntu 22.04+), `fieldcraft-<v>.x86_64.rpm` (Fedora, RHEL, openSUSE) | `packaging/linux/package.sh` |
| Windows x64 | `fieldcraft-<v>-windows-x64.zip` (`fieldcraft.exe`, `fieldcraft-cli.exe`) | `packaging/windows/package.ps1` |

Plus `SHA256SUMS.txt` and the notes from `.github/release-notes.md` followed by GitHub's
generated list of changes. Every package carries `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`,
`NOTICE` and `THIRD-PARTY-LICENSES.txt` (on macOS in `FieldCraft.app/Contents/Resources/Licenses`).
The application id is `io.github.jyviko.fieldcraft`: the macOS bundle id, the Linux `.desktop`
entry and icon name, and the window's Wayland app_id / X11 class. The icons come from
`assets/app-icon`: `icon.icns` and `Assets.car` (macOS 26 and later) in the app bundle,
`icon.ico` embedded in `fieldcraft.exe` (`apps/fieldcraft/build.rs`), the iconset PNGs as the
Linux hicolor icons.

The `.deb` and `.rpm` are built by `cargo-deb` and `cargo-generate-rpm` (versions pinned in
`release.yml`) from the same stripped release binaries as the tar.gz, with the same files under
`/usr` (`/usr/bin`, the desktop entry, the hicolor icons, the licences in
`/usr/share/doc/fieldcraft`); their metadata and file lists are in `apps/fieldcraft/Cargo.toml`.
The binaries link only glibc; the windowing, GPU and file-dialog libraries they open at run time
are listed as dependencies by hand (Debian package names, RPM soname capabilities). A
pre-release `0.2.0-rc.1` is versioned `0.2.0~rc.1` inside the packages so it sorts before
`0.2.0`; the file names keep the release version. The Linux job compares both file lists with
the tar.gz, runs lintian, and installs the packages in Ubuntu 22.04 and Fedora containers to run
`fieldcraft-cli --version`.

## Signing

**macOS.** When the repository has the secrets below, the app, the dmg and the CLI are signed
with a Developer ID (hardened runtime, `packaging/macos/entitlements.plist`) and notarized, and
the tickets are stapled. Without them the build is **ad-hoc signed**, which runs on Apple silicon
but which Gatekeeper blocks on first launch until the user right-clicks › Open; a run warns
about it, and the release notes say which of the two the draft contains.

| Secret | What it is |
|---|---|
| `APPLE_CERTIFICATE` | The "Developer ID Application" certificate with its private key, exported as .p12 and base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The .p12's password |
| `KEYCHAIN_PASSWORD` | Optional: the temporary CI keychain's password (random when unset) |
| `APPLE_ID` | The Apple ID used for notarization |
| `APPLE_PASSWORD` | An app-specific password for that Apple ID (appleid.apple.com › Sign-In and Security) |
| `APPLE_TEAM_ID` | The 10-character team id of the Developer ID |

Add them with `gh secret set <NAME> -R jyviko/fieldcraft`. With the certificate but without the
three notarization secrets the app is signed and not notarized. A dmg made locally with
`packaging/macos/package.sh` is ad-hoc signed unless `MACOS_SIGN_IDENTITY` names an identity
in the keychain.

**Linux and Windows** builds are not signed; the checksums are the integrity check. On Windows
SmartScreen warns about an unsigned download (More info › Run anyway) until a code-signing
certificate is added.

## Usage analytics key

The release builds (macOS, Linux and Windows jobs) build in the PostHog project key from the
repository secret `POSTHOG_KEY` as `FIELDCRAFT_POSTHOG_KEY` ([privacy.md](privacy.md)). Without the
secret the builds ask nothing and send nothing, as local and fork builds do. The web build on
Pages has no key.

## Choose and prepare the version

1. Look at the previous published release (`gh release list -R jyviko/fieldcraft`) and the
   changes since it, and choose one version for the batch: a patch for compatible fixes; a minor
   for new capabilities, and while on `0.x` also for breaking changes (say what breaks in the
   notes); `-rc.N` for a release candidate. `1.0` is the owner's decision.
2. Set `[workspace.package] version` in the root `Cargo.toml` and run
   `cargo update --workspace --offline`, which changes only the workspace crates' versions in
   `Cargo.lock`. Land it on `develop` through a pull request like any change, and let CI on
   `develop` pass. Note the commit to release.

## Build the draft

```sh
git fetch origin
git push origin <commit>:main      # fast-forward main to the verified develop commit
```

`main` holds released code only: it moves only by fast-forwarding to a verified `develop`
commit (a maintainer push; the branch is protected), never by merges, rewrites or force-pushes. The run builds the three platforms; when
all succeed, the last job checks that every file is there, writes `SHA256SUMS.txt` and the notes,
and creates the draft `FieldCraft v<version>` targeting the pushed commit. Pushing
`main` again with the same version rebuilds and replaces the draft's files (its notes are
kept). The job refuses a version whose tag exists or whose release is already published: bump
the version instead.

## Verify and publish

Publishing is a maintainer's call; an agent preparing a release stops at the draft.

1. Check the run succeeded for the intended commit and the draft targets it
   (`gh release view v<version> -R jyviko/fieldcraft`).
2. Download the assets into an empty folder (`gh release download v<version> -R jyviko/fieldcraft`),
   run `shasum -a 256 -c SHA256SUMS.txt`, and check every platform's files are there.
3. Smoke-test what you can run: open the dmg, drag the app to Applications, launch it with
   `--sample cantilever`; run `fieldcraft-cli --version` from the zip or tar.gz; on Linux run the
   AppImage, or install the `.deb` (`sudo apt install ./fieldcraft_<v>_amd64.deb`) or `.rpm`
   (`sudo dnf install ./fieldcraft-<v>.x86_64.rpm`); on Windows unzip and start `fieldcraft.exe`. Look at the run's logs for the
   platforms you could not try (each job checks its own package).
4. Edit the notes: what is new, fixes, breaking changes, known limitations. Keep the install and
   signing paragraphs; never claim signing the notes don't state.
5. Publish: `gh release edit v<version> -R jyviko/fieldcraft --draft=false --latest`
   (a release candidate: `--draft=false --prerelease --latest=false`). Publishing creates the tag
   at the draft's commit; never push release tags by hand.

## Dry runs

`gh workflow run release.yml -R jyviko/fieldcraft --ref <branch>` builds every package and
uploads them with the notes as the `release-assets` artifact of the run, without creating a
release. `-f draft=true` creates or updates the draft, and is accepted on `main` only.
Locally, `packaging/macos/package.sh --arch aarch64` (or `x86_64`, `universal`) and
`packaging/linux/package.sh` write to `dist/release/` (the `.deb` and `.rpm` need
`cargo install --locked cargo-deb cargo-generate-rpm` at the versions in `release.yml`;
`--formats "tar appimage"` skips them).
