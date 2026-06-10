import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  assetSetFavorite,
  assetSetTags,
  collectionDelete,
  collectionSearch,
} from "../api/commands";
import { DRAG_ICON } from "../api/dragIcon";
import { localStreamUrl } from "../api/stream";
import type { CollectedAsset } from "../api/types";
import { playExclusive } from "../preview/audioBus";

const TYPE_FILTERS = ["all", "audio", "gif", "sticker", "image", "video", "green_screen"];

export function CollectionView() {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [assets, setAssets] = useState<CollectedAsset[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setAssets(await collectionSearch(query, filter === "all" ? null : filter));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [query, filter]);

  useEffect(() => {
    const t = window.setTimeout(() => void refresh(), 200);
    return () => window.clearTimeout(t);
  }, [refresh]);

  return (
    <div className="collection-view">
      <div className="search-bar-row">
        <input
          className="search-input"
          placeholder="Search your library (full-text over titles, tags, attribution)…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <div className="filter-row">
        {TYPE_FILTERS.map((t) => (
          <button
            key={t}
            className={`chip ${filter === t ? "chip-active" : ""}`}
            onClick={() => setFilter(t)}
          >
            {t === "all" ? "Everything" : t.replace("_", " ")}
          </button>
        ))}
      </div>
      {error && <div className="hint-row">{error}</div>}
      {assets.length === 0 ? (
        <div className="empty-state">
          <h2>Nothing collected yet</h2>
          <p>Search, preview, and hit Collect. Assets land here, ready to drag into your editor.</p>
        </div>
      ) : (
        <div className="library-grid">
          {assets.map((a) => (
            <LibraryTile key={a.id} asset={a} onChanged={refresh} />
          ))}
        </div>
      )}
    </div>
  );
}

function LibraryTile({
  asset,
  onChanged,
}: {
  asset: CollectedAsset;
  onChanged: () => void;
}) {
  const [tagDraft, setTagDraft] = useState(asset.tags.join(", "));
  const [busy, setBusy] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const isAudio = asset.asset_type === "audio";
  const mediaUrl = localStreamUrl(asset.abs_path);
  const posterUrl = asset.thumb_path ? localStreamUrl(asset.thumb_path) : undefined;

  // native drag-out: drop the real file into editors/file managers
  const onDragStart = (e: React.DragEvent) => {
    e.preventDefault();
    void startDrag({ item: [asset.abs_path], icon: DRAG_ICON });
  };

  const playAudio = () => {
    let el = audioRef.current;
    if (!el) {
      el = document.createElement("audio"); // wzstream URL only
      el.src = mediaUrl;
      audioRef.current = el;
    }
    if (el.paused) playExclusive(el);
    else el.pause();
  };

  const saveTags = async () => {
    setBusy(true);
    try {
      await assetSetTags(
        asset.id,
        tagDraft.split(",").map((t) => t.trim()).filter(Boolean),
      );
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="tile library-tile" draggable onDragStart={onDragStart}>
      <div className="visual-preview" style={{ aspectRatio: isAudio ? "auto" : "16 / 10" }}>
        {isAudio ? (
          <button className="audio-preview" onClick={playAudio}>
            <span className="audio-icon">▶</span>
            <span className="audio-bar" />
          </button>
        ) : asset.mime?.startsWith("video") ? (
          <video src={mediaUrl} poster={posterUrl} muted loop onMouseEnter={(e) => void e.currentTarget.play()} onMouseLeave={(e) => e.currentTarget.pause()} />
        ) : (
          <img src={mediaUrl} alt={asset.title} loading="lazy" />
        )}
      </div>
      <div className="tile-meta">
        <div className="tile-title" title={asset.title}>
          {asset.favorite ? "★ " : ""}
          {asset.title}
        </div>
        <div className="tile-sub">
          <span className={`badge badge-${asset.source}`}>{asset.source}</span>
          <span className="duration">{asset.asset_type.replace("_", " ")}</span>
        </div>
        {asset.attribution && <div className="tile-license">{asset.attribution}</div>}
        <div className="tag-row">
          <input
            className="tag-input"
            placeholder="tags, comma, separated"
            value={tagDraft}
            onChange={(e) => setTagDraft(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void saveTags()}
          />
          <button className="mini" onClick={saveTags} disabled={busy}>
            Save
          </button>
        </div>
        <div className="action-row">
          <button
            className="mini"
            title="Favorite"
            onClick={async () => {
              await assetSetFavorite(asset.id, !asset.favorite);
              onChanged();
            }}
          >
            {asset.favorite ? "★" : "☆"}
          </button>
          <button
            className="mini"
            title="Copy file path"
            onClick={() => void writeText(asset.abs_path)}
          >
            Copy path
          </button>
          <button
            className="mini"
            title="Reveal in file manager"
            onClick={() => void revealItemInDir(asset.abs_path)}
          >
            Reveal
          </button>
          <button
            className="mini danger"
            title="Delete from library"
            onClick={async () => {
              if (confirm(`Delete "${asset.title}" from your library?`)) {
                await collectionDelete(asset.id);
                onChanged();
              }
            }}
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}
