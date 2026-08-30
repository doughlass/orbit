# Resource coverage backlog

Goal: bring Orbit's browseable resources up to cover the AWS services a
k9s-style browser should. Effort is multi-session, so this file is the durable
source of truth for what's done and what's next. **One logical change per
commit; verify each resource against the real wire format before committing.**

Per AGENTS.md:
- New services need a `ServiceDefinition` entry in `src/aws/http.rs`.
- New resources are pure JSON under `src/resources/` (data, not code).
- Every new resource: TDD a registry invariant, confirm the `response_root`
  against a real response, check pagination support in the botocore model.
- Nothing is committed without `cargo test` + `cargo clippy --all-targets -- -D
  warnings` + `cargo fmt --check` green.

## Covered today

See `src/resources/*.json`. Roughly 32 services, 80+ resource views.

## Ordering (do top-down)

### Session 1 (high value, on deck)

- [x] `monitoring` service entry + **CloudWatch Alarms** (`DescribeAlarms`).
      AGENTS.md already flagged this as missing. The service entry name
      (`monitoring`) differs from the resource service key; both were added.
      **Needed a Rust change:** CloudWatch's "Granite" JSON endpoint only
      answers requests carrying `x-amzn-query-mode: true` +
      `application/x-amz-json-1.0`; without it the endpoint returns an XML
      `<UnknownOperationException/>`. Verified live (404 without, 200 with).
- [x] **CloudWatch Dashboards** (`ListDashboards`). Reuses the monitoring
      query-mode target; `DashboardEntries` is the list element.
- [x] **RDS Aurora clusters** (`DescribeDBClusters`). AGENTS.md: only
      `DescribeDBInstances` was wired today. Wire confirmed
      `DescribeDBClustersResult/DBClusters` wrapper; tags on `TagList`. Live
      account has zero clusters (verified `<DBClusters/>` empty) but the
      request returns 200.
- [x] **CloudTrail LookupEvents** (event-history search). The `cloudtrail`
      service + `cloudtrail-trails` already existed, so this was just the
      `cloudtrail-events` resource — a flat `/Events` array paging on NextToken.
      `EventTime` is ISO-8601 in this JSON API (not epoch millis). Live account
      has real events; verified 200.
- [x] **Lambda layers / aliases / versions** (functions only before). Layers is a
      standalone `lambda-layers`; aliases/versions are parent-scoped children of
      `lambda-functions` (`v`/`a` shortcuts, `{functionName}` path placeholder
      mirrored from EKS). **Trap:** `ListLayers` lives under the `2018-10-31`
      API while functions/aliases/versions are `2015-03-31`; using the wrong
      version makes Lambda answer `AccessDenied: unable to determine
      operation`. Layers verified live 200; aliases/versions paths CLI-verified.
- [ ] **S3 bucket policies / lifecycle / replication** (buckets + objects only).

### Session 2

- [x] **EFS** (file systems, access points, mount targets). REST-JSON GET service
      (`elasticfilesystem` signing). Access points and mount targets are
      parent-scoped under a file system, but EFS scopes them by a URI **query
      parameter** (`?FileSystemId=...`), not a path segment — so this added a
      new `api_config.query_params` capability to the rest-json handler. Mount
      targets genuinely need it: `DescribeMountTargets` refuses to run with no
      filter at all. File systems verified live 200 (account empty).
- [x] **FSx** (file systems). Plain JSON-RPC service under target
      `AWSSimbaAPIService_v20180301`, content-type 1.1 (orbit default, no
      special case). Trap: `DescribeFileSystems` caps `MaxResults` at 50 — a
      copy of a neighbour's 100 is out of bounds. FSx has no native Name field,
      so the name column reads `Tags.Name` via `tags_to_map`. Live verified 200
      (account empty).
- [x] **Step Functions** (state machines, executions). Added the `states`
      service (plain `AWSStepFunctions` JSON target — **no** query-mode header,
      unlike CloudWatch). Trap: the endpoint returns an XML
      `UnknownOperationException` unless the request arrives as
      `application/x-amz-json-1.0` (not 1.1). Executions are a parent-scoped
      JSON child passing `stateMachineArn` in the body. Live verified 200.
