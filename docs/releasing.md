# Releasing Kitsu

Kitsu has one version for the whole product: the CLI, the library and the
desktop app. It follows [SemVer 2.0.0](https://semver.org/spec/v2.0.0.html)
and is computed from [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
by [release-please](https://github.com/googleapis/release-please). Nobody
edits a version by hand.

## Where the version lives

| File | Field |
| --- | --- |
| `crates/kitsu/Cargo.toml` | `package.version` (the source of truth) |
| `app/src-tauri/Cargo.toml` | `package.version` |
| `app/package.json`, `app/package-lock.json` | `version`, `packages[""].version` |
| `app/src-tauri/tauri.conf.json` | `version` |
| `.release-please-manifest.json` | `"."`, the last released version |
| `Cargo.lock` | the `kitsu` and `kitsu-app` entries, written by `cargo update --workspace` |

release-please bumps all but `Cargo.lock` in the release PR
(`release-please-config.json`, `extra-files`); the release workflow then runs
`cargo update --workspace` on the PR branch and commits the lockfile.
`.github/scripts/check-versions.sh` runs in CI and fails if any of these
disagree, and every cargo command in CI runs with `--locked`.

## How the next version is chosen

From the commits on `main` since the last release tag:

| Commit | Before 1.0.0 (now) | From 1.0.0 |
| --- | --- | --- |
| `feat!:`, `fix!:`, … or a `BREAKING CHANGE:` footer | minor: 0.2.0 → 0.3.0 | major |
| `feat:` | patch: 0.2.0 → 0.2.1 | minor |
| `fix:`, `perf:` | patch | patch |
| only `docs:`, `ci:`, `chore:`, `test:`, `refactor:`, `build:`, `style:` | no release | no release |

Before 1.0.0 this is Cargo's reading of SemVer (release-please's
`bump-minor-pre-major` and `bump-patch-for-minor-pre-major`): in `0.y.z`, `y`
plays the role of major and `z` of minor and patch, so `^0.2` accepts every
`0.2.z` and nothing breaking ever lands in one. Leaving 0.x is a decision,
not a side effect of a commit: put `Release-As: 1.0.0` in the body of a
commit on `main`. The same footer forces any other version.

The changelog lists features, bug fixes, performance work, reverts and
breaking changes; the other types are left out of it.

Because of this, pull request titles and commits must be Conventional
Commits (`<type>[(scope)][!]: <description>`). The `conventional commits`
workflow checks both on every pull request: the title is what a squash merge
commits, the commits are what a merge or rebase keeps.

## Builds that aren't releases

Every push to any branch, and every pull request from a fork, builds all
platforms once the checks pass. Those builds say which commit they are:

    kitsu --version
    kitsu 0.2.1-dev.14+g1a2b3c4

that is: the patch after the current version, `dev.<commits since the last
v* tag>`, and `g<short sha>`. SemVer orders it after 0.2.0 and before
whatever is released next. `.github/scripts/version.sh` computes it,
the workflow passes it as `KITSU_BUILD_VERSION`, and `crates/kitsu/build.rs`
bakes it in. Without that variable (any local build) or with a value that
isn't SemVer, `kitsu --version` prints the plain Cargo version; the build
never fails over it.

Installers carry the plain numeric version, dev build or not: Windows MSI
accepts only `major.minor.patch[.build]` with major and minor at most 255,
patch and build at most 65535, and Tauri rejects a pre-release that isn't
numeric. The dev version is in the artifact names instead
(`kitsu-app-0.2.1-dev.14+g1a2b3c4-windows-x86_64`) and in the CLI the app
embeds (`kitsu-app --kitsu-cli --version`, on Linux and macOS).

## Making a release

1. Merge pull requests into `main` as usual.
2. release-please keeps one pull request open, titled
   `chore(main): release X.Y.Z`, with the version bumps and the new
   `CHANGELOG.md` section. Read the changelog there; fix a wrong entry by
   rewording the commit or with a `BEGIN_COMMIT_OVERRIDE` block in the
   merged PR's description (see release-please's docs).
3. Merge it. release-please tags `vX.Y.Z` and publishes the GitHub
   Release with those notes. The same workflow run then runs the checks on
   the tag, builds every platform and attaches the files below plus
   `SHA256SUMS`. The release is public at once; its assets appear when
   that run finishes.

To rebuild and re-upload the assets of an existing release, run the
`release` workflow by hand (Actions → release → Run workflow) with the tag.

## Where the files are

Dev builds: the workflow run's page (Actions → ci → the run → Artifacts).
Branch builds are kept 7 days, `main` builds 30.

Releases: the GitHub Release for the tag.

| Platform | Desktop app | CLI (`kitsu`, `kitsu-test-agent`) |
| --- | --- | --- |
| Linux x86_64 | `.deb`, `.rpm`, `.AppImage` | `kitsu-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `.msi`, `-setup.exe` (NSIS) | `kitsu-X.Y.Z-x86_64-pc-windows-msvc.zip` |
| macOS | universal `.dmg` (Apple Silicon and Intel) | `kitsu-X.Y.Z-aarch64-apple-darwin.tar.gz`, `kitsu-X.Y.Z-x86_64-apple-darwin.tar.gz` |

Linux builds run on Ubuntu 22.04 so they work with glibc 2.35 and newer.

## Repository settings

- Settings → Actions → General: allow GitHub Actions to create and approve
  pull requests (release-please opens the release PR).
- Optional secret `RELEASE_PLEASE_TOKEN`: a fine-grained token with
  contents and pull requests read/write on this repository. GitHub starts no
  workflows for a pull request opened with the default token, so without it
  the release PR shows no checks. The tag and the release build don't need
  it: they run in the same workflow run.
- Branch protection on `main`: require `ci` and `conventional commits`.

## Signing

Builds are unsigned unless these secrets exist, and only release builds
receive them: branch builds never see signing keys.

macOS (Tauri reads them; see <https://v2.tauri.app/distribute/sign/macos/>):

| Secret | |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | its export password |
| `APPLE_SIGNING_IDENTITY` | optional; taken from the certificate |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarization with an Apple ID and an app-specific password |
| `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_P8` | or with an App Store Connect API key (`APPLE_API_KEY_P8` holds the `.p8` file's contents) |

Without a certificate the app gets an ad-hoc signature, which Apple Silicon
needs to launch it; Gatekeeper still asks the user to allow it (System
Settings → Privacy & Security).

Windows (see <https://v2.tauri.app/distribute/sign/windows/>):

| Secret | |
| --- | --- |
| `WINDOWS_CERTIFICATE` | base64 `.pfx` (`certutil -encode cert.pfx cert.txt`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | its export password |

and optionally the repository variable `WINDOWS_TIMESTAMP_URL` (default
`http://timestamp.digicert.com`). Certificates whose key lives in an HSM or
a cloud service can't be exported as a `.pfx`; those need Tauri's
`bundle.windows.signCommand` instead.

The CLI archives aren't signed on any platform.

## Why release-please

- **release-please**: a release is a reviewed pull request with the
  changelog and every version bump in it; generic JSON/TOML updaters cover
  Cargo, npm and Tauri files with one version; pre-1.0 bumping is
  configurable; maintained GitHub Action. It doesn't run cargo, so the
  lockfile gets its own step.
- **semantic-release**: publishes on every push without a review step,
  needs plugins and a commit-back to bump Cargo and Tauri files, and
  doesn't do 0.x: its first release is 1.0.0.
- **cargo-release + git-cliff**: the best Rust-native pair (proper
  lockfile handling, excellent changelogs), but driven from a developer's
  machine, the bump level is given rather than derived, and package.json and
  tauri.conf.json need replacement hooks. The PR flow would be ours to build.
- **knope**: handles Cargo and package.json together and has a PR flow, but
  a much smaller user base for something that tags releases.
