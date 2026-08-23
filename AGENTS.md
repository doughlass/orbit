# AGENTS.md

Instructions for AI coding agents working on orbit. Read this before touching anything.

## What this is

`orbit` is a terminal UI for browsing AWS resources, in the spirit of k9s for
Kubernetes. Rust, crate name `orbit-tui`, binary `orbit`, built on
ratatui 0.30 + crossterm. ~19k lines across `src/`.

It is read-mostly: you list resources, inspect them, and run a small set of
explicitly-configured actions. There is a `--readonly` flag that blocks all
write operations.

## The one rule that matters

**Resources are data, not code.** Every AWS resource type lives in a JSON file
under `src/resources/`. There are no hardcoded resource keys in the Rust, and
adding a resource must not require any. If you find yourself writing
`if resource_key == "ec2-instances"`, stop — that is the wrong layer.

Adding a resource is normally:

1. A block in an existing `src/resources/*.json` (or a new file plus one
   `include_str!` line in `src/resource/registry.rs`).
2. Sometimes one entry in the service table in `src/aws/http.rs`.
3. Nothing else.

Rust changes are for new *capabilities* (a protocol quirk, a new transform, a
new navigation behaviour), never for new resources.

## Layout

```
src/main.rs               CLI args (clap), startup, terminal setup
src/app.rs                All application state and key handling (~2.8k lines)
src/event.rs              Event loop
src/config.rs             User config
src/completion.rs          Shell completion generation

src/aws/http.rs           Service table, endpoint resolution, SigV4 signing
src/aws/credentials.rs    Credential chain
src/aws/profiles.rs       ~/.aws/config parsing
src/aws/sso.rs            SSO / OIDC device flow
src/aws/console_login.rs  Federated console sign-in URLs
src/aws/tls.rs            Custom CA bundle handling
src/aws/client.rs         Client wiring

src/resource/registry.rs      Loads and parses all resource JSON; ResourceDef
src/resource/protocol.rs      ApiConfig / DescribeConfig / ActionConfig / FieldMapping
src/resource/fetcher.rs       List fetch + pagination loop
src/resource/dispatch.rs      Picks a handler, builds describe/action requests
src/resource/field_mapper.rs  Applies field_mappings and named transforms
src/resource/path_extractor.rs Path lookup into JSON, array-aware
src/resource/handlers/        One module per wire protocol

src/ui/                   Rendering. mod.rs is the table + layout core
src/resources/*.json      Resource definitions. This is where the work is.
```

## How a list fetch works

1. `app.rs` asks `fetcher.rs` for a resource key.
2. `fetcher.rs` reads the `ResourceDef` from the registry. If it has an
   `api_config`, it goes down the data-driven path (all new resources do).
   `sdk_method` is a legacy field kept for older resources; ignore it for new
   work but keep filling it in, since it is not `Option`.
3. `dispatch.rs` picks a handler by `api_config.protocol`.
4. The handler builds and signs the request via `src/aws/http.rs`, then converts
   the response to JSON (XML responses go through an XML→JSON conversion first).
5. `path_extractor.rs` pulls the item list out with `api_config.response_root`.
6. `field_mapper.rs` turns each raw item into a flat row using
   `field_mappings`.
7. `ui/mod.rs` renders the row using `columns`.

Every step is driven by the JSON. If a column is blank, one of steps 5–7 is
mis-configured; the request itself almost certainly worked.

## Protocols

`api_config.protocol` is one of four (`src/resource/protocol.rs`):

| Value | Style | Examples |
|---|---|---|
| `query` (default) | `Action=X&Version=Y` query params, XML back | EC2, IAM, ELB, RDS |
| `json` | JSON body with an `X-Amz-Target` header | DynamoDB, WAFv2, Logs |
| `rest-json` | REST path + JSON body | Lambda, EKS, ECS |
| `rest-xml` | REST path + XML body | S3, Route53, CloudFront |

### Query protocol conventions (EC2 and friends)

EC2 answers in XML with lowerCamelCase element names that do **not** match the
PascalCase the AWS CLI prints. Get these from the wire, not from the CLI output
(see "Verifying wire names" below).

```json
"response_root": "/DescribeRouteTablesResponse/routeTableSet/item"
```

Lists are `<xxxSet><item>...</item></xxxSet>`. Tags are `/tagSet/item` with
`"transform": "tags_to_map"`, then `"name_field": "Tags.Name"`.

### JSON protocol

Needs a `target_prefix` on the service entry in `http.rs`. Body is built from
`static_params`, then dynamic params, then pagination last (so a page token
always wins). See `src/resource/handlers/json.rs`.

## Anatomy of a resource definition

