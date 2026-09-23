/**
 * Global outbound proxy API
 *
 * Gets, sets and tests the global proxy.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Proxy test result
 */
export interface ProxyTestResult {
  success: boolean;
  latencyMs: number;
  error: string | null;
}

/**
 * Outbound proxy status
 */
export interface UpstreamProxyStatus {
  enabled: boolean;
  proxyUrl: string | null;
}

/**
 * Detected proxy
 */
export interface DetectedProxy {
  url: string;
  proxyType: string;
  port: number;
}

/**
 * Get the global proxy URL
 *
 * @returns the proxy URL, or null when not configured (direct connection)
 */
export async function getGlobalProxyUrl(): Promise<string | null> {
  return invoke<string | null>("get_global_proxy_url");
}

/**
 * Set the global proxy URL
 *
 * @param url - proxy URL (e.g. http://127.0.0.1:7890 or socks5://127.0.0.1:1080)
 *              an empty string clears the proxy (direct connection)
 */
export async function setGlobalProxyUrl(url: string): Promise<void> {
  try {
    return await invoke("set_global_proxy_url", { url });
  } catch (error) {
    // A Tauri invoke error may be a string
    throw new Error(typeof error === "string" ? error : String(error));
  }
}

/**
 * Test the proxy connection
 *
 * @param url - proxy URL to test
 * @returns test result: success, latency and error message
 */
export async function testProxyUrl(url: string): Promise<ProxyTestResult> {
  return invoke<ProxyTestResult>("test_proxy_url", { url });
}

/**
 * Get the current outbound proxy status
 *
 * @returns proxy status: whether enabled and the proxy URL
 */
export async function getUpstreamProxyStatus(): Promise<UpstreamProxyStatus> {
  return invoke<UpstreamProxyStatus>("get_upstream_proxy_status");
}

/**
 * Scan for local proxies
 *
 * @returns detected proxies
 */
export async function scanLocalProxies(): Promise<DetectedProxy[]> {
  return invoke<DetectedProxy[]>("scan_local_proxies");
}
