# Changelog

All notable changes to orbit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.6.2](https://github.com/doughlass/orbit/compare/v1.6.1...v1.6.2) - 2026-09-01

### Other

- reduce allocations in sort_items and iterator chains ([#69](https://github.com/doughlass/orbit/pull/69))

## [1.6.1](https://github.com/doughlass/orbit/compare/v1.6.0...v1.6.1) - 2026-08-31

### Fixed

- render all date columns in local time with UTC offset ([#67](https://github.com/doughlass/orbit/pull/67))

## [1.6.0](https://github.com/doughlass/orbit/compare/v1.5.0...v1.6.0) - 2026-08-31

### Added

- CloudWatch/Alarms, Secrets reveal, detail views + 31 new resources (merge master v1.5.0) ([#65](https://github.com/doughlass/orbit/pull/65))

## [1.5.0](https://github.com/doughlass/orbit/compare/v1.4.1...v1.5.0) - 2026-08-31

### Added

- Lambda function overview diagram, formatted details, and cross-service EventBridge enrich ([#63](https://github.com/doughlass/orbit/pull/63))

## [1.4.1](https://github.com/doughlass/orbit/compare/v1.4.0...v1.4.1) - 2026-08-29

### Fixed

- startup version check with --update, offering self-update via GitHub Releases ([#61](https://github.com/doughlass/orbit/pull/61))

## [1.4.0](https://github.com/doughlass/orbit/compare/v1.3.1...v1.4.0) - 2026-08-29

### Added

- Route53 multi-token pagination, S3 live progress, EKS sub-resources, table/filter improvements ([#59](https://github.com/doughlass/orbit/pull/59))

## [1.3.1](https://github.com/doughlass/orbit/compare/v1.3.0...v1.3.1) - 2026-08-29

### Other

- S3 download with live progress bar, Route53 pagination, table/filter improvements ([#57](https://github.com/doughlass/orbit/pull/57))

## [1.3.0](https://github.com/doughlass/orbit/compare/v1.2.0...v1.3.0) - 2026-08-26

### Added

- EBS volumes extended attribute columns
- arrow keys scroll tables; Tab cycles the sort column
- horizontal table scrolling when columns overflow
- CloudFront extended columns; fix dotted mapping key extraction
- column visibility picker with persisted preferences
- per-item list enrichment for EKS update history
- EKS update history and enhanced cluster describe
- EKS sub-resources — nodegroups, fargate profiles, add-ons
- formatted describe view with labelled key-value fields
- default AMI listing to owner=self instead of amazon
- show total record count in Route53 records header
- multi-token pagination for Route53 records

### Fixed

- block SSM connect in readonly mode
- show real record total instead of 100+ in Route53 title

### Other

- Revert "feat: arrow keys scroll tables; Tab cycles the sort column"
- pin horizontal scroll key handling end-to-end
- verify EKS sub-resources are wired in the registry

## [1.2.0](https://github.com/doughlass/orbit/compare/v1.1.2...v1.2.0) - 2026-08-24

### Added

- make --readonly the default, suppress destructive UI in readonly mode

### Other

- *(ci)* skip issue link check for author doughlass
- *(ci)* bump deprecated GitHub Actions versions

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
