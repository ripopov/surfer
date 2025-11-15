# Release procedure

## Pre release procedure

- [ ] Surfer
  - [ ] Update changelog
    - [ ] Copy content from changelog wiki to CHANGELOG.md
    - [ ] Update unreleased compare link to latest version
    - [ ] Make sure the version header links to the diff between this and the previous version
  - [ ] Bump Cargo.toml version
  - [ ] Build and add Cargo.lock

## Release

- [ ] Merge changelog update MRs
- [ ] Tag resulting commit as `vX.Y.Z` using an annotated tag
- [ ] Push tags
- [ ] Do a release on gitlab
- [ ] Upload Surfer release to zenodo
- [ ] Update release blog post MR with link to relevant changelog section. Merge blog

## Post release

- [ ] Announcements
  - [ ] Discord
  - [ ] Mastodon
