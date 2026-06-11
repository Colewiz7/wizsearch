// Mirrors of the Rust DTOs (serde snake_case).

export type AssetType = "audio" | "gif" | "sticker" | "image" | "video" | "green_screen";
export type PreviewKind = "audio_stream" | "video_loop" | "animated_image" | "poster_loop";

export interface FetchPlan {
  kind: "http_get" | "yt_dlp";
  url: string;
  headers?: [string, string][];
  filename_hint: string;
}

export interface ResultItem {
  id: string;
  source: string;
  asset_type: AssetType;
  title: string;
  thumbnail_url: string | null;
  preview_stream_url: string | null;
  preview_kind: PreviewKind;
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  license: string | null;
  attribution: string | null;
  origin_url: string | null;
  fetch_plan: FetchPlan;
}

export interface SearchPage {
  items: ResultItem[];
  next_cursor: string | null;
}

export interface SourceStatus {
  id: string;
  state: "pending" | "done" | "error" | "disabled" | "timeout" | "needs_key";
  error: string | null;
  count: number;
  next_cursor: string | null;
}

export interface SearchUpdate {
  search_id: number;
  items: ResultItem[];
  sources: SourceStatus[];
  done: boolean;
}

export interface SourceInfo {
  id: string;
  name: string;
  homepage: string;
  key_help_url: string;
  asset_types: AssetType[];
  requires_key: boolean;
  has_key: boolean;
  enabled: boolean;
}

export interface CollectedAsset {
  id: number;
  uid: string;
  source: string;
  asset_type: string;
  title: string;
  license: string | null;
  attribution: string | null;
  origin_url: string | null;
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  favorite: boolean;
  collected_at: number;
  tags: string[];
  abs_path: string;
  thumb_path: string | null;
  mime: string | null;
  bytes: number | null;
}

export interface CollectOutcome {
  asset: CollectedAsset;
  was_duplicate: boolean;
}

export type SettingKind =
  | { type: "bool"; default: boolean }
  | { type: "int"; default: number; min: number; max: number }
  | { type: "float"; default: number; min: number; max: number }
  | { type: "text"; default: string; placeholder: string }
  | { type: "select"; default: string; options: string[] }
  | { type: "secret"; help_url: string };

export interface SettingDef {
  key: string;
  label: string;
  description: string;
  category: string;
  kind: SettingKind;
}

export interface SidecarStatus {
  name: string;
  installed: boolean;
  path: string | null;
  pinned: boolean;
}

export const ASSET_TYPE_LABELS: Record<AssetType, string> = {
  audio: "Sounds",
  gif: "GIFs",
  sticker: "Stickers",
  image: "Images",
  video: "Videos",
  green_screen: "Green screens",
};
