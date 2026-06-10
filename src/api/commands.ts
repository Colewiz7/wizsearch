import { invoke } from "@tauri-apps/api/core";
import type {
  AssetType,
  CollectedAsset,
  CollectOutcome,
  ResultItem,
  SearchPage,
  SettingDef,
  SidecarStatus,
  SourceInfo,
} from "./types";

export const searchStart = (query: string, assetTypes: AssetType[]) =>
  invoke<number>("search_start", { query, assetTypes });

export const searchMore = (
  source: string,
  query: string,
  assetTypes: AssetType[],
  cursor: string | null,
) => invoke<SearchPage>("search_more", { source, query, assetTypes, cursor });

export const sourcesList = () => invoke<SourceInfo[]>("sources_list");

export const collectItem = (item: ResultItem) =>
  invoke<CollectOutcome>("collect_item", { item });

export const collectionSearch = (query: string, assetType: string | null) =>
  invoke<CollectedAsset[]>("collection_search", { query, assetType });

export const collectionDelete = (assetId: number) =>
  invoke<void>("collection_delete", { assetId });

export const assetSetTags = (assetId: number, tags: string[]) =>
  invoke<void>("asset_set_tags", { assetId, tags });

export const assetSetFavorite = (assetId: number, favorite: boolean) =>
  invoke<void>("asset_set_favorite", { assetId, favorite });

export const settingsDefs = () => invoke<SettingDef[]>("settings_defs");

export const settingsValues = () =>
  invoke<Record<string, unknown>>("settings_values");

export const settingsSet = (key: string, value: unknown) =>
  invoke<void>("settings_set", { key, value });

export const secretSet = (key: string, value: string) =>
  invoke<void>("secret_set", { key, value });

export const secretExists = (key: string) =>
  invoke<boolean>("secret_exists", { key });

export const secretClear = (key: string) => invoke<void>("secret_clear", { key });

export const sidecarsStatus = () => invoke<SidecarStatus[]>("sidecars_status");

export const sidecarsEnsure = () => invoke<SidecarStatus[]>("sidecars_ensure");

export const ytdlpUpdate = () => invoke<string>("ytdlp_update");

export const assetPath = (assetId: number) =>
  invoke<string>("asset_path", { assetId });
