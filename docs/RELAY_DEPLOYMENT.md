# Operating the single Tokyo relay

This is the deployment owner’s procedure for one dedicated Ubuntu 24.04 x86_64 VM in `asia-northeast1`.
It deploys only the memory-only relay from [RELAY_PROTOCOL.md](RELAY_PROTOCOL.md).
The phone site still deploys through [release-candidate.yml on main](BUILD.md#the-hosted-phone-release); this procedure never publishes it or a desktop release.
Creating resources, authenticating, changing DNS and running the installation on production belong to the deployment owner.
The source PR and its Linux fixture do not prove real TLS, cloud ingestion, delivered alerts or physical-phone pairing.

## Ownership and prerequisites

Use a dedicated `e2-small`, a 10 GiB balanced persistent boot disk and one reserved regional external IPv4 attached to the VM.
Use a dedicated VPC/subnet and firewall target service account, with ingress TCP 80/443 from the internet and TCP 22 only from IAP’s `35.235.240.0/20`.
Do not create a public SSH rule, open 8787/2020, delete another VM’s rules, or reuse an existing application host.
Enable OS Login and grant the human operator IAP tunnel access plus the appropriate OS Login role, scoped to this VM where possible.
Verify the effective firewall rules, including inherited policies, rather than assuming a newly created restrictive rule overrides a broad allow.
Keep VPC flow logging and firewall connection logging off for this deployment: they retain complete client addresses.
Point `relay.withhide.dev` directly at the reserved IPv4; no CDN or other proxy sits in front of Caddy.
Do not add an AAAA record without a working IPv6 listener and firewall policy.
Allow outbound HTTPS to the certificate authority, package repositories and Google Logging, plus platform DNS/time services.

The VM service account receives only `roles/logging.logWriter` in the destination project, with the `logging.write` access scope.
Fluent Bit uses that VM’s metadata identity, never a JSON key, repository credential or CLI login.
It writes no Monitoring metrics, so the VM does not need `roles/monitoring.metricWriter`; uptime checks run externally.
Human provisioning rights are separate from this runtime account.
Keep the free-trial billing account unchanged; never upgrade billing as part of this procedure.

Use no swap, hibernation, memory dumps, disk snapshots or backup jobs that copy the journal.
The service units disable core dumps and swapping and impose memory/task caps, but an independent backup would otherwise defeat retention.
Do not install a second log agent or collect syslog, proxy output, the whole system journal or collector diagnostics.
If the VM already has such an agent or a backup policy, stop and resolve ownership before deployment; do not delete unrelated logs or configuration.

Install Python 3.11+, curl, Caddy and Fluent Bit from their signed package repositories.
The isolated CI uses Ubuntu’s Caddy package and Fluent Bit `5.1.5`; record the installed versions and package origin with each deployment.
Follow [Caddy’s package instructions](https://caddyserver.com/docs/install#debian-ubuntu-raspbian) and [Fluent Bit’s signed Ubuntu repository instructions](https://docs.fluentbit.io/manual/installation/downloads/linux/ubuntu).
Before installing on this new dedicated VM, mask the unused package services (`systemctl mask caddy.service fluent-bit.service`) so only the units in this deployment can listen or collect.
Do not mask an existing operator-owned service.
The expected executables are `/usr/bin/caddy` and `/opt/fluent-bit/bin/fluent-bit`; a missing executable is an installation failure.
Create the three non-login accounts once:

```sh
sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin hide-relay
sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin hide-relay-proxy
sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin hide-relay-logs
```

On a repeat run, inspect existing accounts instead of accepting a same-name interactive account.
The relay has no writable application state; Caddy owns its certificate and configuration state under `/var/lib/hide-relay-proxy`, and the collector owns only its cursor under `/var/lib/hide-relay-logs`.
`scripts/relay-deploy.py` owns the explicit input contract and validates all required values before staging; `deploy/relay-inputs.example.json` lists placeholders.
Real input values, rendered cloud bodies, receipts and logs stay outside version control.
The service runtime has no user-provided environment contract; Caddy’s two XDG variables name fixed service-owned paths.

## Source and artifact

1. Select the protected-main commit that will be deployed and require its successful `verify` push/main run.
2. Dispatch `relay-artifact.yml` on `main`; the workflow builds its exact dispatch SHA with `bash scripts/verify-cargo.sh build --release -p hide-relay --bin hide-relay` and the locked dependencies.
3. Read the successful workflow run’s repository, path, branch, SHA and attempt from GitHub before downloading its `hide-relay-linux-x64-<SHA>` artifact.
   Require that SHA to equal the selected verified commit; main moving between selection and dispatch is a new candidate, not permission to relabel the artifact.
4. Keep `source.json`, `SHA256SUMS`, the run URL/attempt and downloaded bytes in the deployment record, then run `sha256sum --check SHA256SUMS` in the artifact directory.
   These are provenance and integrity evidence from the trusted CI run, not signing or a substitute for verifying which run supplied them.
5. Copy only the artifact, `scripts/relay-deploy.py` and `deploy/relay/` from that exact source through IAP to the VM.
   Use `gcloud compute scp --tunnel-through-iap --project <project> --zone <zone> ...`; the VM gets no clone or GitHub credential.

The manual producer has no deployment secret and performs no cloud or DNS operation.
Its artifact expires after seven days; keep a verified active and previous binary on the VM, and rebuild an expired artifact through main CI when needed.
Never build the production binary on the VM or substitute a workstation build.

## Logging routing before starting services

Render cloud API bodies with explicit values from the approved project and target VM:

```sh
python3 scripts/relay-deploy.py cloud-config \
  --project <project-id> --project-number <numeric-project-number> \
  --instance-id <numeric-instance-id> \
  --channel projects/<project-id>/notificationChannels/<verified-email-channel-id> \
  --currency <actual-billing-currency> --out <private-cloud-directory>
```

The command refuses non-KRW currency without creating output.
If the actual billing account cannot express KRW 100000, bring the currency and proposed amount to the conductor/operator for a decision; do not convert or replace the amount silently.
Cloud rendering does not authenticate or create resources.
For non-KRW billing, the deployment remains blocked on that decision even if service configuration is otherwise ready.

Before starting the collector, the deployment owner applies this routing, using `relay-filter.txt` unchanged as the scope:

- Create `hide-relay` in Logging location `asia-northeast1` with retention **7 days**, then create a sink to `logging.googleapis.com/projects/<project-id>/locations/asia-northeast1/buckets/hide-relay` with that filter.
- Add a named `hide-relay` exclusion with exactly the same filter to the `_Default` sink, preserving all other exclusions and its inclusion filter.
  `gcloud logging sinks update _Default --project <project-id> --add-exclusion='name=hide-relay,filter=<rendered-filter>'` adds the owned exclusion; do not use a replacement exclusion list or disable the sink.
- Inspect every project, folder and organization sink that can also route the same events.
  An additional destination must have a seven-day-or-shorter lifecycle or exclude just these events; an unmodifiable ancestor sink blocks rollout.
- Read back the bucket, sink and exclusions and keep that effective configuration in the private deployment record before service start.
  A dedicated seven-day bucket is insufficient if `_Default` or another sink still keeps a second copy.

The filter includes resource type, project ID, numeric instance ID and exact log ID `hide-relay`.
It does not exclude audit logs or logs belonging to any other VM.
Fluent Bit derives the log ID from its `hide-relay` input tag and obtains the GCE resource identity from metadata.
Confirm those fields on an actual canary before declaring routing verified.
Do not create log-based metrics or notification label extractors containing payload fields.

## Install and update

Run from the copied exact-source deployment files on the VM, with a new root-owned staging path for each attempt:

```sh
sudo python3 scripts/relay-deploy.py stage \
  --artifact <artifact-directory> --out <new-private-stage-directory> \
  --source <full-main-sha> --sha256 <binary-sha256-from-verified-run> \
  --project <project-id> --machine-id <target-etc-machine-id>
sudo python3 scripts/relay-deploy.py install <private-stage-directory>
```

Staging checks source, target, workflow identity and binary SHA-256 without executing the download.
Installation verifies the staged inventory, target machine and ownership of every unit/configuration path before stopping services.
Only trusted root-owned source and staging directories may be installed: the inventory detects damage, not an attacker able to replace both files and inventory.
The installer takes `/run/lock/hide-relay-install.lock`, retains immutable releases under `/opt/hide-relay/releases/<SHA>`, and changes one `current` symlink for both binary and configuration.
It refuses unmanaged units, conflicting bytes for an already-used SHA and more than three retained releases.
It stops the three deployment services, switches the link, restarts the namespace journal and services, then requires local relay/collector health, a certificate-validated HTTPS proxy response with status 200 and the relay's exact health body, and active services.
The HTTPS check resolves the production domain to loopback and does not follow HTTP redirects; public DNS must already allow Caddy to obtain its certificate.
It also enables and runs the journal retention timer/service.
Public certificate issuance and remote logging/notifications still require the acceptance checks below.

A normal activation failure restores the previous link and restarts its services; on the first installation, failure leaves the services disabled with no active release.
If recovery fails or installation is interrupted, `pending.json` remains and another install is refused.
The recovery record is flushed and atomically published before any service stops; interruption while writing its temporary file leaves the running release retryable.
Repair the reported prerequisite and run `sudo python3 scripts/relay-deploy.py recover`; a successful recovery clears the record.
Never delete `pending.json` to bypass recovery.
After a successful update, explicitly reinstall the retained previous release to roll back; its inputs and bytes stay available.
Updates briefly disconnect all rooms and clear memory; clients must reconnect with fresh admission and Noise sessions.
After reviewing the active/previous links, manually remove only an older inactive release to stay under the three-release cap.
Do not remove Caddy’s certificate state or the collector cursor during a binary rollback.

## Privacy, bounds and retention

`deploy/relay/` owns the units and configuration; changes to those files update this guide in the same PR.
Caddy overwrites `X-Real-IP` with `{remote_host}` and sends requests only to `127.0.0.1:8787`.
The relay trusts only `127.0.0.1`; both the relay listener and collector health listener are loopback-only.
Caddy automatically issues and renews the domain certificate, redirects HTTP to HTTPS and retains its certificate state across updates.
There is no access logger; its global logger discards output and systemd discards both streams because proxy runtime failures can contain request objects too.
Remote health checks, rather than retained request-bearing errors, report proxy failure.

The relay writes its content-free JSON only to the `hide-relay` journal namespace.
That namespace disables forwarding, caps storage at 64 MiB, rotates files hourly and expires them at six days; an hourly rotate/vacuum timer provides margin inside the seven-day ceiling, including active-file rotation.
An offline disk cannot execute expiry: before bringing a VM retained offline longer than seven days back into service, remove only this namespace’s expired journal files and verify the namespace before starting the relay/collector.
Do not retain snapshots or copies of those files.
Host service-manager lifecycle records may remain in the default journal, but no relay event payload is forwarded there.

Fluent Bit reads only that namespace directory and exact relay unit.
Its parser drops original journal fields; its event filter permits only `cap_exceeded`, `server_failed` and the collector's categorical `collector_canary`; its field allowlist sends only `event`, never room, content, credentials or client addresses.
Unparseable messages are discarded rather than forwarded raw.
The cursor database contains position metadata, not events.
There is no disk spool; the journal input buffer is 8 MiB, the canary input buffer is 1 MiB, output retry count is three with a 30-second scheduler cap, and systemd replaces the collector every twelve hours so buffered copies expire within the journal’s remaining retention margin.
The collector can lose events after exhausted delivery retries or restart; its health check exposes output errors/retry failures, and journal retention is not a promise of lossless delivery.
In particular, a cursor advanced before an output failure does not rewind on restart.
The separate `/collector-health` uptime alert is mandatory.
The built-in dummy input sends one content-free canary at startup and every sixty seconds through the same parser, filters and Stackdriver destination.
This keeps authentication/network delivery failures visible inside the five-minute health error window even when no real relay event occurs; without a canary, an exhausted retry could age out and falsely report recovery.
The canary uses the same log ID, VM scope and seven-day destinations, adds 1,440 small records per day, and never triggers the two relay-event alerts.
After a collector restart, allow one canary/flush cycle to establish delivery health; process startup alone is not proof of delivery.
The mandatory cloud delivery-absence alert observes received canaries independently of local health, so restarting the collector does not reset the delivery observation.
Do not enable debug/trace logs to diagnose production ingestion; inspect counts/status and use a private synthetic fixture instead.

Memory limits are relay 640 MiB, proxy 256 MiB and collector 192 MiB; task limits are 64, 128 and 64 respectively.
The service manager restarts failures after five seconds, with five starts per minute before a persistent failure requires operator repair/reset.
The 20-second stop timeout exceeds the relay’s sixteen-second drain bound.
These are resource guards, not evidence of production throughput or of a hard billing cap.

## Alerts and acceptance

Apply `cap_exceeded.json` and `server_failed.json` through the Monitoring alert-policy API or `gcloud monitoring policies create --policy-from-file <file>`.
Use the already verified email channel; a configured channel is not delivered-mail evidence.
Create `collector-delivery-metric.json` through Logging's `projects.metrics.create`, then create `collector-delivery-alert.json` through the Monitoring alert-policy API.
The counter selects only this VM's received canaries, with no extracted payload labels; Monitoring stores aggregate counts, not another copy of a relay event.
The absence policy requires an established time series: observe the first nonzero counter point before accepting the deployment, and fail acceptance if it never appears.
It detects five minutes without metric data, plus Logging/Monitoring processing delay and a possible adjacent zero-count interval; do not claim a precise five-minute delivery SLA.
Confirm the absence incident stays open across a collector restart during an ongoing delivery failure and clears only after real delivery resumes.
Create both `relay-uptime.json` and `collector-uptime.json` through `projects.uptimeCheckConfigs.create`.
Replace only `REPLACE_WITH_CREATED_CHECK_ID` in each corresponding uptime-alert body with that check’s returned ID, then create its alert policy.
The checks require valid HTTPS on 443 every minute; two failed locations for two minutes alert, and missing evaluation data is active.
`/collector-health` exposes only Fluent Bit’s health response, never its metrics/admin endpoints.
Apply `budget.json` through the Billing Budgets API for the approved billing account: monthly, project-only, KRW 100000, thresholds 50/90/100%, costs before credits, verified email channel plus billing IAM recipients.
Inventory existing owned resource IDs before creating anything; on retries update those exact resources instead of creating duplicates.

Before accepting users, record these as separate observations:

1. Exact source/run/attempt, installed binary digest, package versions, effective units and active restart/resource limits.
2. Public DNS, trusted TLS certificate and `/health` response; actual WSS/Noise pairing with private client state; spoofed incoming `X-Real-IP` does not control accounting.
3. Kill only this deployment’s relay and observe restart and client recovery; verify certificate-renewal configuration separately from any future renewal observation.
4. Trigger a controlled cap event and a controlled server failure through the real unit/collector path, and confirm the two log IDs/fields in the dedicated bucket and absence from `_Default` and every other destination.
5. Observe a nonzero canary delivery metric, then stop the collector and separately induce a controlled collector delivery refusal; observe `/collector-health` failure with no relay events for longer than five minutes, uptime/delivery-absence incidents and actual email receipt, restoring it after each check.
6. Exercise relay health failure and restoration, confirm the notification outcome, read back budget scope/currency/thresholds, and record local/cloud retention including all copies.
7. Confirm the retention timer’s last result and run the namespace-only vacuum manually; inspect no unrelated journal or log file.
8. Ask the operator for the physical-phone QR/reply observation after the separate phone release; browser emulation does not satisfy it.

Do not mark the deploy node’s production criteria passed from source tests or a running process.
`python3 -m unittest scripts.tests.test_relay_deploy` exercises staging/installation failures with real files and an OS-service double.
`relay-deploy-check.yml` additionally runs real systemd, Caddy and Fluent Bit on a disposable Ubuntu runner with a locally trusted fixture certificate, local relay process and metadata/Logging API fixtures.
It exercises the actual Stackdriver plugin's token request and Logging envelope, restart, broken-HTTPS-proxy rollback, forged-header overwrite, field removal, and quiet persistent 403 delivery failure beyond the five-minute health window.
It never reaches a certificate authority or a GCP account.

## Costs and official references

Capacity planning starts with one small VM, 10 GiB disk and one used public IPv4; internet egress is workload-dependent and must be watched.
The mission’s 2026-11-03 official-price research estimated USD 20.64/month for `e2-small` or USD 36.33 for `e2-medium`, at 730 hours including that disk and IP, excluding egress, logging/monitoring, taxes and snapshots.
Recheck the [Compute Engine](https://cloud.google.com/compute/all-pricing), [disk](https://cloud.google.com/compute/disks-image-pricing) and [network](https://cloud.google.com/vpc/network-pricing) prices before provisioning; this is an estimate, not a current quote or currency conversion.
Tokyo E2 is not the US-region always-free allowance, and [budgets](https://cloud.google.com/billing/docs/how-to/budgets) alert rather than stop spending.

The choices above follow [IAP TCP forwarding](https://cloud.google.com/iap/docs/using-tcp-forwarding), [OS Login](https://cloud.google.com/compute/docs/oslogin/set-up-oslogin), [Caddy automatic HTTPS](https://caddyserver.com/docs/automatic-https) and [header replacement](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#headers).
The dedicated journal follows [systemd journal namespaces/retention](https://www.freedesktop.org/software/systemd/man/latest/journald.conf.html).
Fluent Bit’s [systemd path/filter](https://docs.fluentbit.io/manual/data-pipeline/inputs/systemd), [bounded canary input](https://docs.fluentbit.io/manual/data-pipeline/inputs/dummy), [memory buffering](https://docs.fluentbit.io/manual/administration/buffering-and-storage), [health](https://docs.fluentbit.io/manual/administration/monitoring) and [GCE Logging output](https://docs.fluentbit.io/manual/data-pipeline/outputs/stackdriver) provide the collection boundary.
It is used instead of Ops Agent here because the latter’s documented journald receiver has no namespace-path selection; changing the shared journal’s retention would affect unrelated events.
Cloud configuration follows [log routing](https://cloud.google.com/logging/docs/routing/overview), [bucket retention](https://cloud.google.com/logging/docs/buckets), [log alerts](https://cloud.google.com/logging/docs/alerting/log-based-alerts), [uptime API](https://cloud.google.com/monitoring/api/ref_v3/rest/v3/projects.uptimeCheckConfigs) and [budget currency/project filters](https://cloud.google.com/billing/docs/reference/budget/rest/v1/billingAccounts.budgets).
Delivery freshness uses [counter metrics](https://cloud.google.com/logging/docs/logs-based-metrics/counter-metrics) and [metric-absence alerts](https://cloud.google.com/monitoring/alerts/metric-absence); counter zero values and gaps follow [Logging's documented time-series rules](https://cloud.google.com/logging/docs/logs-based-metrics).
