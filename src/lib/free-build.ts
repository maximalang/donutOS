"use client";

import { invoke } from "@tauri-apps/api/core";

/**
 * Free local build: cloud vendor services are stripped. Cloud-only UI
 * (cookie bot, agent relay, cloud account) must be hidden; everything
 * local (profiles, proxies, VPN, local API + MCP, self-hosted sync) stays.
 */
let cached: boolean | null = null;

export async function detectFreeBuild(): Promise<boolean> {
  if (cached !== null) return cached;
  try {
    cached = await invoke<boolean>("is_free_build");
  } catch {
    cached = false;
  }
  return cached;
}

export function freeBuildKnown(): boolean {
  return cached !== null;
}

export function isFreeBuildSync(): boolean {
  return cached === true;
}