```json
"route-tables": {
  "display_name": "Route Tables",
  "service": "ec2",                    // key into the service table in http.rs
  "sdk_method": "describe_route_tables", // legacy, required, unused on this path
  "sdk_method_params": {},
  "response_path": "route_tables",     // key the normalized list is stored under
  "id_field": "RouteTableId",          // must exist in field_mappings
  "name_field": "Tags.Name",           // shown in the breadcrumb
  "is_global": false,                   // DISPLAY ONLY - see the warning below
  "columns": [
    { "header": "NAME", "json_path": "Tags.Name", "width": 20 },
    { "header": "MAIN", "json_path": "Main", "width": 6, "color_map": "bool" }
  ],
  "sub_resources": [],
  "actions": [],
  "api_config": {
    "protocol": "query",
    "action": "DescribeRouteTables",
    "response_root": "/DescribeRouteTablesResponse/routeTableSet/item",
    "pagination": {
      "input_token": "NextToken",
      "output_token": "/DescribeRouteTablesResponse/nextToken",
      "max_results_param": "MaxResults",
      "max_results": 100
    }
  },
  "field_mappings": {
    "RouteTableId": { "source": "/routeTableId", "default": "-" },
    "Destinations": {
      "source": "/routeSet/item/destinationCidrBlock",
      "transform": "array_to_csv",
      "default": "-"
    },
    "Tags": { "source": "/tagSet/item", "transform": "tags_to_map" }
  },
  "describe_config": { }
}
```

Optional extras: `describe_config` (what `d` fetches), `action_configs`,
`filters_config` (`{ "enabled": true, "hint": "owner, state" }`, enables
AWS-side filtering), `requires_parent`, `enter_sub_resource`.

`column.json_path` reads the **mapped** row, not the raw response. Dotted paths
like `Tags.Name` index into a mapped map. `field_mappings.source` reads the
**raw** item with a leading-slash path.

### Available transforms

From `apply_transform` in `src/resource/field_mapper.rs`. One per field, they do
not chain:

`tags_to_map`, `format_bytes`, `format_epoch_millis`, `bool_to_yes_no`,
`array_to_csv`, `first_item`, `private_zone_to_type`, `route53_record_value`,
`route53_record_id`.

An unknown transform name is silently ignored and the raw value passes through.
Nothing warns you. Spell them exactly.

### Color maps

Defined in `src/resources/common.json`, referenced by `column.color_map`. Only
`state` and `bool` exist. `bool` matches `true`/`false` and `Yes`/`No`, so it
works with or without `bool_to_yes_no`.

## Traps that have already bitten

**`is_global` on a resource is display-only.** It only omits the region from the
table title (`src/ui/mod.rs`). The real endpoint and signing region come from
`ServiceDefinition.is_global` in `src/aws/http.rs`. Setting it on the resource
and expecting the request to move regions does nothing.

**The global branch of `get_endpoint()` drops the region from the host**
(`https://{prefix}.{domain}`). That is right for IAM and CloudFront, wrong for
anything that is "one region only" but still region-addressed. WAFv2 needed its
own match arm for exactly this. Check before you reuse `is_global: true`.

**A wrong `response_root` returns an empty list with no error at all.** This is
the single easiest mistake in these files and it looks identical to "the account
has none of these". Always confirm the root against a real response.

**Pagination is not universal.** `DescribeAddresses` has no paginator and
rejects `MaxResults`/`NextToken` with `InvalidParameterCombination`, so copying
a neighbour's pagination block yields zero rows. Check the botocore model for
the operation before adding the block.

**`PaginationConfig` holds a single token pair.** Operations that page on two
tokens cannot be expressed. `ListResourceRecordSets` (Route53 records) needs
`NextRecordName` + `NextRecordType`, so records currently truncate at the first
page. Fixing that needs a Rust change, not a JSON one.

**Unknown JSON keys are dropped in silence.** `ResourceDef` does not use
`deny_unknown_fields`, so a misspelled or invented key parses fine and does
nothing. `vpc.json` still carries a `"tag_filter"` block that no Rust code has
ever read; the real field is `filters_config`. Confirm a key exists in
`src/resource/registry.rs` or `src/resource/protocol.rs` before relying on it.

**Single-item XML lists collapse.** `path_extractor.rs` returns a scalar when
one match is found and an array when several are. `array_to_csv` and
`first_item` both pass non-arrays through unchanged, which is why they work
either way. Do not assume you have an array.

## Navigation

Two ways to drill from a parent row into children:

```json
"sub_resources": [
  { "shortcut": "s", "display_name": "Subnets", "resource_key": "subnets",
    "parent_id_field": "VpcId", "filter_param": "vpc-id",
    "filter_type": "ec2_filter" }
],
"enter_sub_resource": "subnets"
```

- `sub_resources` gives a single-letter shortcut. `filter_type` is `scalar`
  (default, a plain param) or `ec2_filter` (EC2's `Filter.N.Name`/`Value`
  form).
- `enter_sub_resource` makes **Enter** drill into one of them. It must also
  appear in `sub_resources`, because `navigate_to_sub_resource` rejects
  undeclared targets and Enter would become an error message. A test pins this.
- The child needs `"requires_parent": true` if it cannot be listed standalone.

