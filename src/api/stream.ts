// ALL media playback goes through the Rust wzstream protocol. Never hand an
// <audio>/<video> element a raw path or a remote URL directly.

const isWindows = navigator.userAgent.includes("Windows");
const BASE = isWindows ? "http://wzstream.localhost" : "wzstream://localhost";

/** remote preview, proxied + allowlist-checked by the host */
export function remoteStreamUrl(sourceId: string, url: string): string {
  return `${BASE}/remote?src=${encodeURIComponent(sourceId)}&url=${encodeURIComponent(url)}`;
}

/** local collected file, range-served and confined to the collection dir */
export function localStreamUrl(absPath: string): string {
  return `${BASE}/local?path=${encodeURIComponent(absPath)}`;
}
