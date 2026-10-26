import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { detectFreeBuild } from "@/lib/free-build";
import type { CloudAuthState, CloudUser } from "@/types";

/**
 * Donut Free build: the local entitlements user. When the desktop is compiled
 * with the `free` feature it reports `free_build_enabled() = true`, and this
 * account replaces the login upsell: local automation, cross-OS fingerprints
 * and an effectively unlimited hourly budget; every cloud-served capability
 * (cookie bot, remote sessions, team, scheduled automation) stays off.
 * `profileLimit: 0` follows the project convention for "unlimited".
 */
const FREE_LOCAL_USER: CloudUser = {
  id: "free-local",
  email: "",
  plan: "local",
  planPeriod: null,
  subscriptionStatus: "active",
  profileLimit: 0,
  cloudProfilesUsed: 0,
  proxyBandwidthLimitMb: 0,
  proxyBandwidthUsedMb: 0,
  proxyBandwidthExtraMb: 0,
  deviceOrdinal: 1,
  deviceCount: 1,
  isPrimaryDevice: true,
  entitlements: {
    active: true,
    browserAutomation: true,
    crossOsFingerprints: true,
    cloudBackup: true,
    teamCollaboration: false,
    cookieBot: true,
    remoteInteractive: false,
    remoteControl: false,
    scheduledAutomation: true,
    profileLimit: 0,
    requestsPerHour: 1_000_000,
    remoteBrowserHours: 0,
  },
};

interface UseCloudAuthReturn {
  user: CloudUser | null;
  /** When this desktop signed in, as the backend recorded it. */
  loggedInAt: string | null;
  isLoggedIn: boolean;
  isLoading: boolean;
  exchangeDeviceCode: (code: string) => Promise<CloudAuthState>;
  logout: () => Promise<void>;
  refreshProfile: () => Promise<CloudUser>;
}

export function useCloudAuth(): UseCloudAuthReturn {
  const [authState, setAuthState] = useState<CloudAuthState | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const loadUser = useCallback(async () => {
    try {
      const freeBuild = await detectFreeBuild();
      if (freeBuild) {
        setAuthState({
          user: FREE_LOCAL_USER,
          logged_in_at: new Date().toISOString(),
        });
      } else {
        const state = await invoke<CloudAuthState | null>("cloud_get_user");
        setAuthState(state);
      }
    } catch (error) {
      console.error("Failed to load cloud auth state:", error);
      setAuthState(null);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadUser();

    const unlistenExpired = listen("cloud-auth-expired", () => {
      setAuthState(null);
    });

    const unlistenChanged = listen("cloud-auth-changed", () => {
      void loadUser();
    });

    return () => {
      void unlistenExpired.then((unlisten) => {
        unlisten();
      });
      void unlistenChanged.then((unlisten) => {
        unlisten();
      });
    };
  }, [loadUser]);

  const exchangeDeviceCode = useCallback(
    async (code: string): Promise<CloudAuthState> => {
      const state = await invoke<CloudAuthState>("cloud_exchange_device_code", {
        code,
      });
      setAuthState(state);
      return state;
    },
    [],
  );

  const logout = useCallback(async () => {
    if (await detectFreeBuild()) {
      // Local build: nothing to sign out of; keep the local session.
      return;
    }
    await invoke("cloud_logout");
    setAuthState(null);
  }, []);

  const refreshProfile = useCallback(async (): Promise<CloudUser> => {
    const user = await invoke<CloudUser>("cloud_refresh_profile");
    setAuthState((prev) =>
      prev
        ? { ...prev, user }
        : { user, logged_in_at: new Date().toISOString() },
    );
    return user;
  }, []);

  return {
    user: authState?.user ?? null,
    loggedInAt: authState?.logged_in_at ?? null,
    isLoggedIn: authState !== null,
    isLoading,
    exchangeDeviceCode,
    logout,
    refreshProfile,
  };
}
