import { listen } from "@tauri-apps/api/event";
import { VirtuosoMasonry } from "@virtuoso.dev/masonry";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { searchMore, searchStart, sourcesList } from "../api/commands";
import type {
  AssetType,
  ResultItem,
  SearchUpdate,
  SourceInfo,
  SourceStatus,
} from "../api/types";
import { ASSET_TYPE_LABELS } from "../api/types";
import { useSetting, useSettings } from "../settings/useSettings";
import { ResultTile } from "./tiles/ResultTile";

const TYPE_FILTERS: (AssetType | "all")[] = [
  "all",
  "audio",
  "gif",
  "sticker",
  "image",
  "video",
  "green_screen",
];

export function SearchView() {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<AssetType | "all">("all");
  // null = all sources; otherwise show only this source's results
  const [sourceFilter, setSourceFilter] = useState<string | null>(null);
  const [items, setItems] = useState<ResultItem[]>([]);
  const [statuses, setStatuses] = useState<SourceStatus[]>([]);
  const [sources, setSources] = useState<SourceInfo[]>([]);
  const [done, setDone] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const searchIdRef = useRef(0);
  const { setValue } = useSettings();
  const searchAsYouType = useSetting("search.search_as_you_type", true);
  const debounceMs = useSetting("search.debounce_ms", 450);
  const volume = useSetting<number>("preview.audio_volume", 0.8);
  const tileMinWidth = useSetting("grid.tile_min_width", 220);
  const [columnCount, setColumnCount] = useState(4);
  const gridRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    sourcesList().then(setSources).catch(console.error);
  }, []);

  // responsive column count from tile size setting
  useEffect(() => {
    const el = gridRef.current;
    if (!el) return;
    const update = () =>
      setColumnCount(Math.max(1, Math.floor(el.clientWidth / tileMinWidth)));
    update();
    const obs = new ResizeObserver(update);
    obs.observe(el);
    return () => obs.disconnect();
  }, [tileMinWidth]);

  // progressive results from the host's merged emissions
  useEffect(() => {
    const un = listen<SearchUpdate>("search://update", (ev) => {
      if (ev.payload.search_id !== searchIdRef.current) return;
      setItems(ev.payload.items);
      setStatuses(ev.payload.sources);
      setDone(ev.payload.done);
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const assetTypes = useMemo<AssetType[]>(
    () => (filter === "all" ? [] : [filter]),
    [filter],
  );

  const runSearch = useCallback(
    async (q: string, types: AssetType[]) => {
      if (!q.trim()) {
        searchIdRef.current = 0;
        setItems([]);
        setStatuses([]);
        setDone(true);
        return;
      }
      setDone(false);
      const id = await searchStart(q, types);
      searchIdRef.current = id;
    },
    [],
  );

  // search-as-you-type (a setting), debounced (also a setting)
  useEffect(() => {
    if (!searchAsYouType) return;
    const t = window.setTimeout(() => void runSearch(query, assetTypes), debounceMs);
    return () => window.clearTimeout(t);
  }, [query, assetTypes, searchAsYouType, debounceMs, runSearch]);

  const loadMore = useCallback(async () => {
    setLoadingMore(true);
    try {
      const withMore = statuses.filter((s) => s.next_cursor && s.state === "done");
      const pages = await Promise.allSettled(
        withMore.map((s) => searchMore(s.id, query, assetTypes, s.next_cursor)),
      );
      const extra: ResultItem[] = [];
      const updated = [...statuses];
      pages.forEach((p, i) => {
        const src = withMore[i];
        const slot = updated.find((s) => s.id === src.id);
        if (p.status === "fulfilled") {
          extra.push(...p.value.items);
          if (slot) slot.next_cursor = p.value.next_cursor;
        } else if (slot) {
          slot.next_cursor = null;
        }
      });
      setItems((prev) => {
        const seen = new Set(prev.map((i) => i.id));
        return [...prev, ...extra.filter((i) => !seen.has(i.id))];
      });
      setStatuses(updated);
    } finally {
      setLoadingMore(false);
    }
  }, [statuses, query, assetTypes]);

  const hasMore = statuses.some((s) => s.next_cursor && s.state === "done");
  const keyless = sources.filter((s) => s.requires_key && !s.has_key && s.enabled);
  // client-side source filter on top of the host's merged results
  const shownItems = useMemo(
    () => (sourceFilter ? items.filter((i) => i.source === sourceFilter) : items),
    [items, sourceFilter],
  );

  return (
    <div className="search-view">
      <div className="search-bar-row">
        <input
          className="search-input"
          placeholder="Search every source at once… (vine boom, deal with it, confetti green screen)"
          value={query}
          autoFocus
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch(query, assetTypes);
          }}
        />
        <label className="volume-control" title="Preview volume">
          <span className="volume-icon">{volume === 0 ? "🔇" : "🔊"}</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={volume}
            onChange={(e) => void setValue("preview.audio_volume", Number(e.target.value))}
          />
        </label>
      </div>

      <div className="filter-row">
        {TYPE_FILTERS.map((t) => (
          <button
            key={t}
            className={`chip ${filter === t ? "chip-active" : ""}`}
            onClick={() => setFilter(t)}
          >
            {t === "all" ? "Everything" : ASSET_TYPE_LABELS[t]}
          </button>
        ))}
        {sourceFilter && (
          <button className="chip chip-active" onClick={() => setSourceFilter(null)}>
            only {sourceFilter} ✕
          </button>
        )}
        <span className="spacer" />
        {statuses.map((s) => {
          const active = sourceFilter === s.id;
          const clickable = s.state === "done" && s.count > 0;
          return (
            <button
              key={s.id}
              className={`source-status status-${s.state} ${active ? "source-active" : ""}`}
              title={
                s.state === "needs_key"
                  ? "add a key in Settings"
                  : clickable
                    ? `show only ${s.id}`
                    : s.error ?? ""
              }
              disabled={!clickable}
              onClick={() => setSourceFilter(active ? null : s.id)}
            >
              {s.id}
              {s.state === "pending" && "…"}
              {s.state === "done" && ` ${s.count}`}
              {s.state === "needs_key" && " 🔑"}
              {(s.state === "error" || s.state === "timeout") && " ⚠"}
            </button>
          );
        })}
      </div>

      {keyless.length > 0 && (
        <div className="hint-row">
          {keyless.map((s) => s.name).join(" and ")} need a free API key — add yours in
          Settings to include {keyless.length > 1 ? "them" : "it"}.
        </div>
      )}

      <div className="grid-wrap" ref={gridRef}>
        {items.length === 0 && done && query.trim() === "" ? (
          <div className="empty-state">
            <h2>Find the sound. Find the clip. Keep editing.</h2>
            <p>
              One search across every source at once. Hover to preview, collect
              what you'll reuse.
            </p>
          </div>
        ) : (
          <VirtuosoMasonry
            key={`${columnCount}-${sourceFilter ?? "all"}`}
            columnCount={columnCount}
            data={shownItems}
            style={{ height: "100%" }}
            ItemContent={({ data }) =>
              data ? <ResultTile item={data as ResultItem} /> : null
            }
          />
        )}
      </div>

      {hasMore && (
        <div className="load-more-row">
          <button className="load-more" onClick={loadMore} disabled={loadingMore}>
            {loadingMore ? "Loading…" : "Load more"}
          </button>
        </div>
      )}
    </div>
  );
}
