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
- [x] **S3 bucket policies / lifecycle / replication** (buckets + objects only).
      All three are per-bucket children reachable from the buckets table
      (`l`/`r`/`p` shortcuts, scoped via the bucket row's `Name`). Trap: the
      three config documents speak three different wire formats from the same
      bucket row. `GetBucketPolicy` returns the **raw JSON policy document**, not
      an XML wrapper, so `s3-bucket-policy` is the one S3 resource that goes
      through the **rest-json** handler (its `xml_to_json` step would corrupt
      the body) and lists each `Statement` as a row. Lifecycle and replication
      are ordinary XML lists (`/LifecycleConfiguration/Rule`,
      `/ReplicationConfiguration/Rule`). Live verified: lifecycle + policy 200
      against real configs, replication correctly 404s in an account with none.

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
- [x] **Shield**, **WAF classic (v1)**. Both are classic AWS JSON-RPC
      (`X-Amz-Target` POST /) so they slot into the existing json handler with a
      target prefix — no new capability. Shield (`AWSShield_20160616`) is served
      from a regional `shield.<region>` host and offers **protections** +
      **protection groups** (`NextToken`/`MaxResults` body pagination). Classic
      WAF (`AWSWAF_20150824`) is the **global region-less shape**: its host is
      `waf.amazonaws.com` (never a region, same as IAM), so the new `waf`
      service entry is `is_global: true` and the signer resolves it to us-east-1
      automatically. WAF paginates with `NextMarker`/`Limit` — the one
      `input_token` in the codebase that is not `NextToken` — and offers
      **web ACLs**, **rules**, and **IP sets**. Live verified 200 on all five
      resources; protections and web ACLs have real rows.
- [x] **Key pairs, launch templates, placement groups, dedicated hosts** (EC2).
      Four more blocks in `ec2.json`, all with the same query protocol. Two wire
      facts pin them: `DescribeLaunchTemplates` returns a `<launchTemplates>`
      wrapper (no `<...Set>`), while the other three keep Set-style roots — and
      key pairs + placement groups take **no** `NextToken`/`MaxResults` at all
      (the `DescribeAddresses` trap, so they deliberately omit the pagination
      block), while launch templates + hosts do paginate. Host `InstanceType`
      is nested at `/hostProperties/instanceType`. Layout: launch templates and
      placement groups verified against real rows; key pairs real; hosts empty
      (account has none) but the request returns 200.

### Later sessions (long tail)

- [x] Kinesis (streams, delivery streams), Data Firehose. Both services are
      plain JSON-RPC (X-Amz-Target header, 1.1 content type). `kinesis-streams`
      reads the modern `ListStreams` `NextToken`-paginated `StreamSummaries`
      (status colour, stream mode, created). **Firehose** is the scalar-list
      pattern again: `ListDeliveryStreams` returns bare name strings, and its
      pagination marker (`ExclusiveStartDeliveryStreamName` = last-returned
      name) cannot be derived by a path extractor, so it sends `Limit=100`
      with no token loop — a page-max-only pagination block. Live verified 200
      (kinesis empty, firehose has three real streams).
- [x] AppSync + Amazon MQ. Both rest-json GETs with JSON list roots
      (`/v1/apis` → `/graphqlApis`, `/v1/brokers` → `/BrokerSummaries`). Trap:
      the two page on opposite token casing — AppSync camelCase
      (`nextToken`/`maxResults`), MQ PascalCase (`NextToken`/`MaxResults`) —
      pinned by test since a swapped case silently never pages. Live verified
      200 (both accounts empty of these resources).
- [x] DocumentDB + Neptune clusters/instances. RDS-family Query protocol: both
      are served from `rds.<region>.amazonaws.com` with signing_name `rds`, so
      their service entries reuse the RDS endpoint/signing identity rather than
      inventing a non-existent `docdb.<region>`/`neptune.<region>` host — pinned
      by test. Wire roots and actions are identical to RDS's
      (`DescribeDBClustersResponse/.../DBClusters`, etc.). Live verified 200 on
      all four (both accounts empty).
- [ ] Timestream (databases/tables) + QLDB ledgers — **blocked for live
      verification**: the target account is not enabled for Timestream
      (`AccessDeniedException` on DescribeEndpoints), and QLDB is not bundled
      in this awscli's botocore model, so a wrong `response_root` cannot be
      caught. Implement only once both can be confirmed against a real response.
- [x] Glue (jobs, databases, crawlers, triggers) + EMR clusters. Both JSON
      protocol. Glue pages every list on `NextToken`/`MaxResults` with rich
      summary shapes (job role/worker/version, crawler state/schedule, trigger
      schedule). **EMR is the list-shaper trap:** `ListClusters` pages on a bare
      `Marker` token but, unlike most JSON APIs, accepts **no** `MaxResults`
      parameter at all — the block omits `max_results_param` so the pager never
      sends one (sending an unsupported param fails silently to a single page).
      Target prefixes pinned by test (`AWSGlue`, `ElasticMapReduce`). Live
      verified 200 on all five.
- [x] DataSync tasks + Transfer Family servers/users. Both plain JSON protocol
      with their own `X-Amz-Target` prefixes (`FmrsService`, `TransferService`)
      pinned by test. Transfer users are scoped per-server: `transfer-users`
      is `requires_parent` and the servers table drills into it (`u` shortcut,
      Enter) via a `ServerId` parent filter — live driven through the drill and
      both hops returned 200. DataSync live 200 (empty), Transfer found a real
      ONLINE server.
- [x] App Mesh, Cloud Map, VPC Lattice, Route53 Resolver. The four group by
      wire protocol: App Mesh and VPC Lattice are rest-json GETs with lowercase
      camelCase roots and tokens (`/meshes`, `/items`, `nextToken`); Cloud Map
      (servicediscovery) and Route53 Resolver are JSON protocol with their own
      `X-Amz-Target` prefixes (`Route53AutoNaming_v20170314`,
      `Route53Resolver`) and PascalCase roots. Live verified 200 on all seven —
      real Cloud Map services/namespaces and Route53 Resolver rules
      (autodefined Internet Resolver) rendered.
- [ ] Global Accelerator — **blocked**: `ListAccelerators` is explicitly denied
      by an IAM identity policy on the target account, so the response body and
      `response_root` cannot be verified; implement only once a live response
      is obtainable.
- [x] Backup vaults/plans, Resource Groups, Service Quotas. Backup vaults and
      plans are rest-json GETs with full query-string pagination; Service
      Quotas is plain JSON (`ServiceQuotasV20190624`). **Resource Groups is the
      quirk:** `ListGroups` is a *POST* whose `MaxResults`/`NextToken` live in
      the query string, which the rest-json handler cannot express (it pages
      POST bodies), so the resource deliberately omits pagination rather than
      send params AWS would silently ignore — it falls back to AWS's default
      one page. Pinned by test. Live verified 200 on all four; Backup returned
      a real Default vault, quotas returned the full service list.
- [x] Trusted Advisor checks + Health events. Trusted Advisor is a *global*
      service served from `trustedadvisor.amazonaws.com` (no region in the
      host), so its service entry carries `is_global: true` and the resource is
      marked global to omit the region from the title — pinned by test. Health
      stays regional in the selected region and is plain JSON
      (`AWSHealth_20160804`). Live verified 200 on both; Trusted Advisor
      returned the real check catalog, Health returned live events.
- [ ] Cost Explorer + Budgets — **blocked for the same reason as Timestream**:
      `GetCostAndUsage` needs a `TimePeriod` date range and `Budgets`
      `DescribeBudgets` needs the caller's `AccountId`, and the JSON resource
      model has no dynamic-parameter substitution — a fixed static date goes
      stale nightly and a hardcoded account lies. Both need new Rust capability
      (dynamic date window / caller account resolution) not just JSON.
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
