# Changelog

All notable changes to orbit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.2](https://github.com/doughlass/orbit/compare/v1.1.1...v1.1.2) - 2026-08-24

### Other

- release v1.1.1

## [1.1.1](https://github.com/doughlass/orbit/compare/v1.1.0...v1.1.1) - 2026-08-24

### Fixed

- *(ci)* add Accept header to homebrew formula downloads

### Other

- release v1.1.1
- rewrite README and CHANGELOG for orbit
- update copyright to doughlass
- add orbit copyright to LICENSE

## [1.1.0](https://github.com/doughlass/orbit/releases/tag/v1.1.0) - 2026-08-24

### Added
- ECR image listing: select a repository and view its images with tags, digest, push date and size
- ECR repository visibility column (Public/Private)
- Demo mode (`--demo`) with synthetic data for EC2, Route53 and CloudFront
- Resource count indicator shows `+` when more pages exist
- Homebrew formula with auto-update on each release
- Sortable table columns

### Fixed
- ECR `createdAt` dates formatted correctly (epoch seconds, not millis)
- Version number embedded in binary header via `CARGO_PKG_VERSION`
- Release pipeline vendored from upstream, no external dependencies
- Issue-link check skipped for release-plz PRs

### Changed
- Automated releases via release-plz (conventional commits → semver)
- Repository decoupled from upstream taws project

## [1.0.1](https://github.com/doughlass/orbit/releases/tag/v1.0.1) - 2026-08-23

### Added
- Multi-platform binary releases (macOS arm64/x86_64, Linux musl arm64/x86_64, Windows)
- Docker image published to `ghcr.io/doughlass/orbit`
- crates.io publishing (`cargo install orbit-tui`)

### Changed
- Renamed from taws to orbit
- 5-platform release builds via GitHub Actions
- Available via `brew tap doughlass/tap && brew install orbit`
