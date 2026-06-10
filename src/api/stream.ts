// ALL media playback goes through the Rust stream server (same allowlist and
// Range handling as the wzstream protocol, served over 127.0.0.1 because
// webkitgtk's GStreamer can't read custom URI schemes). Never hand an
// <audio>/<video> element a raw path or a remote URL directly.

import { invoke } from "@tauri-apps/api/core";

let base = "";
let token = "";

/** called once at startup before the app renders */
export async function initStream(): Promise<void> {
  const info = await invoke<{ base: string; token: string }>("stream_base");
  base = info.base;
  token = info.token;
}

/** remote preview, proxied + allowlist-checked by the host */
export function remoteStreamUrl(sourceId: string, url: string): string {
  return `${base}/remote?src=${encodeURIComponent(sourceId)}&url=${encodeURIComponent(url)}&t=${token}`;
}

/** local collected file, range-served and confined to the collection dir */
export function localStreamUrl(absPath: string): string {
  return `${base}/local?path=${encodeURIComponent(absPath)}&t=${token}`;
}
