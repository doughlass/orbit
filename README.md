```
█▀█ █▀█ █▄▄ █ ▀█▀
█▄█ █▀▄ █▄█ █  █
```

# orbit — Terminal UI for AWS

Browse, observe and manage your AWS resources from the terminal. Keyboard-driven, vim-style navigation. Built for operators who live in the shell.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)
[![Crates.io](https://img.shields.io/crates/v/orbit-tui.svg)](https://crates.io/crates/orbit-tui)

<p align="center">
  <img src="assets/screenshot-ec2.png" alt="EC2 Instances" width="800"/>
</p>

<p align="center">
  <img src="assets/screenshot-r53.png" alt="Route53 Hosted Zones" width="800"/>
</p>

## Install

```bash
brew tap doughlass/tap && brew install orbit   # macOS / Linux (Homebrew)
cargo install orbit-tui                         # via crates.io
docker run --rm -it -v ~/.aws:/root/.aws:ro ghcr.io/doughlass/orbit   # Docker
```

The crate is named `orbit-tui` (the name `orbit` was taken on crates.io). The installed binary is `orbit`.

Pre-built binaries for macOS (arm64/x86_64), Linux (musl arm64/x86_64) and Windows (x86_64) are available on the [releases page](https://github.com/doughlass/orbit/releases).

### From source

Requires Rust 1.94+ and a C compiler.

```bash
git clone https://github.com/doughlass/orbit.git && cd orbit
cargo build --release
./target/release/orbit
```

## Quick start

```bash
orbit                               # default profile and region
orbit --profile prod                # specific profile
orbit --region eu-west-1            # specific region
orbit --readonly                    # block all write operations
orbit --demo                        # synthetic data, no AWS connection
orbit --demo all                    # EC2 + Route53 + CloudFront demo
orbit --demo ec2-instances,rds      # choose resources to demo
orbit --log-level debug             # write debug log
```

Shell completions for bash, zsh and fish:

```bash
eval "$(orbit completion bash)"     # add to ~/.bashrc
eval "$(orbit completion zsh)"      # add to ~/.zshrc
orbit completion fish | source      # add to config.fish
```

## Features

- **60+ resource types** across 32 AWS services
- **Keyboard-driven** — vim keys, `:` command mode, `/` filtering
- **Fuzzy filtering** — client-side, or server-side AWS tag filters where supported
- **Sortable columns** — click a column header with `j`/`k` to sort
- **Resource actions** — start, stop, terminate EC2 instances, view secret values, SSM shell connect
- **Data-driven** — every resource type is a JSON definition, no hardcoded keys in Rust
- **Multi-profile, multi-region** — SSO, role assumption, console login, credential chain
- **Demo mode** — `--demo` starts instantly with synthetic data for screenshots or testing
- **Read-only mode** — `--readonly` blocks all mutating operations
- **Pagination** — large resource lists with `]`/`[` keys

## Key bindings

| Key | Action |
|-----|--------|
| `j` `k` `↑` `↓` | Navigate items |
| `gg` `G` | Jump top / bottom |
| `PgUp` `PgDn` `Ctrl+b` `Ctrl+f` | Page up / down |
| `]` `[` | Next / previous page |
| `Enter` | Describe resource or enter sub-resource |
| `d` | Describe selected resource |
| `:` | Open resource picker |
| `/` | Filter (fuzzy match or AWS tag filters) |
| `J` `K` | Sort by column |
| `R` | Refresh |
| `0`–`5` | Quick region switch |
| `?` | Help |
| `Ctrl+c` | Quit |
| `Esc` `Backspace` | Go back |

Resource-specific actions appear in the help screen (`?`). Examples: `c` connects to EC2 via SSM, `x` reveals a secret value, `i` lists ECR images.

## Supported services

| Category | Service | Resources |
|----------|---------|-----------|
| **Compute** | EC2 | Instances, Volumes, Snapshots, AMIs |
| | Lambda | Functions |
| | ECS | Clusters, Services, Tasks |
| | EKS | Clusters |
| | Auto Scaling | Auto Scaling Groups |
| **Storage** | S3 | Buckets, Objects |
| **Database** | RDS | Instances, Snapshots |
| | DynamoDB | Tables |
| | ElastiCache | Clusters |
| | Redshift | Clusters |
| **Networking** | VPC | VPCs, Subnets, Security Groups, Network ACLs, Route Tables, Internet Gateways, NAT Gateways, Elastic IPs, Network Interfaces, VPC Endpoints, VPC Peering Connections |
| | ELBv2 | Load Balancers, Listeners, Rules, Target Groups, Targets |
| | Route 53 | Hosted Zones, Records |
| | CloudFront | Distributions |
| | API Gateway | REST APIs |
| **Security** | IAM | Users, Groups, Roles, Policies, Access Keys |
| | Secrets Manager | Secrets |
| | KMS | Keys |
| | ACM | Certificates |
| | Cognito | User Pools |
| | WAFv2 | Web ACLs, IP Sets, Rule Groups |
| **Management** | CloudFormation | Stacks, Stack Events, Stack Outputs |
| | CloudWatch | Log Groups, Log Streams |
| | CloudTrail | Trails |
| | SSM | Parameters |
| **Messaging** | SQS | Queues |
| | SNS | Topics |
| | EventBridge | Event Buses, Rules |
| **Containers** | ECR | Repositories, Images |
| **DevOps** | CodePipeline | Pipelines |
| | CodeBuild | Projects |
| **Analytics** | Athena | Workgroups |
| | MSK | Clusters |

> Missing a service? [Open an issue](https://github.com/doughlass/orbit/issues/new).

## Authentication

orbit uses the standard AWS credential chain: environment variables → SSO → console login → `~/.aws/credentials` → `~/.aws/config` → IMDSv2.

For SSO: if your profile uses Identity Center and the token is expired, orbit prompts you to authenticate. If you already logged in via `aws sso login`, the cached token is reused.

For role assumption: `role_arn` with `source_profile` or `credential_source` (EC2, ECS, Environment) is fully supported.

Set `AWS_CA_BUNDLE` if you are behind a corporate proxy with SSL inspection.

For LocalStack: `orbit --endpoint-url http://localhost:4566`.

## Configuration

| Variable | Purpose |
|----------|---------|
| `AWS_PROFILE` | Default profile |
| `AWS_REGION` | Default region |
| `AWS_ENDPOINT_URL` | Custom endpoint (LocalStack) |
| `AWS_CA_BUNDLE` | Corporate SSL certificate bundle |

Logs: `~/Library/Application Support/orbit/orbit.log` (macOS), `~/.config/orbit/orbit.log` (Linux).

## Contributing

Contributions welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). Before adding a new AWS service, please [open an issue](https://github.com/doughlass/orbit/issues/new) first.

## Acknowledgments

Originally forked from [taws](https://github.com/huseyinbabal/taws) by Hüseyin Babal. Built with [Ratatui](https://github.com/ratatui-org/ratatui). Inspired by [k9s](https://github.com/derailed/k9s).

## License

Licensed under MIT. See [LICENSE](LICENSE) for details.

orbit is not affiliated with or endorsed by Amazon Web Services, Inc. "AWS" is a trademark of Amazon.com, Inc.
