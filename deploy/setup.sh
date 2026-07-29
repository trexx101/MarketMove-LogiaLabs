#!/usr/bin/env bash
set -euo pipefail

# MarketMoves — VPS Hardening Script
# Target: Ubuntu 24.04 LTS (Noble Numbat)
# Scope:  UFW firewall, Docker Compose plugin, deploy user, SSH hardening
#
# NOTE: All deploy/ scripts in this repo use the `docker compose` (v2)
# command, never `docker-compose` (v1). v1 is incompatible with Docker
# server 25+ and will fail with `KeyError: ContainerConfig`.

log() {
    printf '[%s] %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*"
}

if [[ "$(id -u)" -ne 0 ]]; then
    log "ERROR: This script must be run as root (or with sudo)."
    exit 1
fi

SSHD_CHANGED=0

# ─────────────────────────────────────────────────────────────────────────────
# UFW Firewall
# ─────────────────────────────────────────────────────────────────────────────
log "=== UFW Firewall ==="

ufw_is_active() {
    ufw status | grep -qi '^Status: active'
}

ufw_rule_exists() {
    local rule="$1"
    ufw status | grep -qF "$rule"
}

if ufw_is_active && ufw_rule_exists "22/tcp" && ufw_rule_exists "80/tcp" && ufw_rule_exists "443/tcp" && ufw_rule_exists "5555/tcp"; then
    log "UFW already configured — skipping."
else
    if ! ufw_is_active; then
        log "Enabling UFW with default deny incoming / allow outgoing..."
        ufw default deny incoming
        ufw default allow outgoing
        ufw --force enable
    fi

    for port in 22 80 443; do
        if ! ufw_rule_exists "${port}/tcp"; then
            log "Allowing ${port}/tcp..."
            ufw allow "${port}/tcp"
        else
            log "Port ${port}/tcp already allowed — skipping."
        fi
    done

    if ! ufw_rule_exists "5555/tcp" || ! ufw status | grep -q "5555/tcp.*DENY"; then
        if ufw_rule_exists "5555/tcp" && ! ufw status | grep -q "5555/tcp.*DENY"; then
            log "WARNING: 5555/tcp rule exists but is not DENY. Not deleting — manual review needed."
        elif ! ufw_rule_exists "5555/tcp"; then
            log "Denying 5555/tcp (ZMQ — must never be public)..."
            ufw deny 5555/tcp
        else
            log "5555/tcp already denied — skipping."
        fi
    else
        log "5555/tcp already denied — skipping."
    fi

    log "UFW status:"
    ufw status verbose
fi

# ─────────────────────────────────────────────────────────────────────────────
# Docker Compose Plugin
# ─────────────────────────────────────────────────────────────────────────────
log "=== Docker Compose Plugin ==="

if docker compose version &>/dev/null; then
    log "Docker Compose plugin already installed: $(docker compose version --short 2>/dev/null || docker compose version)"
else
    log "Docker Compose plugin not found. Installing..."

    if ! apt-cache show docker-compose-plugin &>/dev/null; then
        log "Adding Docker official repository..."
        install -m 0755 -d /etc/apt/keyrings
        curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
        chmod a+r /etc/apt/keyrings/docker.asc

        # shellcheck source=/dev/null
        printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu %s stable\n' \
            "$(dpkg --print-architecture)" \
            "$(. /etc/os-release && printf '%s' "$VERSION_CODENAME")" \
            > /etc/apt/sources.list.d/docker.list

        apt-get update
    fi

    apt-get install -y docker-compose-plugin

    if docker compose version &>/dev/null; then
        log "Docker Compose plugin installed: $(docker compose version --short 2>/dev/null || docker compose version)"
    else
        log "ERROR: Docker Compose plugin installation failed."
        exit 1
    fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# Deploy User
# ─────────────────────────────────────────────────────────────────────────────
log "=== Deploy User ==="

if id -u deploy &>/dev/null; then
    log "User 'deploy' already exists — skipping creation."
else
    log "Creating user 'deploy'..."
    useradd --create-home --shell /bin/bash deploy
fi

if ! id -nG deploy | grep -qw docker; then
    log "Adding 'deploy' to docker group..."
    usermod -aG docker deploy
else
    log "User 'deploy' already in docker group — skipping."
fi

log "Setting up SSH for deploy user..."
DEPLOY_SSH_DIR="/home/deploy/.ssh"
mkdir -p "$DEPLOY_SSH_DIR"
chmod 0700 "$DEPLOY_SSH_DIR"

if [[ -f /root/.ssh/authorized_keys ]]; then
    cp /root/.ssh/authorized_keys "${DEPLOY_SSH_DIR}/authorized_keys"
    log "Copied /root/.ssh/authorized_keys to deploy user."
else
    log "WARNING: /root/.ssh/authorized_keys not found. Deploy user will have no SSH keys."
    touch "${DEPLOY_SSH_DIR}/authorized_keys"
fi

chown -R deploy:deploy "$DEPLOY_SSH_DIR"
chmod 0600 "${DEPLOY_SSH_DIR}/authorized_keys"

SUDOERS_FILE="/etc/sudoers.d/deploy"
if [[ -f "$SUDOERS_FILE" ]]; then
    log "Sudoers file already exists — skipping."
else
    log "Granting passwordless sudo to deploy user..."
    printf 'deploy ALL=(ALL) NOPASSWD:ALL\n' > "$SUDOERS_FILE"
    chmod 0440 "$SUDOERS_FILE"
fi

# ─────────────────────────────────────────────────────────────────────────────
# SSH Hardening
# ─────────────────────────────────────────────────────────────────────────────
log "=== SSH Hardening ==="

SSHD_CONFIG="/etc/ssh/sshd_config"

if [[ ! -f "$SSHD_CONFIG" ]]; then
    log "WARNING: ${SSHD_CONFIG} not found — skipping SSH hardening."
else
    if grep -qE '^\s*PermitRootLogin\s+no\s*$' "$SSHD_CONFIG"; then
        log "PermitRootLogin already set to no — skipping."
    else
        if grep -qE '^\s*#?\s*PermitRootLogin' "$SSHD_CONFIG"; then
            sed -i 's/^\s*#\?\s*PermitRootLogin.*/PermitRootLogin no/' "$SSHD_CONFIG"
        else
            printf '\nPermitRootLogin no\n' >> "$SSHD_CONFIG"
        fi
        log "Set PermitRootLogin no."
        SSHD_CHANGED=1
    fi

    if grep -qE '^\s*PasswordAuthentication\s+no\s*$' "$SSHD_CONFIG"; then
        log "PasswordAuthentication already set to no — skipping."
    else
        if grep -qE '^\s*#?\s*PasswordAuthentication' "$SSHD_CONFIG"; then
            sed -i 's/^\s*#\?\s*PasswordAuthentication.*/PasswordAuthentication no/' "$SSHD_CONFIG"
        else
            printf '\nPasswordAuthentication no\n' >> "$SSHD_CONFIG"
        fi
        log "Set PasswordAuthentication no."
        SSHD_CHANGED=1
    fi

    if [[ "$SSHD_CHANGED" -eq 1 ]]; then
        log "Restarting sshd..."
        systemctl restart sshd
    else
        log "sshd config unchanged — no restart needed."
    fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# Summary
# ─────────────────────────────────────────────────────────────────────────────
log "=== Hardening Complete ==="
log ""
log "Verify with:"
log "  ufw status verbose"
log "  docker compose version"
log "  id deploy"
log ""
log "Done."
