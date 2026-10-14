# Releasing FieldCraft

Releases are deliberate. Development lands on `main`; pushing the chosen commit to the `release`
branch builds every package and creates a **draft** GitHub Release; a maintainer verifies the
draft and publishes it. Pushes to `main` and tags never start a release, and nothing bumps the
version automatically. The workflow is `.github/workflows/release.yml`; the packaging scripts
are in `packaging/` and run the same way on a developer machine.

## What a release contains

| Platform | Files | Script |
|---|---|---|
| macOS 11+, universal (arm64 + x86_64) | `fieldcraft-<v>-macos-universal.dmg` (FieldCraft.app on a drag-to-Applications image), `fieldcraft-cli-<v>-macos-universal.zip` | `packaging/macos/package.sh` |
| Linux x86_64, glibc 2.35+ | `fieldcraft-<v>-linux-x86_64.tar.gz` (`bin/`, `share/`), `fieldcraft-<v>-linux-x86_64.AppImage` | `packaging/linux/package.sh` |
| Windows x64 | `fieldcraft-<v>-windows-x64.zip` (`fieldcraft.exe`, `fieldcraft-cli.exe`) | `packaging/windows/package.ps1` |

Plus `SHA256SUMS.txt` and the notes from `.github/release-notes.md` followed by GitHub's
generated list of changes. Every package carries `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`,
`NOTICE` and `THIRD-PARTY-LICENSES.txt` (on macOS in `FieldCraft.app/Contents/Resources/Licenses`).
The application id is `io.github.jyviko.fieldcraft`: the macOS bundle id, the Linux `.desktop`
entry and icon name, and the window's Wayland app_id / X11 class. The icons come from
`assets/app-icon`: `icon.icns` and `Assets.car` (macOS 26 and later) in the app bundle,
`icon.ico` embedded in `fieldcraft.exe` (`apps/fieldcraft/build.rs`), the iconset PNGs as the
Linux hicolor icons.

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

## Choose and prepare the version

1. Look at the previous published release (`gh release list -R jyviko/fieldcraft`) and the
   changes since it, and choose one version for the batch: a patch for compatible fixes; a minor
   for new capabilities, and while on `0.x` also for breaking changes (say what breaks in the
   notes); `-rc.N` for a release candidate. `1.0` is the owner's decision.
2. Set `[workspace.package] version` in the root `Cargo.toml` and run
   `cargo update --workspace --offline`, which changes only the workspace crates' versions in
   `Cargo.lock`. Land it on `main` through a pull request like any change, and let CI on `main`
   pass. Note the commit to release.

## Build the draft

```sh
git fetch origin
git switch release                 # the first time: git switch -c release origin/main
git merge --ff-only origin/main
git push origin release
```

Only fast-forward `release` and never force-push it. The run builds the three platforms; when
all succeed, the last job checks that every file is there, writes `SHA256SUMS.txt` and the notes,
and creates the draft `FieldCraft v<version>` targeting the pushed commit. Pushing
`release` again with the same version rebuilds and replaces the draft's files (its notes are
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
   AppImage; on Windows unzip and start `fieldcraft.exe`. Look at the run's logs for the
   platforms you could not try (each job checks its own package).
4. Edit the notes: what is new, fixes, breaking changes, known limitations. Keep the install and
   signing paragraphs; never claim signing the notes don't state.
5. Publish: `gh release edit v<version> -R jyviko/fieldcraft --draft=false --latest`
   (a release candidate: `--draft=false --prerelease --latest=false`). Publishing creates the tag
   at the draft's commit; never push release tags by hand.

## Dry runs

`gh workflow run release.yml -R jyviko/fieldcraft --ref <branch>` builds every package and
uploads them with the notes as the `release-assets` artifact of the run, without creating a
release. `-f draft=true` creates or updates the draft, and is accepted on `release` only.
Locally, `packaging/macos/package.sh --arch aarch64` (or `x86_64`, `universal`) and
`packaging/linux/package.sh` write to `dist/release/`.
