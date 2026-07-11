# Feature 02 — VPS Hardening & Infra Setup

**Depends on:** 01
**Goal:** Produce an idempotent provisioning script and docs that secure the Ubuntu 24.04 VPS and install the container runtime.

## Requirements

- UFW configured: allow 22 and 80/443; deny 5555 and all internal service ports (default deny incoming).
- Docker Engine + Docker Compose plugin installed.
- Non-root `deploy` user with docker group membership.
- Provisioning documented and repeatable.

## Technical Implementation Steps

1. `deploy/setup.sh`: enable UFW with `default deny incoming` / `allow outgoing`, `ufw allow 22,80,443`, explicit `ufw deny 5555`.
2. Install Docker via official convenience script or apt repo; enable/start service.
3. Create `deploy` user, add to `docker` group, set up SSH.
4. `deploy/PROVISIONING.md` documenting each step and how to re-run safely.
5. Since opencode runs on the VPS, verify current firewall/docker state before mutating; make the script idempotent (guard clauses).

## Acceptance Criteria

- [ ] `ufw status verbose` shows only 22/80/443 allowed, 5555 denied.
- [ ] `docker compose version` and `docker run hello-world` succeed.
- [ ] Re-running `setup.sh` makes no destructive changes.