- [x] **EventBridge Scheduler** (schedules). Plain REST-JSON GET service
      (`GET /schedules`, no `X-Amz-Target` — unlike the old `events` JSON-RPC
      service). Schedules list account-wide; the summary shape carries no cron
      expression (that is a GetSchedule describe call), so the columns are
      Name/State/Group/TargetArn/LastModificationDate. Live verified 200 (account
      empty).
- [x] **API Gateway v2** (HTTP/WebSocket APIs). Shares the existing
      `apigateway` service entry — the v2 CLI is the same REST-JSON GET with the
      same host and signing name, and `api_version` only feeds the query
      protocol's `Version` param. The only thing that differs is the request
      path (`/v2/apis`), which lives in the resource JSON, not the service
      table. `GetApis` returns `Items` + `NextToken`. Live verified 200 (account
      empty).
- [x] **ECS task definitions**. The classic scalar-string list: `ListTaskDefinitions`
      returns bare ARN strings, not objects, so the empty source maps each string
      directly (the DynamoDB table-names pattern). Added the `taskdef_arn_name` /
      `taskdef_arn_family` / `taskdef_arn_revision` transforms to split
      `task-definition/<family>:<rev>` out of the ARN, keeping the full ARN as the
      mapped id. Reuses the existing `ecs` service entry (its target prefix is
      already `AmazonEC2ContainerServiceV20141113`). Live verified 200 against 6
      real task definitions.

### Session 3

- [x] **GuardDuty** (detectors). Detectors are a bare-ID
      list (`GET /detector` → `DetectorIds`), so it maps like the ECS
      task-definition scalar list. Because a bare-string id cannot be extracted
      for the `{resource_id}` describe path, detectors have no describe (same
      limit as the ECS task-definition resource). Live verified 200 against the
      one real detector.
- [x] **Security Hub** (standards). Standards are a clean
      REST-JSON GET (`GET /standards` → `Standards`), no X-Amz-Target — live
      verified 200 against real standards. New `securityhub` service.
- [x] **Macie**, **Inspector**. Both services' useful lists are **POSTs whose
      pagination lives in the JSON body** (`maxResults`/`nextToken`), which is
      exactly the rest-json handler gap that deferred GuardDuty and Security Hub
      findings. Rather than a new flag, the handler now mirrors the JSON
      protocol's body-pagination for POSTs and, crucially, skips any param
      consumed by a `{key}` path placeholder — so a parent `DetectorId` that
      scopes a `/{DetectorId}` path segment is no longer leaked into the payload
      (GuardDuty `ListFindings` rejects it there). This unblocks
      **Inspector2 findings** (`POST /findings/list`, new `severity` colour map)
      and **Macie2 classification jobs** (`POST /jobs/list`, new `macie2` &
      `inspector2` service entries). Macie2 live verified 200. GuardDuty/Security
      Hub findings are no longer blocked by the handler.
- [ ] **Shield**, **WAF classic (v1)**.
- [ ] **Key pairs, launch templates, placement groups, dedicated hosts** (EC2).

### Later sessions (long tail)

- [ ] Kinesis (streams, delivery streams), Data Firehose.
- [ ] AppSync, MQ, Timestream, QLDB, DocumentDB, Neptune.
- [ ] Glue, EMR, DataSync, Transfer Family.
- [ ] Global Accelerator, App Mesh, Cloud Map, VPC Lattice, Route53 Resolver.
- [ ] Backup (vaults/plans), Resource Groups, Service Quotas.
- [ ] Cost Explorer, Budgets, Trusted Advisor, Health.
- [ ] Config rules/compliance, X-Ray, Systems Manager fleet ops (Run Command /
      Patch / State Manager / Sessions).
- [ ] SSO / Identity Center (permission sets, accounts, assignments).
- [ ] Cognito identity pools, user pool clients/domains.
- [ ] IAM SAML/OIDC providers, instance profiles, credential report.
- [ ] KMS aliases, key policies, grants, rotation config.
- [ ] SNS subscriptions, SQS DLQ/redrive.
- [ ] Elasticache parameter groups/subnets/snapshots; Redshift parameter
      groups/reserved nodes; RDS parameter groups/event subscriptions.

## Known structural blockers

- Two-token pagination (e.g. Route53 health checks / some List* ops) needs a
  Rust change in `PaginationConfig`, not a JSON one.
- Windows self-update asset (zip) not read by the tar extractor.
- Column widths are fixed percentages; content-aware autofit unimplemented.
