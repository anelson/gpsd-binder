---
name: release
description: Make a numbered release of gpsd-binder. Sets the version, dates the changelog entry, commits, tags and pushes; the release workflow then builds the archives, creates the GitHub release and publishes the crate to crates.io. Use only when the user asks for a release.
argument-hint: "[version]"
disable-model-invocation: true
compatibility: Needs git, cargo, curl, and gh logged in to github.com.
---

# Release gpsd-binder

A release is one commit, `Release vX.Y.Z`, which sets the version in `Cargo.toml` and
`Cargo.lock` and dates the changelog entry, and an annotated tag `vX.Y.Z` on that commit. Pushing
the tag runs `.github/workflows/release.yml`, which builds both Android binaries, creates the
GitHub release with the archives and the changelog entry as its notes, and publishes the crate to
crates.io. Nothing else is done by hand.

The argument is the version, `X.Y.Z`, without the `v`. If there is no argument,
show the newest tag and the `## [Unreleased]` section of `CHANGELOG.md`, propose a version
(while the major version is 0, a breaking change raises the minor version and anything else the
patch version), and ask.

Rules:

- The user's request for a release is the instruction to commit, tag and push the release commit
  and its tag. It covers nothing else. Never force-push. Never move or delete a tag except as
  "If something fails" says, and then only after the user agrees.
- The commit message is exactly `Release vX.Y.Z`, with no body and no trailers.
- The changelog entry becomes the release notes, published under the user's name. The user must
  see it and accept it before the commit.
- Never ask for, store or use a crates.io API token. The first publication is the user's job
  ("First release" below).

## 1. Check the preconditions

Stop and report the problem if any check fails.

1. The current branch is `master`, and `git status --porcelain --untracked-files=no` prints
   nothing. Untracked files do not matter: the commit stages files by name, and `include` in
   `Cargo.toml` keeps them out of the crate.
2. After `git fetch origin --tags`, `git rev-list --left-right --count origin/master...HEAD`
   prints `0` and `0`. If local commits are not pushed yet, the user pushes them and CI runs on
   them first.
3. No tag `vX.Y.Z` exists locally or on `origin` (`git ls-remote --tags origin vX.Y.Z`).
4. The version is greater than the newest tag (`git tag --list 'v*' --sort=-v:refname`) and not
   less than `version` in `Cargo.toml`.
5. CI passed on `HEAD`:

   ```sh
   gh run list --workflow ci.yml --commit "$(git rev-parse HEAD)" \
     --json databaseId,status,conclusion
   ```

   A merge that GitHub Actions made, such as an auto-merged Dependabot pull request, starts no
   CI run. If `HEAD` has no run, start one with `gh workflow run ci.yml --ref master`, find its ID
   with the command above, and wait for it with `gh run watch <id> --exit-status`.
6. Ask crates.io about the crate, with a User-Agent that names the project:

   ```sh
   curl -sS -o /dev/null -w '%{http_code}\n' \
     -A 'gpsd-binder release (https://github.com/anelson/gpsd-binder)' \
     https://crates.io/api/v1/crates/gpsd-binder/X.Y.Z
   ```

   200 means that the version exists. That is correct only for the first release, after the
   user's hand publication; otherwise stop, because a crates.io version cannot be published
   twice. For 404, also ask for `https://crates.io/api/v1/crates/gpsd-binder`: if that is 404
   too, the crate has never been published, so stop and give the user the steps in
   "First release".

## 2. Prepare the release commit

1. If `Cargo.toml` does not already have the version, set `version` in `[package]`, then run
   `cargo update --workspace`. `git diff Cargo.lock` must show only the `gpsd-binder` entry; if
   anything else changed, stop.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` with today's date,
   and put a new, empty `## [Unreleased]` above it. If the Unreleased section was empty, draft
   entries from `git log --oneline <newest tag>..HEAD` (the whole log for the first release),
   grouped under `### Added`, `### Changed`, `### Fixed` and `### Removed` as Keep a Changelog
   does, in the plain style of the existing entries, and say that they are a draft.
3. Show the user the version, the new changelog section exactly as it will appear in the release
   notes, `git diff --stat`, and the commands in step 3. Wait for the user to accept it or to ask
   for changes. This is the only stop in a normal release.

## 3. Commit, tag and push

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "Release vX.Y.Z"
git tag -a vX.Y.Z -m "gpsd-binder X.Y.Z"
git push --atomic origin master vX.Y.Z
```

## 4. Watch the release workflow

The run can take a few seconds to appear. Find it and wait for it:

```sh
gh run list --workflow release.yml --event push --branch vX.Y.Z \
  --json databaseId --jq '.[0].databaseId'
gh run watch <id> --exit-status
```

## 5. Check the result

1. `gh release view vX.Y.Z --json url,assets --jq '.url, .assets[].name'` lists
   `gpsd-binder-X.Y.Z-x86_64-linux-android.tar.gz`,
   `gpsd-binder-X.Y.Z-aarch64-linux-android.tar.gz` and `SHA256SUMS`.
2. The crates.io request in step 1.6 now returns 200. The index can lag by a minute.
3. Report the release URL and `https://crates.io/crates/gpsd-binder/X.Y.Z`.

## If something fails

- The `build` job failed: nothing was published. Fix the cause with ordinary commits on master,
  wait for CI, and then, with the user's agreement, delete the tag
  (`git tag -d vX.Y.Z` and `git push --delete origin vX.Y.Z`), tag the new `HEAD`, and push the
  tag alone.
- The `github-release` or `crates-io` job failed: fix the cause if it is outside the repository
  (a setting, an outage), then `gh run rerun <id> --failed`. The `crates-io` job skips a version
  that crates.io already has, so a rerun is harmless.
- `crates-io` fails with an authentication error: the trusted publisher on crates.io does not
  match the workflow. It must name owner `anelson`, repository `gpsd-binder`, workflow
  `release.yml` and environment `release`. Only the user can change it.
- After crates.io has the version, never delete the tag or the GitHub release. Fix forward with
  a new patch release.

## First release

crates.io accepts a trusted publisher only for a crate that already exists, so the user publishes
the first version by hand, once:

1. Create a crates.io API token with the `publish-new` scope and a short expiry.
2. From a clean `master`, run `cargo publish --locked` with that token, then revoke the token.
3. On crates.io, open the crate's Settings, Trusted Publishing, and add a GitHub publisher:
   owner `anelson`, repository `gpsd-binder`, workflow `release.yml`, environment `release`.

Then run this skill for the same version. The version is already in `Cargo.toml`, so step 2 only
dates the changelog. The workflow creates the GitHub release, and its `crates-io` job finds the
version on crates.io and skips it. Every later version is published by the workflow.