**Single-letter shortcuts are scarce.** Already taken globally: `j k g G d J R
w t q / : ? [ ] ← → Tab Shift+Tab Enter Esc Backspace Ctrl+b Ctrl+f Ctrl+c`.
Check `src/ui/help.rs` before claiming one. Colliding silently shadows the
global key for that resource.

## Working practice

**TDD is not optional here, and the tests are cheap.** The registry is parsed
at startup from embedded JSON, so a malformed or mis-shaped definition is
catchable in a unit test with no AWS access at all. The pattern used throughout
`src/resource/registry.rs`'s test module:

1. Write a test that pins an invariant across a family of resources (not one
   field of one resource).
2. Run it and **watch it fail for the right reason**. A test you never saw fail
   proves nothing.
3. Make the JSON change.
4. Re-run, then **mutate**: deliberately break the JSON in the way you fear,
   confirm the test catches it with a message that names the problem, revert.

Invariants worth pinning, all of which have caught real mistakes:

- `response_root` matches `/{action}Response/...Set/item`
- every `column.json_path`'s root segment exists in `field_mappings`
- pagination present exactly where the API supports it
- `id_field` is mapped (an unmapped id makes describe silently send an empty
  string)
- a list call and its describe call agree on any scope/scoping param

### Verifying wire names against real AWS

Do not guess element names and do not read them off AWS CLI output, which
renames things (`IsDefault` arrives as `default`; ENI tags arrive as `tagSet`).
Two sources, both read-only:

```bash
# 1. The raw wire response. The debug log dumps the XML body verbatim.
aws ec2 describe-route-tables --debug 2>&1 | grep -A2 'parsers - DEBUG - Response body'

# 2. The local botocore service model - authoritative for locationName,
#    pagination support, and limit bounds.
ls /opt/homebrew/var/homebrew/linked/awscli/libexec/lib/python3.14/site-packages/awscli/botocore/data/
```

An empty account or region proves nothing. Try another region before concluding
the mapping is right.

### Driving the real TUI headlessly

Unit tests cannot prove the request actually leaves the machine. This does,
and it is the only end-to-end check available:

```bash
LOG="$HOME/Library/Application Support/orbit/orbit.log"; rm -f "$LOG"
( sleep 4; printf ':'; sleep 1; printf 'route-tables'; sleep 1.5; printf '\r'; sleep 8 ) \
  | timeout 30 script -q /dev/null ./target/debug/orbit \
      --region eu-west-1 --readonly --log-level debug > /dev/null 2>&1
grep -oE 'action=[A-Za-z]*|Response status: [0-9]+|https://[^ "]*' "$LOG" | sort -u
```

`script` supplies the pty crossterm needs. Sleeps matter — keystrokes sent
before the TUI initialises are dropped and you will see only the startup fetch.
Rendered cell text is not reliably capturable this way, so treat this as proof
of the *request*, not of the render.

### Before every commit

```bash
cargo test --quiet                              # currently 201 tests, all green
cargo clippy --all-targets -- -D warnings       # must be silent
cargo fmt --check
```

All three are enforced. `-D warnings` means clippy lints are errors.

## Commits

- Never add a `Co-Authored-By:` trailer for an AI tool.
- Branch off `master`; do not commit to it directly.
- Write the *why*, not the *what* — the diff already says what changed. Name the
  AWS quirk or failure mode that forced the shape of the change, because that is
  what the next reader cannot reconstruct.
- One logical change per commit. If a change touches a shared file for two
  reasons, split the file's content across commits and verify each intermediate
  state compiles and tests green.

## Code style

Match the surrounding code. Specifics that are consistent throughout:

- Doc comments explain *why* and name the gotcha. Skip the obvious. Fewer
  comments beat more.
- Errors via `anyhow::Result`, with messages that say what was expected and what
  arrived.
- Prefer failing loudly on a malformed definition over silently rendering a
  blank cell.
- Test names are sentences describing behaviour
  (`vpc_networking_pagination_follows_the_ec2_api`), not `test_1`.
- Assertion messages name the resource key, so a failure across 60 resources
  tells you which one broke.

## Known gaps and open work

- Route53 records truncate at the first page (two-token pagination, above).
- Aurora clusters are invisible; only `DescribeDBInstances` is wired, not
  `DescribeDBClusters`.
- No CloudWatch alarms — needs a `monitoring` service entry in `http.rs`.
- No CloudTrail `LookupEvents`.
- The `elb` (classic) service is registered in `http.rs` but has no resources.
- The new VPC networking resources have no `sub_resources` wiring, deliberately:
  the obvious shortcut letters all collide with global keys.
- Column widths are fixed percentages per resource and most do not sum to 100.
  ratatui compresses them proportionally, which the layout code now measures
  accurately. Content-aware auto-fit is unimplemented.
- Edition is 2021; moving to 2024 and `reqwest` 0.13 is blocked on the custom
  CA-bundle handling in `src/aws/tls.rs`.
