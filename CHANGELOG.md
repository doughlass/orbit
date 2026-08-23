# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.2](https://github.com/doughlass/orbit/compare/v1.0.1...v1.0.2) - 2026-08-23

### Fixed

- *(ci)* inject ORBIT_VERSION so header shows release version

### Other

- *(ci)* remove release_always after first publish

## [1.0.0](https://github.com/doughlass/orbit/releases/tag/v1.0.0) - 2026-08-23

### Added

- *(ci)* automate version bumps with release-plz
- browse S3 buckets and download objects with Enter
- add MSK cluster listing
- show alternate domains and origins for CloudFront distributions
- add copy-to-clipboard hotkey for SSM parameter and secret values
- view ssm parameter values
- add Redshift clusters/snapshots support and fix S3 object navigation
- *(credentials)* add console login support
- show resource-specific actions in help popup
- add AMI support with unified Filters: syntax
- add server-side tag filtering for resources that support it
- Support custom CA bundle for corporate SSL inspection
- Support credential_source for ECS/EC2/Environment credentials
- Support role_arn and source_profile for IAM role assumption
- Add SSM Connect for EC2 instances
- Add PageUp and PageDown keyboard support
- Data-driven action and describe operations
- Migrate ECS, EKS, KMS, CodeBuild to data-driven dispatch
- Migrate 30+ resources to data-driven dispatch
- Migrate SSM, ECR, ACM, EventBridge, CloudWatch to data-driven dispatch
- Add data-driven SDK dispatch infrastructure
- ESC Support

### Fixed

- *(ci)* rename git_release to git_release_enable
- *(ci)* use release-plz/action@v0.5 not v0.6
- *(ci)* drop crates.io publishing from release-plz
- *(ui)* stop the header tagline clipping on narrow terminals
- *(ci)* give the branch cleanup workflow permission to delete
- satisfy clippy on the SSO WaitingForAuth pattern
- linting issues
- linting issues from clippy
- remove unused fetch_resources function to fix clippy
- add pagination support for EC2 instances listing
- use filter_type for sub-resource filtering
- show resource-specific actions at the top of help popup
- address clippy warning for manual strip_prefix
- prevent direct access to sub-resources and fix serialization error
- improve truncation and add line wrap to Describe view
- dynamically calculate column width for truncation
- revert hover behavior and add line wrap to Describe view
- truncate long names from beginning and show full on hover
- pressing '/' clears tag filter and starts fresh
- remove tag filter from Auto Scaling (uses different filter format)
- respect AWS_CONFIG_FILE env var for custom config paths
- Handle certificates with unsupported critical extensions in CA bundle
- Support AWS CLI v2 SSO cache and CLI assume-role cache
- Respect AWS_ENDPOINT_URL and AWS_CONFIG_FILE for role assumption
- Support AWS_SHARED_CREDENTIALS_FILE environment variable
- Address clippy warnings
- fix message grammar
- add semicolon
- fix merge
- fixed fuzzy search score
- fixed copy/paste
- fix sso modal event
- cache aws credentials per profile
- fix typo in readme
- fix ecs service pagination
- fix pagination
- fix s3 400 error
- fix crate lock file in gh actions
- use rustls-tls and remove unused service definitions

### Other

