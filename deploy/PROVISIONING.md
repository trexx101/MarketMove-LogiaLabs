# Provisioning

Step-by-step guide to harden a fresh Ubuntu 24.04 LTS VPS and prepare it for
MarketMarkovNet.

**Prerequisites:**

- A fresh Ubuntu 24.04 LTS VPS with root SSH access.
- Your SSH public key installed on the VPS (typically via your hosting
  provider's console or `ssh-copy-id root@<host>`).
- The MarketMarkovNet repository cloned locally (to transfer `setup.sh`).

**What this document covers:**

1. Initial system update
2. Running `deploy/setup.sh` (UFW firewall, Docker Compose plugin, deploy
   user, SSH hardening)
3. Verifying the hardening
4. Application deployment (links to `deploy/README.md`)
5. Filesystem layout
6. Re-running safely (idempotency)
7. Manual steps the script does not cover
8. Troubleshooting

**Companion docs:**

- [`deploy/README.md`](./README.md) — Docker Compose operations guide.
- [`deploy/config.md`](./config.md) — environment variable reference.
- [`deploy/KRAKEN_KEYS.md`](./KRAKEN_KEYS.md) — Kraken API key creation.

---

## Step 1: Initial access

SSH into the VPS as root and update the system:

```bash
ssh root@<vps-host>
apt update && apt upgrade -y
```

Reboot if the kernel was updated:

```bash
# Only if apt upgrade installed a new kernel:
reboot
# Then SSH back in.
```

---

## Step 2: Run setup.sh

### Transfer the script

From your local machine:

```bash
scp deploy/setup.sh root@<vps-host>:/root/setup.sh
```

Or, if the repo is already cloned on the VPS:

```bash
cp /path/to/repo/deploy/setup.sh /root/setup.sh
```

### Execute

```bash
chmod +x /root/setup.sh
/root/setup.sh
```

The script performs four operations, each with idempotency guards:

| Section | What it does |
|---------|-------------|
| **UFW Firewall** | Enables UFW with default-deny incoming. Allows 22/tcp, 80/tcp, 443/tcp. Explicitly denies 5555/tcp (ZMQ). |
| **Docker Compose Plugin** | Installs `docker-compose-plugin` via apt. Adds Docker's official repo if the package is not in the default repos. |
| **Deploy User** | Creates `deploy` user with home directory, adds to `docker` group, copies root's SSH keys, grants passwordless sudo. |
| **SSH Hardening** | Sets `PermitRootLogin no` and `PasswordAuthentication no` in sshd_config. Restarts sshd only if config changed. |

---

## Step 3: Verify hardening

Run each command and confirm the expected output.

### 3.1 Firewall

```bash
ufw status verbose
```

Expected:

```
Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), disabled (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW IN    Anywhere
80/tcp                     ALLOW IN    Anywhere
443/tcp                    ALLOW IN    Anywhere
5555/tcp                   DENY IN     Anywhere
22/tcp (v6)                ALLOW IN    Anywhere (v6)
80/tcp (v6)                ALLOW IN    Anywhere (v6)
443/tcp (v6)               ALLOW IN    Anywhere (v6)
5555/tcp (v6)              DENY IN     Anywhere (v6)
```

Only ports 22, 80, and 443 are allowed. Port 5555 is explicitly denied.

### 3.2 Docker Compose plugin

```bash
docker compose version
```

Expected: `Docker Compose version v2.x.x` (v2 or later).

### 3.3 Docker runtime

```bash
docker run hello-world
```

Expected: `Hello from Docker!` message and clean exit.

### 3.4 Deploy user

```bash
id deploy
```

Expected: `uid=...(deploy) gid=...(deploy) groups=...(deploy),...(docker)`

The `docker` group membership must be present.

### 3.5 SSH hardening

```bash
grep -E '^(PermitRootLogin|PasswordAuthentication)' /etc/ssh/sshd_config
```

Expected:

```
PermitRootLogin no
PasswordAuthentication no
```

### 3.6 Idempotency check

Re-run the script:

```bash
/root/setup.sh
```

Expected: every section reports "already configured" or "skipping" messages.
No destructive changes. No sshd restart. UFW rules are not duplicated.

---

## Step 4: Application deployment

With the VPS hardened, deploy the application stack. The full operational
guide is in [`deploy/README.md`](./README.md) (quick start, operations,
backups, security notes).

Summary:

1. Create the filesystem layout (see Step 5 below).
2. Clone the repo into `/opt/marketmarkovnet/app/`.
3. Place model files in `/opt/marketmarkovnet/models/`.
4. Configure `.env` — see [`deploy/config.md`](./config.md).
5. Build and launch with `docker compose -f deploy/docker-compose.yml up -d`.
6. Verify with `curl -fsSL https://$HOST/api/status | jq`.

For Kraken API key setup (live trading), see
[`deploy/KRAKEN_KEYS.md`](./KRAKEN_KEYS.md).

---

## Step 5: Filesystem layout

```
/opt/marketmarkovnet/
├── app/                 # git clone of this repo (engine, inference, deploy/)
├── models/              # model.pt, norm_stats.json (chmod 0600)
├── data/                # SQLite databases (bind-mounted into engine)
└── .env                 # secrets (chmod 0600, never committed)
```

Create it:

```bash
mkdir -p /opt/marketmarkovnet/{app,models,data}
chown -R deploy:deploy /opt/marketmarkovnet
```

From here, switch to the `deploy` user:

```bash
su - deploy
cd /opt/marketmarkovnet/app
git clone <repo-url> .
```

---

## Re-running safely

`setup.sh` is fully idempotent. Every section checks whether its target state
already exists before making changes:

- **UFW**: skips if all rules are already present and the firewall is active.
- **Docker Compose plugin**: skips if `docker compose version` succeeds.
- **Deploy user**: skips user creation if the user exists, skips group add if
  already a member, skips sudoers if the file exists.
- **SSH hardening**: skips each directive if already set correctly. Restarts
  sshd only when at least one directive was changed.

Re-running the script on an already-hardened VPS produces only "skipping"
messages and exits cleanly.

---

## Manual steps

The script does **not** handle these — they require operator action:

| Task | Why manual |
|------|-----------|
| **DNS setup** | Point your domain's A record to the VPS IP. Required before Caddy can issue a Let's Encrypt cert. |
| **`.env` secrets** | Kraken API keys, trading mode, and other secrets must be set by the operator. See [`deploy/config.md`](./config.md). |
| **Model files** | `model.pt` and `norm_stats.json` are not in the repo. Place them in `/opt/marketmarkovnet/models/` and `chmod 0600`. |
| **Git clone** | The repo must be cloned to `/opt/marketmarkovnet/app/` by the operator. |
| **Initial system update** | `apt update && apt upgrade -y` should be run before `setup.sh`. |
| **Backups** | Configure off-host backups for the `data` volume and `.env`. See [`deploy/README.md`](./README.md#backups). |

---

## Troubleshooting

### Locked out after SSH hardening

If you lose SSH access after running the script:

- The script copies `/root/.ssh/authorized_keys` to the deploy user. If root
  had no authorized_keys, the deploy user will have none either.
- **Fix**: use your hosting provider's web console to log in as root and add
  your key to `/home/deploy/.ssh/authorized_keys`.
- Root SSH login is disabled after the script runs. Always keep an active
  session open while testing, or use the provider's console.

### Docker group requires new login

After adding the `deploy` user to the `docker` group, the user must log out
and back in for the group membership to take effect:

```bash
su - deploy        # fresh login shell
docker ps          # should work without sudo
```

Alternatively, run `newgrp docker` in the current shell.

### Docker Compose plugin not found after install

If `docker compose version` fails after installation:

```bash
apt-get update
apt-get install -y docker-compose-plugin
```

If the package is not in the default Ubuntu repos, the script adds Docker's
official repository automatically. Verify:

```bash
cat /etc/apt/sources.list.d/docker.list
```

### UFW blocking Docker ports

UFW and Docker interact through iptables. Docker manipulates iptables directly,
which can bypass UFW rules for container-published ports. The MarketMarkovNet
compose file only publishes ports 80 and 443 (via Caddy), which are already
allowed by UFW. Internal ports (5555, 8080) are not published to the host.

Verify from an external machine:

```bash
nc -vz <vps-ip> 5555   # must fail (connection refused or timeout)
nc -vz <vps-ip> 8080   # must fail
nc -vz <vps-ip> 443    # must succeed
```

### sshd restart fails

If `systemctl restart sshd` fails, check the config syntax:

```bash
sshd -t
```

Fix any reported errors in `/etc/ssh/sshd_config` and retry.
