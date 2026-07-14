/**
 * API client for MarketMarkovNet control room.
 * Each function returns parsed JSON or throws on non-2xx.
 */

const BASE = "";

async function request(path) {
  const res = await fetch(BASE + path, { cache: "no-store" });
  if (!res.ok) {
    throw new Error(`${path} → ${res.status} ${res.statusText}`);
  }
  return res.json();
}

export function fetchStatus() {
  return request("/api/status");
}

export function fetchPredictions() {
  return request("/api/predictions");
}

export function fetchChart() {
  return request("/api/chart");
}