- vendor workflows, cut upstream taws ties
- set version to 1.0.0, drop inherited author, reword taglines
- stop the AWS_LOGIN_CACHE_DIRECTORY tests racing each other
- run cargo audit on dependency changes and weekly
- *(deps)* patch two high-severity quick-xml advisories and refresh the tree
- turn off the CodeQL scan until the repo can upload results
- Merge pull request #6 from doughlass/fix/resource-pagination
- Use the orbit ring artwork on the splash screen
- Redraw the splash screen wordmark as ORBIT
- Rename the project from taws to orbit
- list MSK in the supported services table
- pin MSK cluster state colour mappings
- Merge pull request #169 from derrike/feat/copy-to-clipboard-ssm-secrets
- bump cargo version
- add test for ssm param
- linting
- created a record_id and added unit tests
- linting
- add subresource Route53 RecordSets
- bump version to 1.3.0-rc.7
- bump version to 1.3.0-rc.6
- *(console-login)* mirror SSO flow structure
- bump version to 1.3.0-rc.5
- consolidate Modes and Common Commands into General section
- make help popup fully context-aware
- bump version to 1.0.0-rc.4
- bump version to 1.3.0-rc.3
- bump version to 1.3.0-rc.2
- undo direct commits to master (will recreate as PR)
- bump version to 1.3.0-rc.1
- add tag filtering documentation to README
- bump version to 1.2.1
- *(deps)* bump quick-xml from 0.38.4 to 0.39.0
- bump version to 1.2.0
- Add dynamic shell completion for --profile and --region
- bump version to 1.2.0-rc.7
- Update README with EBS Volumes and Snapshots
- Add EBS Snapshots and EBS Volumes support
- bump version to 1.2.0-rc.6
- Add shell completion support for bash, zsh, fish, and PowerShell
- bump version to 1.2.0-rc.5
- Add search functionality for resource description view
- Add PageUp/PageDown support for resource description view
- bump version to 1.2.0-rc.4
- Merge pull request #91 from ldziedziul/view-secrets
- Fix formatting
- Use 'x' on a secret to retrieve and display its decrypted value
- Add view secret value action for secrets manager
- bump version to 1.2.0-rc.2
- Update CONTRIBUTING.md for data-driven architecture
- Clean up naming - remove 'data_driven' suffix
- Remove legacy fallback functions from dispatch
- Merge sdk_dispatch and data_driven_dispatch into single dispatch module
- Clean up sdk_dispatch.rs - remove migrated list operations
- Fix region shortcuts to work as LRU cache with defaults
- Fix workflow permissions for security scan
- Extend filter to search all column values (attributes)
- Add confirmation dialogs for start and rotate actions
- - fixed global services for ESC
- update readme for musl linux builds
- update readme for docker
- add dockerfile
- bump version to v1.1.7
- add reusable actions
- combine all Dependabot dependency updates
- handle pr reviews & bump version to 1.1.5
- Merge pull request #49 from stevepapa/feature/fuzzy-search
- bump version to 1.1.4
- least recently used regions added
- lambda pagination fixed
- added more logs to explain config events
- Update FUNDING.yml to remove comment
- bump version to 1.1.3
- Merge pull request #42 from stevepapa/fix/cloudwatch-timestamp-formatting
- bump version to 1.1.2
- improve pagination
- added ci improvements & funding
- bump version to 1.1.1
- bump version to 1.1.0
- Add new feature to restart the servers
- Add new feature to restart the servers
- basic cloudwatch logs added
- add sso support
- add imdsv2 login readme
- add imdsv2 login
- added elbv2 support
- minio is not an option
- added custom endpoint url to work with localstack
- delete stale branches
- add scoop bucket support
- add readonly mode
- generate cargo lock file
- bump version to 1.0.1
- update readme for build dependencies
- added better resources handling
- added logfile descriptions
- added crates flow
- update dependencies
- edition fix
- update Cargo for crates
- update readme
- inject version
- added lambda screenshots
- ec2 improvements
- add CONTRIBUTING.md and streamline README
- update code
- update gh actions
- goreleaser deps
- goreleaser added
- update readme
- update readme
- upgrade dependencies
- Merge pull request #5 from huseyinbabal/dependabot/cargo/dirs-6.0.0
- Merge pull request #2 from huseyinbabal/dependabot/github_actions/actions/checkout-6
- Merge pull request #3 from huseyinbabal/dependabot/github_actions/actions/download-artifact-7
- Bump actions/download-artifact from 4 to 7
- Initial commit with release workflow and dependabot
